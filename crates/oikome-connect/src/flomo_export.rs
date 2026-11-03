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

//! flomo's HTML export (every plan): `flomo_backup.zip`, its HTML file, or
//! the unpacked folder.
//!
//! Each memo is a `.memo` element with its creation time (`.time`, to the
//! second) and its body (`.content`). The export carries no memo ids, so a
//! memo is keyed by its creation time and, for memos created in the same
//! second, its place among them. Every import lists every memo
//! (`complete`): memos missing from a newer export were deleted. An export
//! without any memo is refused rather than taken to mean "delete them all".

use std::io::Read;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use std::cell::RefCell;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use crate::{
    content_hash, ColumnDef, ColumnKind, ConnectError, Connector, Pull, Row, TableDef, Value,
};

pub const KIND: &str = "flomo-export";
/// The table memos land in.
pub const TABLE: &str = "memos";
/// Largest HTML file read from an export.
const MAX_HTML_BYTES: u64 = 512 << 20;

/// The memos table.
pub fn memos_table() -> TableDef {
    let text = |name: &str| ColumnDef {
        name: name.to_string(),
        kind: ColumnKind::Text,
    };
    TableDef {
        name: TABLE.to_string(),
        columns: vec![
            text("id"),
            text("created_at"),
            text("content"),
            text("content_html"),
            text("tags"),
        ],
        key: vec!["id".to_string()],
    }
}

pub struct FlomoExport {
    path: PathBuf,
}

/// One memo read from an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memo {
    pub created_at: String,
    pub content: String,
    pub content_html: String,
    pub tags: Vec<String>,
}

impl FlomoExport {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The HTML documents of the export, with the bytes they were read from.
    fn documents(&self) -> Result<(Vec<String>, Vec<u8>), ConnectError> {
        let io = |e: std::io::Error| ConnectError::Io(format!("{}: {e}", self.path.display()));
        let path = &self.path;
        let is_html = |p: &Path| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
        };
        if path.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(path)
                .map_err(io)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| is_html(p))
                .collect();
            files.sort();
            let mut docs = Vec::new();
            let mut all = Vec::new();
            for file in files {
                let bytes = std::fs::read(&file).map_err(io)?;
                all.extend_from_slice(&bytes);
                docs.push(String::from_utf8_lossy(&bytes).into_owned());
            }
            return Ok((docs, all));
        }
        let bytes = std::fs::read(path).map_err(io)?;
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        {
            let doc = String::from_utf8_lossy(&bytes).into_owned();
            return Ok((vec![doc], bytes));
        }
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
            .map_err(|e| ConnectError::Format(format!("{}: {e}", path.display())))?;
        let mut names: Vec<String> = archive
            .file_names()
            .filter_map(Result::ok)
            .map(|n| n.into_owned())
            .filter(|n| is_html(Path::new(n)))
            .collect();
        names.sort();
        let mut docs = Vec::with_capacity(names.len());
        for name in names {
            let entry = archive
                .by_name(&name)
                .map_err(|e| ConnectError::Format(format!("{}: {name}: {e}", path.display())))?;
            let mut text = String::new();
            entry
                .take(MAX_HTML_BYTES)
                .read_to_string(&mut text)
                .map_err(|e| ConnectError::Format(format!("{}: {name}: {e}", path.display())))?;
            docs.push(text);
        }
        Ok((docs, bytes))
    }
}

/// Elements that never have content or an end tag.
const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
/// Elements that start a line of plain text.
const BLOCK: [&str; 6] = ["p", "li", "div", "ul", "ol", "blockquote"];

/// What [`Collector`] is reading.
#[derive(Default)]
struct Reading {
    /// Names of the open elements.
    open: Vec<String>,
    /// Depth of the open `.memo`, `.time` and `.content` elements.
    memo: Option<usize>,
    time: Option<usize>,
    content: Option<usize>,
    created_at: String,
    text: String,
    html: String,
    seen_content: bool,
    memos: Vec<Memo>,
    error: Option<String>,
}

/// Collects memos from the tokens of an export: html5ever's tokenizer, no
/// DOM. flomo writes the export, so its elements are closed in order.
#[derive(Default)]
struct Collector(RefCell<Reading>);

fn has_class(tag: &Tag, class: &str) -> bool {
    tag.attrs
        .iter()
        .any(|a| &*a.name.local == "class" && a.value.split_whitespace().any(|c| c == class))
}

fn escape(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

impl Reading {
    fn start(&mut self, tag: &Tag) {
        let name = tag.name.to_string();
        if self.content.is_some() {
            if name == "br" {
                self.text.push('\n');
            } else if BLOCK.contains(&name.as_str()) {
                if !self.text.is_empty() && !self.text.ends_with('\n') {
                    self.text.push('\n');
                }
                if name == "li" {
                    self.text.push_str("- ");
                }
            }
            self.html.push('<');
            self.html.push_str(&name);
            for a in &tag.attrs {
                self.html
                    .push_str(&format!(" {}=\"{}\"", a.name.local, escape(&a.value, true)));
            }
            self.html.push('>');
        }
        if VOID.contains(&name.as_str()) || tag.self_closing {
            return;
        }
        self.open.push(name);
        let depth = self.open.len();
        if self.memo.is_none() && has_class(tag, "memo") {
            self.memo = Some(depth);
            self.created_at.clear();
            self.seen_content = false;
        } else if self.memo.is_some() && self.time.is_none() && has_class(tag, "time") {
            self.time = Some(depth);
        } else if self.memo.is_some() && self.content.is_none() && has_class(tag, "content") {
            self.content = Some(depth);
            self.seen_content = true;
            self.text.clear();
            self.html.clear();
        }
    }

    fn end(&mut self, name: &str) {
        let Some(at) = self.open.iter().rposition(|n| n == name) else {
            return;
        };
        let depth = at + 1;
        if self.content == Some(depth) {
            self.content = None;
        } else if self.content.is_some() {
            if BLOCK.contains(&name) && !self.text.ends_with('\n') {
                self.text.push('\n');
            }
            self.html.push_str(&format!("</{name}>"));
        }
        if self.time == Some(depth) {
            self.time = None;
        }
        if self.memo == Some(depth) {
            self.memo = None;
            self.finish();
        }
        self.open.truncate(at);
    }

    fn finish(&mut self) {
        let created_at = self.created_at.trim().to_string();
        if created_at.is_empty() {
            self.error = Some("a memo without its time (.time)".to_string());
            return;
        }
        if !self.seen_content {
            self.error = Some(format!(
                "the memo of {created_at} has no content (.content)"
            ));
            return;
        }
        let content = self.text.trim().to_string();
        self.memos.push(Memo {
            tags: tags_in(&content),
            content,
            content_html: self.html.trim().to_string(),
            created_at,
        });
    }
}

impl TokenSink for Collector {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut r = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => r.start(&tag),
                TagKind::EndTag => r.end(&tag.name),
            },
            Token::CharacterTokens(text) => {
                if r.time.is_some() {
                    r.created_at.push_str(&text);
                } else if r.content.is_some() {
                    r.text.push_str(&text);
                    let escaped = escape(&text, false);
                    r.html.push_str(&escaped);
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// `#tags` in `text`, in order, without repeats; a tag runs to the next
/// space and may hold `/` for sub-tags.
pub fn tags_in(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        for part in word.split('#').skip(1) {
            let tag = part.trim_end_matches(|c: char| ",.;:!?，。；：！？)）".contains(c));
            if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
                tags.push(tag.to_string());
            }
        }
    }
    tags
}

/// The memos in one HTML document of the export.
pub fn parse_memos(html: &str) -> Result<Vec<Memo>, ConnectError> {
    let tokenizer = Tokenizer::new(Collector::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let reading = tokenizer.sink.0.into_inner();
    match reading.error {
        Some(e) => Err(ConnectError::Format(e)),
        None => Ok(reading.memos),
    }
}

/// The rows of `memos`, in export order: ids are the creation time and the
/// memo's place among those of the same second.
pub fn memo_rows(memos: Vec<Memo>) -> Vec<Row> {
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    memos
        .into_iter()
        .map(|m| {
            let n = seen.entry(m.created_at.clone()).or_insert(0);
            let id = format!("{}#{n}", m.created_at);
            *n += 1;
            let tags = serde_json::to_string(&m.tags).unwrap_or_else(|_| "[]".to_string());
            vec![
                Value::Text(id),
                Value::Text(m.created_at),
                Value::Text(m.content),
                Value::Text(m.content_html),
                Value::Text(tags),
            ]
        })
        .collect()
}

#[async_trait]
impl Connector for FlomoExport {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn tables(&self) -> Vec<TableDef> {
        vec![memos_table()]
    }

    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError> {
        let this = FlomoExport::new(self.path.clone());
        let (docs, bytes) = tokio::task::spawn_blocking(move || this.documents())
            .await
            .map_err(|e| ConnectError::Io(e.to_string()))??;
        let hash = content_hash(&bytes);
        if cursor == Some(hash.as_str()) {
            return Ok(Pull::unchanged(Some(hash)));
        }
        let mut memos = Vec::new();
        for doc in &docs {
            memos.extend(parse_memos(doc)?);
        }
        if memos.is_empty() {
            return Err(ConnectError::Format(format!(
                "{} holds no memos; is it flomo's HTML export?",
                self.path.display()
            )));
        }
        Ok(Pull {
            upserts: vec![(TABLE.to_string(), memo_rows(memos))],
            deletes: Vec::new(),
            cursor: Some(hash),
            complete: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = r#"<html><body><div class="memos">
      <div class="memo"><div class="time">2026-10-01 08:00:00</div>
        <div class="content"><p>First <mark>idea</mark> #reading/books</p><p>second line</p></div></div>
      <div class="memo"><div class="time">2026-10-01 08:00:00</div>
        <div class="content"><ul><li>a</li><li>b #todo</li></ul></div></div>
      <div class="memo"><div class="time">2026-10-02 09:30:15</div>
        <div class="content"><p>#reading again, #todo.</p></div></div>
    </div></body></html>"#;

    #[test]
    fn memos_tags_and_ids_come_from_the_export() {
        let memos = parse_memos(EXPORT).unwrap();
        assert_eq!(memos.len(), 3);
        assert_eq!(memos[0].content, "First idea #reading/books\nsecond line");
        assert!(memos[0].content_html.contains("<mark>idea</mark>"));
        assert_eq!(memos[0].tags, vec!["reading/books"]);
        assert_eq!(memos[1].content, "- a\n- b #todo");
        assert_eq!(memos[2].tags, vec!["reading", "todo"]);
        let rows = memo_rows(memos);
        let ids: Vec<_> = rows.iter().map(|r| r[0].clone()).collect();
        assert_eq!(
            ids,
            vec![
                Value::Text("2026-10-01 08:00:00#0".into()),
                Value::Text("2026-10-01 08:00:00#1".into()),
                Value::Text("2026-10-02 09:30:15#0".into()),
            ]
        );
        assert_eq!(rows[2][4], Value::Text(r#"["reading","todo"]"#.into()));
    }

    #[tokio::test]
    async fn exports_are_read_from_a_zip_and_an_empty_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("flomo_backup.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("flomo@me-20261002/index.html", options)
                .unwrap();
            std::io::Write::write_all(&mut zip, EXPORT.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let export = FlomoExport::new(zip_path);
        let pull = export.pull(None).await.unwrap();
        assert!(pull.complete);
        assert_eq!(pull.upserts[0].1.len(), 3);
        let again = export.pull(pull.cursor.as_deref()).await.unwrap();
        assert!(again.upserts.is_empty());

        let empty = dir.path().join("empty.html");
        std::fs::write(&empty, "<html><body>nothing</body></html>").unwrap();
        assert!(FlomoExport::new(empty).pull(None).await.is_err());
    }
}
