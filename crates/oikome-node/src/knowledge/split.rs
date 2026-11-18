// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Splitting an original into chunks (docs/knowledge-schema.md §8): by
//! heading and paragraph for Markdown, by paragraph for text and code, page
//! by page for a PDF. Every chunk is an exact slice of the text it came from
//! and keeps where it was: its heading path, its characters and its lines.

/// What kind of original a file is, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Markdown,
    Text,
    Code,
    Pdf,
}

impl Kind {
    /// The kind of `name`, or `None` for a file knowledge does not read yet
    /// (tables are K3, media K4).
    pub(crate) fn of(name: &str) -> Option<Kind> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "md" | "markdown" | "mdown" | "mkd" | "mdx" => Kind::Markdown,
            "txt" | "text" | "rst" | "org" | "adoc" | "asciidoc" | "log" => Kind::Text,
            "pdf" => Kind::Pdf,
            "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go" | "java" | "kt"
            | "swift" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "rb" | "php" | "sh" | "bash"
            | "zsh" | "sql" | "css" | "scss" | "html" | "htm" | "xml" | "yaml" | "yml" | "toml"
            | "ini" | "cfg" | "conf" | "json" => Kind::Code,
            _ => return None,
        })
    }

    /// The name shown and stored for how it was split.
    pub(crate) fn converter(self) -> &'static str {
        match self {
            Kind::Markdown => "markdown",
            Kind::Text => "text",
            Kind::Code => "code",
            Kind::Pdf => "pdf",
        }
    }
}

/// A chunk: a slice of the original's text and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub text: String,
    /// Headings above it, outermost first.
    pub heading: Vec<String>,
    /// The PDF page, from 1.
    pub page: Option<u32>,
    /// Characters (Unicode scalar values) of the text it was cut from, end
    /// exclusive; for a PDF, of its page's text.
    pub char_start: u64,
    pub char_end: u64,
    /// Lines, from 1, both inclusive.
    pub line_start: u32,
    pub line_end: u32,
}

/// Aim for chunks about this long, in characters…
const TARGET: usize = 900;
/// …and cut a single paragraph longer than this.
const MAX: usize = 1800;

#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    text: &'a str,
    /// Characters before the line.
    start: usize,
    number: u32,
}

#[derive(Debug)]
enum Block {
    Heading {
        level: usize,
        title: String,
        first: usize,
    },
    Body {
        first: usize,
        last: usize,
    },
}

fn lines_of(text: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        out.push(Line {
            text: line,
            start,
            number: u32::try_from(i + 1).unwrap_or(u32::MAX),
        });
        start += raw.chars().count() + 1;
    }
    out
}

/// An ATX heading (`## Title`): its level and title.
fn heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim_end().to_string();
    Some((level, title))
}

fn fence(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if t.starts_with("```") {
        Some("```")
    } else if t.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// Blocks of lines: headings, and runs of non-blank lines (a fenced code
/// block, blank lines and all, is one run).
fn blocks(lines: &[Line<'_>], markdown: bool) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].text;
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if markdown {
            if let Some((level, title)) = heading(line) {
                out.push(Block::Heading {
                    level,
                    title,
                    first: i,
                });
                i += 1;
                continue;
            }
        }
        let first = i;
        let mut open: Option<&str> = None;
        while i < lines.len() {
            let text = lines[i].text;
            if markdown {
                if let Some(mark) = fence(text) {
                    open = match open {
                        Some(m) if m == mark => None,
                        Some(m) => Some(m),
                        None => Some(mark),
                    };
                    i += 1;
                    continue;
                }
            }
            if open.is_none() {
                if text.trim().is_empty() {
                    break;
                }
                if markdown && i > first && heading(text).is_some() {
                    break;
                }
            }
            i += 1;
        }
        out.push(Block::Body { first, last: i - 1 });
    }
    out
}

fn chars_of(lines: &[Line<'_>], first: usize, last: usize) -> usize {
    let end = lines[last].start + lines[last].text.chars().count();
    end - lines[first].start
}

fn titles(path: &[(usize, String)]) -> Vec<String> {
    path.iter().map(|(_, title)| title.clone()).collect()
}

/// Split `text` into chunks; `page` is the PDF page it is.
pub(crate) fn split(text: &str, kind: Kind, page: Option<u32>) -> Vec<Piece> {
    // A byte-order mark is a character of the file, not of its text.
    let (text, base) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (rest, 1),
        None => (text, 0),
    };
    let chars: Vec<char> = text.chars().collect();
    let lines = lines_of(text);
    let blocks = blocks(&lines, kind == Kind::Markdown);
    let mut out = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    // The lines of the chunk being built, and its heading path.
    let mut current: Option<(usize, usize, Vec<String>)> = None;

    let flush = |current: &mut Option<(usize, usize, Vec<String>)>, out: &mut Vec<Piece>| {
        if let Some((first, last, heading)) = current.take() {
            // A heading with nothing under it is no passage; it lives on
            // in the heading path of the chunks after it.
            if first == last && is_heading(&lines, first, kind) {
                return;
            }
            push_range(&chars, &lines, first, last, heading, page, out);
        }
    };

    for block in &blocks {
        match block {
            Block::Heading {
                level,
                title,
                first,
            } => {
                flush(&mut current, &mut out);
                path.retain(|(l, _)| l < level);
                path.push((*level, title.clone()));
                // The heading line opens the next chunk.
                current = Some((*first, *first, titles(&path)));
            }
            Block::Body { first, last } => {
                if chars_of(&lines, *first, *last) > MAX {
                    flush(&mut current, &mut out);
                    split_long(&chars, &lines, *first, *last, titles(&path), page, &mut out);
                    continue;
                }
                match &mut current {
                    Some((start, end, _)) if chars_of(&lines, *start, *last) <= TARGET => {
                        *end = *last;
                    }
                    Some((start, end, _)) if *start == *end && is_heading(&lines, *start, kind) => {
                        // A heading stays with its first paragraph.
                        *end = *last;
                    }
                    _ => {
                        flush(&mut current, &mut out);
                        current = Some((*first, *last, titles(&path)));
                    }
                }
            }
        }
    }
    flush(&mut current, &mut out);
    for piece in &mut out {
        piece.char_start += base;
        piece.char_end += base;
    }
    out
}

fn is_heading(lines: &[Line<'_>], i: usize, kind: Kind) -> bool {
    kind == Kind::Markdown && heading(lines[i].text).is_some()
}

#[allow(clippy::too_many_arguments)]
fn push_range(
    chars: &[char],
    lines: &[Line<'_>],
    first: usize,
    last: usize,
    heading: Vec<String>,
    page: Option<u32>,
    out: &mut Vec<Piece>,
) {
    let start = lines[first].start;
    let end = lines[last].start + lines[last].text.chars().count();
    push_chars(chars, lines, start, end, heading, page, out);
}

fn push_chars(
    chars: &[char],
    lines: &[Line<'_>],
    start: usize,
    end: usize,
    heading: Vec<String>,
    page: Option<u32>,
    out: &mut Vec<Piece>,
) {
    let text: String = chars[start..end].iter().collect();
    if text.trim().is_empty() {
        return;
    }
    let line_at = |at: usize| {
        lines
            .iter()
            .rev()
            .find(|l| l.start <= at)
            .map(|l| l.number)
            .unwrap_or(1)
    };
    out.push(Piece {
        text,
        heading,
        page,
        char_start: start as u64,
        char_end: end as u64,
        line_start: line_at(start),
        line_end: line_at(end.saturating_sub(1).max(start)),
    });
}

/// A paragraph longer than [`MAX`]: cut by lines when it has several, then
/// at sentence ends, then wherever [`TARGET`] falls.
fn split_long(
    chars: &[char],
    lines: &[Line<'_>],
    first: usize,
    last: usize,
    heading: Vec<String>,
    page: Option<u32>,
    out: &mut Vec<Piece>,
) {
    let end = lines[last].start + lines[last].text.chars().count();
    let mut start = lines[first].start;
    while start < end {
        if end - start <= TARGET {
            push_chars(chars, lines, start, end, heading.clone(), page, out);
            break;
        }
        let limit = start + TARGET;
        // The last line break, else sentence end, before the limit.
        let cut = (start + TARGET / 3..limit)
            .rev()
            .find(|&i| chars[i] == '\n')
            .or_else(|| {
                (start + TARGET / 3..limit)
                    .rev()
                    .find(|&i| {
                        matches!(chars[i], '。' | '！' | '？' | '.' | '!' | '?' | '；' | ';')
                    })
                    .map(|i| i + 1)
            })
            .unwrap_or(limit);
        push_chars(chars, lines, start, cut, heading.clone(), page, out);
        start = cut;
        while start < end && chars[start].is_whitespace() {
            start += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, p: &Piece) -> String {
        text.chars()
            .skip(p.char_start as usize)
            .take((p.char_end - p.char_start) as usize)
            .collect()
    }

    #[test]
    fn kinds_follow_the_extension() {
        assert_eq!(Kind::of("Notes/Lisbon.MD"), Some(Kind::Markdown));
        assert_eq!(Kind::of("a.pdf"), Some(Kind::Pdf));
        assert_eq!(Kind::of("main.rs"), Some(Kind::Code));
        assert_eq!(Kind::of("readings.csv"), None);
        assert_eq!(Kind::of("photo.jpg"), None);
        assert_eq!(Kind::of("README"), None);
    }

    #[test]
    fn markdown_keeps_headings_and_exact_places() {
        let text = "# Oikome\n\nIntro line.\n\n## 联邦层\n\nDataFusion 是协调者。\n\n### Providers\n\nEngine executors.\n\n## Storage\n\nParquet files.\n";
        let pieces = split(text, Kind::Markdown, None);
        let headings: Vec<_> = pieces.iter().map(|p| p.heading.join(" > ")).collect();
        assert_eq!(
            headings,
            vec![
                "Oikome",
                "Oikome > 联邦层",
                "Oikome > 联邦层 > Providers",
                "Oikome > Storage"
            ]
        );
        for p in &pieces {
            assert_eq!(slice(text, p), p.text, "{p:?}");
        }
        assert!(pieces[1].text.starts_with("## 联邦层"));
        assert!(pieces[1].text.contains("协调者"));
        assert_eq!((pieces[1].line_start, pieces[1].line_end), (5, 7));

        // A heading with nothing under it makes no chunk of its own.
        let bare = split("# 架构\n\n## 联邦层\n\n协调者。\n", Kind::Markdown, None);
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].heading, vec!["架构", "联邦层"]);
    }

    #[test]
    fn short_paragraphs_share_a_chunk_and_fences_stay_whole() {
        let text = "One.\n\nTwo.\n\n```\nlet a = 1;\n\n# not a heading\n```\n\nThree.\n";
        let pieces = split(text, Kind::Markdown, None);
        assert_eq!(pieces.len(), 1);
        assert!(pieces[0].heading.is_empty());
        assert!(pieces[0].text.contains("# not a heading"));
        assert_eq!(pieces[0].line_end, 11);
    }

    #[test]
    fn long_paragraphs_are_cut_at_sentences() {
        let sentence = "这是一个关于数据中心的句子。";
        let text = sentence.repeat(200);
        let pieces = split(&text, Kind::Text, Some(3));
        assert!(pieces.len() > 2);
        for p in &pieces {
            assert!(p.text.chars().count() <= TARGET);
            assert_eq!(p.page, Some(3));
            assert!(p.text.ends_with('。'), "{}", p.text);
            assert_eq!(slice(&text, p), p.text);
        }
        assert_eq!(
            pieces.last().unwrap().char_end as usize,
            text.chars().count()
        );
    }

    #[test]
    fn a_byte_order_mark_counts_as_a_character() {
        let text = "\u{feff}Hello there.";
        let pieces = split(text, Kind::Text, None);
        assert_eq!(pieces[0].text, "Hello there.");
        assert_eq!((pieces[0].char_start, pieces[0].char_end), (1, 13));
    }

    #[test]
    fn blank_text_has_no_chunks() {
        assert!(split("\n\n  \n", Kind::Text, None).is_empty());
        assert!(split("\u{feff}", Kind::Markdown, None).is_empty());
    }
}
