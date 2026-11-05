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

//! Searching the owner's collections (M3a): `search` looks through each of
//! them (one Space at a time, as the node searches) and ranks what it found,
//! with an excerpt of each row and where the words are in it; `read_row`
//! opens one row of a hit by its key.

use oikome_catalog::Space;
use oikome_core::SpaceKind;
use oikome_node::{OikomeSession, QueryResult, SearchMatch};

use crate::{
    encode_rows, Api, ApiError, Caller, ReadRow, Rows, Search, SearchHit, SearchMark,
    SearchResults, SearchSkip, SearchSkipped,
};

/// Longest query, in characters.
const MAX_QUERY_CHARS: usize = 200;
/// Words of a query used; the rest are left out.
const MAX_TERMS: usize = 8;
const DEFAULT_HITS: u32 = 30;
const MAX_HITS: u32 = 100;
/// How many hits a search asked for returns.
pub(crate) fn hits_wanted(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_HITS).clamp(1, MAX_HITS) as usize
}

/// Characters of an excerpt before the first match, and in all.
const EXCERPT_BEFORE: usize = 40;
const EXCERPT_CHARS: usize = 160;
/// Most key columns `read_row` takes.
pub(crate) const MAX_KEY_COLUMNS: usize = 8;

/// The words of a query, each once (case aside), without control characters.
pub(crate) fn terms_of(query: &str) -> Result<Vec<String>, ApiError> {
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::invalid(format!(
            "search for at most {MAX_QUERY_CHARS} characters"
        )));
    }
    let clean: String = query
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut terms: Vec<String> = Vec::new();
    for word in clean.split_whitespace() {
        if !terms
            .iter()
            .any(|t| t.to_lowercase() == word.to_lowercase())
        {
            terms.push(word.to_string());
        }
    }
    terms.truncate(MAX_TERMS);
    Ok(terms)
}

/// A character as a search compares it: lower case, common accents off.
fn fold(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        _ => c,
    }
}

/// Where each of `terms` is in `text`, as character ranges, merged.
fn find_all(text: &[char], terms: &[String]) -> Vec<(usize, usize)> {
    let folded: Vec<char> = text.iter().map(|&c| fold(c)).collect();
    let mut ranges = Vec::new();
    for term in terms {
        let needle: Vec<char> = term.chars().map(fold).collect();
        if needle.is_empty() || needle.len() > folded.len() {
            continue;
        }
        let mut at = 0;
        while at + needle.len() <= folded.len() {
            if folded[at..at + needle.len()] == needle[..] {
                ranges.push((at, at + needle.len()));
                at += needle.len();
            } else {
                at += 1;
            }
        }
    }
    ranges.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The excerpt of a matched row: the first of its text columns with a word
/// in it, other than its key (a note's text over its file name, a photo's
/// title over its file), around the first match, on one line, with the
/// words marked.
fn hit_of(collection: String, found: SearchMatch, terms: &[String]) -> SearchHit {
    let keys: Vec<String> = found.key.clone().unwrap_or_default();
    let lines: Vec<(String, Vec<char>)> = found
        .texts
        .into_iter()
        .map(|(column, text)| {
            let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
            (column, line.chars().collect())
        })
        .collect();
    let is_key = |chars: &Vec<char>| keys.iter().any(|k| k.chars().eq(chars.iter().copied()));
    let matched = |chars: &Vec<char>| !find_all(chars, terms).is_empty();
    let chosen = lines
        .iter()
        .find(|(_, chars)| matched(chars) && !is_key(chars))
        .or_else(|| lines.iter().find(|(_, chars)| matched(chars)))
        .or_else(|| lines.iter().find(|(_, chars)| !chars.is_empty()));
    let (column, excerpt, marks) = match chosen {
        Some((column, chars)) => {
            let first = find_all(chars, terms).first().map_or(0, |r| r.0);
            let start = first.saturating_sub(EXCERPT_BEFORE);
            let end = (start + EXCERPT_CHARS).min(chars.len());
            let mut cut: Vec<char> = Vec::new();
            if start > 0 {
                cut.push('…');
            }
            cut.extend_from_slice(&chars[start..end]);
            if end < chars.len() {
                cut.push('…');
            }
            // Offsets in UTF-16 units, as the page's JavaScript counts them.
            let mut utf16 = Vec::with_capacity(cut.len() + 1);
            let mut n = 0u32;
            for c in &cut {
                utf16.push(n);
                n += c.len_utf16() as u32;
            }
            utf16.push(n);
            let marks = find_all(&cut, terms)
                .into_iter()
                .map(|(s, e)| SearchMark {
                    start: utf16[s],
                    end: utf16[e],
                })
                .collect();
            (column.clone(), cut.into_iter().collect(), marks)
        }
        None => (String::new(), String::new(), Vec::new()),
    };
    SearchHit {
        collection,
        table: found.table,
        key: found.key,
        column,
        excerpt,
        marks,
    }
}

impl Api {
    /// Search the owner's collections, or one of them, for rows with every
    /// word of the query in their text, best first. Collections shared with
    /// the owner are listed as skipped, as is one whose search failed.
    pub async fn search(&self, caller: &Caller, req: Search) -> Result<SearchResults, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let terms = terms_of(&req.query)?;
        let spaces = match &req.collection {
            Some(name) => {
                crate::validate_space_name(name)?;
                vec![self.catalog.get_space(name)?]
            }
            None => self.catalog.list_spaces(),
        };
        Ok(self
            .search_in(&self.session(None), spaces, &terms, hits_wanted(req.limit))
            .await)
    }

    /// Search `spaces` in `session` (the owner's, or an assistant's) one
    /// at a time and rank what was found.
    pub(crate) async fn search_in(
        &self,
        session: &OikomeSession,
        spaces: Vec<Space>,
        terms: &[String],
        limit: usize,
    ) -> SearchResults {
        let mut results = SearchResults {
            hits: Vec::new(),
            skipped: Vec::new(),
        };
        if terms.is_empty() {
            return results;
        }
        let mut found: Vec<(String, SearchMatch)> = Vec::new();
        for space in spaces {
            if space.kind == SpaceKind::Remote {
                results.skipped.push(SearchSkipped {
                    collection: space.name,
                    reason: SearchSkip::SharedWithYou,
                    message: None,
                });
                continue;
            }
            match session.search(&space.name, terms, limit).await {
                Ok(matches) => found.extend(matches.into_iter().map(|m| (space.name.clone(), m))),
                Err(e) => {
                    tracing::warn!(collection = %space.name, error = %e, "searching a collection failed");
                    results.skipped.push(SearchSkipped {
                        collection: space.name,
                        reason: SearchSkip::Failed,
                        message: Some(e.to_string()),
                    });
                }
            }
        }
        // Ranked hits (BM25, lower is better) first, then the rest in the
        // order their collections gave them.
        found.sort_by(|(_, a), (_, b)| match (a.score, b.score) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        found.truncate(limit);
        results.hits = found
            .into_iter()
            .map(|(collection, m)| hit_of(collection, m, terms))
            .collect();
        results
    }

    /// One row of a collection a source fills, by its primary key: what a
    /// search hit opens.
    pub async fn read_row(&self, caller: &Caller, req: ReadRow) -> Result<Rows, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        crate::validate_space_name(&req.space)?;
        if req.key.is_empty() || req.key.len() > MAX_KEY_COLUMNS {
            return Err(ApiError::invalid(format!(
                "a key has 1 to {MAX_KEY_COLUMNS} columns"
            )));
        }
        let session = self.session(None);
        match session.row_by_key(&req.space, &req.table, &req.key).await? {
            QueryResult::Batches { schema, batches } => {
                if batches.iter().all(|b| b.num_rows() == 0) {
                    return Err(ApiError::not_found(format!(
                        "No such row in {}.{}",
                        req.space, req.table
                    )));
                }
                encode_rows(&schema, &batches, 1, usize::MAX)
            }
            _ => Err(ApiError::internal("reading a row returned no rows")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_its_words_once() {
        assert_eq!(
            terms_of("  Pessoa\ttrain pessoa\u{0007}café ").unwrap(),
            vec!["Pessoa", "train", "café"]
        );
        assert!(terms_of(&"x".repeat(201)).is_err());
        assert_eq!(terms_of("a b c d e f g h i j").unwrap().len(), MAX_TERMS);
    }

    #[test]
    fn excerpts_mark_words_where_javascript_counts_them() {
        let found = SearchMatch {
            table: "notes".into(),
            key: Some(vec!["a.md".into()]),
            texts: vec![
                ("file".into(), "a.md".into()),
                ("note".into(), "🙂 Coffee at the\n Café Brasileira".into()),
            ],
            score: None,
        };
        let hit = hit_of("notes".into(), found, &["cafe".into()]);
        assert_eq!(hit.column, "note");
        assert_eq!(hit.excerpt, "🙂 Coffee at the Café Brasileira");
        // The emoji is two UTF-16 units; "Café" starts after "🙂 Coffee at the ".
        assert_eq!(hit.marks, vec![SearchMark { start: 17, end: 21 }]);
    }

    #[test]
    fn a_photo_is_shown_by_its_title_not_its_file_or_credit() {
        let found = SearchMatch {
            table: "photos".into(),
            key: Some(vec!["Pastéis de Belém.jpg".into()]),
            texts: vec![
                ("file".into(), "Pastéis de Belém.jpg".into()),
                ("title".into(), "Pastéis de Belém".into()),
                (
                    "credit".into(),
                    "\"Pasteis de Belem\" by someone, CC BY 2.0".into(),
                ),
            ],
            score: Some(-1.0),
        };
        let hit = hit_of("lisbon-photos".into(), found, &["belem".into()]);
        assert_eq!(hit.column, "title");
        assert_eq!(hit.excerpt, "Pastéis de Belém");
    }

    #[test]
    fn a_note_is_shown_by_its_text_not_its_file_name() {
        let found = SearchMatch {
            table: "notes".into(),
            key: Some(vec!["1 Welcome.md".into()]),
            texts: vec![
                ("file".into(), "1 Welcome.md".into()),
                (
                    "note".into(),
                    "Welcome to Oikome. Your data stays here.".into(),
                ),
            ],
            score: Some(-1.0),
        };
        let hit = hit_of("guide".into(), found, &["welcome".into()]);
        assert_eq!(hit.column, "note");
        assert!(hit.excerpt.starts_with("Welcome to Oikome."));
    }

    #[test]
    fn long_texts_are_cut_around_the_first_match() {
        let text = format!("{} needle {}", "a".repeat(300), "b".repeat(300));
        let found = SearchMatch {
            table: "notes".into(),
            key: None,
            texts: vec![("note".into(), text)],
            score: Some(-1.0),
        };
        let hit = hit_of("notes".into(), found, &["needle".into()]);
        assert!(hit.excerpt.starts_with('…') && hit.excerpt.ends_with('…'));
        assert_eq!(hit.excerpt.chars().count(), EXCERPT_CHARS + 2);
        let mark = hit.marks[0];
        let marked: String = hit
            .excerpt
            .encode_utf16()
            .skip(mark.start as usize)
            .take((mark.end - mark.start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap())
            .collect();
        assert_eq!(marked, "needle");
    }
}
