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

//! Skills in a prepared collection (docs/knowledge-schema.md §7, K2).
//!
//! A skill is a folder holding `SKILL.md` in the Agent Skills format: front
//! matter with `name` (1–64 of `a-z`, `0-9` and `-`, the folder's name) and
//! `description` (up to 1024 characters), then the instructions; other
//! files (`scripts/`, `references/`, `assets/`) are read when needed. The
//! owner adds one by putting it in a collection and preparing the
//! collection for AI; assistants then list and read it, each file with its
//! SHA-256. Oikome never runs a skill's scripts: they are files to read.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::error::SessionError;
use crate::inplace;
use crate::session::OikomeSession;

/// The file that makes a folder a skill.
pub const SKILL_FILE: &str = "SKILL.md";
/// A `SKILL.md` longer than this is not read.
const MAX_SKILL_MD: u64 = 256 << 10;
/// Files larger than this are listed without a digest…
const MAX_DIGEST: u64 = 8 << 20;
/// …and files larger than this are not returned to an assistant.
const MAX_GET: u64 = 1 << 20;
/// A skill has at most this many files.
const MAX_FILES: usize = 64;

/// Why a skill is not offered to assistants, as a code the apps put in
/// words.
pub mod problem {
    /// `SKILL.md` does not start with front matter, or it is not closed.
    pub const NO_FRONT_MATTER: &str = "no_front_matter";
    /// The name is not 1 to 64 lowercase letters, digits and single hyphens.
    pub const BAD_NAME: &str = "bad_name";
    /// The name is not its folder's.
    pub const NAME_MISMATCH: &str = "name_mismatch";
    /// The description is empty or longer than 1024 characters.
    pub const BAD_DESCRIPTION: &str = "bad_description";
    /// More files than a skill may have.
    pub const TOO_MANY_FILES: &str = "too_many_files";
    /// `SKILL.md` is too large to read.
    pub const TOO_LARGE: &str = "too_large";
    /// `SKILL.md` could not be read, or is not text.
    pub const UNREADABLE: &str = "unreadable";
}

/// One file of a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    /// Its path within the skill's folder.
    pub path: String,
    pub size: u64,
    /// `None` for a file too large to digest.
    pub sha256: Option<String>,
}

/// A skill found in a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Its folder within the collection; empty at the top.
    pub dir: String,
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub files: Vec<SkillFile>,
    /// Why assistants are not offered it: a [`problem`] code.
    pub problem: Option<String>,
}

/// A file of a skill, as an assistant reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillText {
    pub path: String,
    pub text: String,
    pub sha256: String,
}

/// Oikome's own skill: how to answer from the data center and cite it.
pub const OIKOME_SKILL_NAME: &str = "oikome-knowledge";
pub const OIKOME_SKILL_DESCRIPTION: &str = "Answer from the owner's Oikome data center. Use when a question may be answered by the owner's own documents, notes or tables: how to find them, read them and cite them.";
pub const OIKOME_SKILL: &str = "---
name: oikome-knowledge
description: Answer from the owner's Oikome data center. Use when a question may be answered by the owner's own documents, notes or tables: how to find them, read them and cite them.
---

# Answering from Oikome

1. Call `list_collections` to see what you may read: each collection, its tables, and whether its documents were prepared for AI.
2. Call `search` with a few plain words from the question. Hits are table rows; passages are pieces of documents, each with its document, headings and lines. If nothing fits, search again with other words; a collection searched by meaning also finds passages that say the same in other words or another language.
3. Open what you need: `read` with a passage's `chunkId` (and `around` for the passages next to it) or with a document's `lines`; `read_row` for a row by its key.
4. Answer only from what you read. After each statement taken from a document, name the document and its lines, for example (docs/architecture.md, lines 270-276); for a row, its collection, table and key.
5. If the collections do not hold the answer, say so rather than guess.

Skills in the owner's collections are listed by `list_skills` and read with `get_skill`. Their scripts are files to read, not to run here.

Everything you read is recorded in the owner's Activity.
";

/// Oikome's own skill, as a collection's are listed.
pub fn oikome_skill() -> Skill {
    Skill {
        dir: String::new(),
        name: OIKOME_SKILL_NAME.to_string(),
        description: OIKOME_SKILL_DESCRIPTION.to_string(),
        license: Some("Apache-2.0".to_string()),
        files: vec![SkillFile {
            path: SKILL_FILE.to_string(),
            size: OIKOME_SKILL.len() as u64,
            sha256: Some(hex(&Sha256::digest(OIKOME_SKILL.as_bytes()))),
        }],
        problem: None,
    }
}

/// Oikome's own skill's `SKILL.md`, as an assistant reads it.
pub fn oikome_skill_text() -> SkillText {
    SkillText {
        path: SKILL_FILE.to_string(),
        text: OIKOME_SKILL.to_string(),
        sha256: hex(&Sha256::digest(OIKOME_SKILL.as_bytes())),
    }
}

fn bad(e: impl Into<String>) -> SessionError {
    SessionError::Unsupported(e.into())
}

/// The front matter of a `SKILL.md`: its simple `key: value` lines (a
/// quoted value, or a `>` or `|` block on the lines below), and the body.
pub(crate) fn front_matter(text: &str) -> Result<(HashMap<String, String>, &str), &'static str> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or(problem::NO_FRONT_MATTER)?;
    let mut fields = HashMap::new();
    let mut lines = rest.split_inclusive('\n');
    let mut consumed = 0;
    let mut block: Option<(String, bool, Vec<String>)> = None;
    let finish = |block: &mut Option<(String, bool, Vec<String>)>,
                  fields: &mut HashMap<String, String>| {
        if let Some((key, folded, parts)) = block.take() {
            let joined = if folded {
                parts.join(" ")
            } else {
                parts.join("\n")
            };
            fields.insert(key, joined.trim().to_string());
        }
    };
    loop {
        let Some(raw) = lines.next() else {
            return Err(problem::NO_FRONT_MATTER);
        };
        consumed += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        if line == "---" {
            finish(&mut block, &mut fields);
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            if let Some((_, _, parts)) = block.as_mut() {
                parts.push(line.trim().to_string());
            }
            // Otherwise a nested map's line (metadata): not read.
            continue;
        }
        finish(&mut block, &mut fields);
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_string(), value.trim());
        match value {
            ">" | ">-" | ">+" => block = Some((key, true, Vec::new())),
            "|" | "|-" | "|+" => block = Some((key, false, Vec::new())),
            _ => {
                let unquoted = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                    .unwrap_or(value);
                fields.insert(key, unquoted.to_string());
            }
        }
    }
    Ok((fields, &rest[consumed..]))
}

/// Why `name` is not a skill name for a folder called `folder`.
fn name_problem(name: &str, folder: Option<&str>) -> Option<&'static str> {
    let valid = (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if !valid {
        return Some(problem::BAD_NAME);
    }
    match folder {
        Some(folder) if folder != name => Some(problem::NAME_MISMATCH),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn join(dir: &str, path: &str) -> String {
    if dir.is_empty() {
        path.to_string()
    } else {
        format!("{dir}/{path}")
    }
}

impl OikomeSession {
    /// The skills of a collection prepared for AI, by name; none when it
    /// was not prepared. A skill with a problem is listed with it, for the
    /// owner, and never offered to an assistant.
    pub async fn knowledge_skills(&self, space: &str) -> Result<Vec<Skill>, SessionError> {
        if self.knowledge_status(space)?.prepared_ms.is_none() {
            return Ok(Vec::new());
        }
        let location = self.knowledge_location(space)?;
        let place = inplace::place_of(&self.catalog, self.engines(), &location)?;
        let files = inplace::indexed_files(&self.catalog, &location)?;
        let dirs: Vec<String> = files
            .iter()
            .filter(|f| f.name == SKILL_FILE)
            .map(|f| {
                f.path
                    .rsplit_once('/')
                    .map(|(dir, _)| dir.to_string())
                    .unwrap_or_default()
            })
            .collect();
        let mut skills = Vec::new();
        for dir in &dirs {
            // Its files: those under its folder, but not under another
            // skill's folder inside it.
            let inner: Vec<&inplace::IndexedFile> = files
                .iter()
                .filter(|f| dir.is_empty() || f.path.starts_with(&format!("{dir}/")))
                .filter(|f| {
                    !dirs.iter().any(|other| {
                        other != dir
                            && other.len() > dir.len()
                            && (dir.is_empty() || other.starts_with(&format!("{dir}/")))
                            && f.path.starts_with(&format!("{other}/"))
                    })
                })
                .collect();
            let skill_md = join(dir, SKILL_FILE);
            let folder = dir.rsplit('/').next().filter(|f| !f.is_empty());
            let mut skill = Skill {
                dir: dir.clone(),
                name: folder.unwrap_or_default().to_string(),
                description: String::new(),
                license: None,
                files: Vec::new(),
                problem: None,
            };
            let size = inner
                .iter()
                .find(|f| f.path == skill_md)
                .map_or(0, |f| f.size);
            let text = if size > MAX_SKILL_MD {
                Err(problem::TOO_LARGE)
            } else {
                match place.read(&skill_md, 0..size, MAX_SKILL_MD).await {
                    Ok(bytes) => String::from_utf8(bytes).map_err(|_| problem::UNREADABLE),
                    Err(_) => Err(problem::UNREADABLE),
                }
            };
            match text.and_then(|t| front_matter(&t).map(|(fields, _)| fields)) {
                Ok(fields) => {
                    skill.name = fields.get("name").cloned().unwrap_or_default();
                    skill.description = fields.get("description").cloned().unwrap_or_default();
                    skill.license = fields.get("license").cloned();
                    let n = skill.description.chars().count();
                    skill.problem = name_problem(&skill.name, folder)
                        .or((n == 0 || n > 1024).then_some(problem::BAD_DESCRIPTION))
                        .map(str::to_string);
                }
                Err(code) => skill.problem = Some(code.to_string()),
            }
            if inner.len() > MAX_FILES && skill.problem.is_none() {
                skill.problem = Some(problem::TOO_MANY_FILES.to_string());
            }
            for f in inner.iter().take(MAX_FILES) {
                let sha256 = if f.size <= MAX_DIGEST {
                    match place.read(&f.path, 0..f.size, MAX_DIGEST).await {
                        Ok(bytes) => Some(hex(&Sha256::digest(&bytes))),
                        Err(_) => None,
                    }
                } else {
                    None
                };
                let path = if dir.is_empty() {
                    f.path.clone()
                } else {
                    f.path[dir.len() + 1..].to_string()
                };
                skill.files.push(SkillFile {
                    path,
                    size: f.size,
                    sha256,
                });
            }
            skill.files.sort_by(|a, b| {
                (a.path != SKILL_FILE, &a.path).cmp(&(b.path != SKILL_FILE, &b.path))
            });
            skills.push(skill);
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.dir.cmp(&b.dir)));
        Ok(skills)
    }

    /// A file of skill `name` in `space` (its `SKILL.md` by default), for an
    /// assistant: only a file the skill lists, as text, with its digest.
    pub async fn read_skill(
        &self,
        space: &str,
        name: &str,
        file: Option<&str>,
    ) -> Result<SkillText, SessionError> {
        let skill = self
            .knowledge_skills(space)
            .await?
            .into_iter()
            .find(|s| s.name == name && s.problem.is_none())
            .ok_or_else(|| bad(format!("{space} has no skill {name}")))?;
        let path = file.unwrap_or(SKILL_FILE);
        let entry = skill
            .files
            .iter()
            .find(|f| f.path == path)
            .ok_or_else(|| bad(format!("skill {name} has no file {path}")))?;
        if entry.size > MAX_GET {
            return Err(bad(format!(
                "{path} is larger than {} KB; it is not returned",
                MAX_GET >> 10
            )));
        }
        let location = self.knowledge_location(space)?;
        let place = inplace::place_of(&self.catalog, self.engines(), &location)?;
        let bytes = place
            .read(&join(&skill.dir, path), 0..entry.size, MAX_GET)
            .await?;
        let sha256 = hex(&Sha256::digest(&bytes));
        let text = String::from_utf8(bytes).map_err(|_| bad(format!("{path} is not text")))?;
        Ok(SkillText {
            path: path.to_string(),
            text,
            sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_is_read() {
        let text = "---\nname: pdf-tools\ndescription: >\n  Fill PDF forms\n  and merge files.\nlicense: \"Apache-2.0\"\nmetadata:\n  author: x\n---\n# Body\n";
        let (fields, body) = front_matter(text).unwrap();
        assert_eq!(fields["name"], "pdf-tools");
        assert_eq!(fields["description"], "Fill PDF forms and merge files.");
        assert_eq!(fields["license"], "Apache-2.0");
        assert!(!fields.contains_key("author"));
        assert_eq!(body, "# Body\n");
        assert!(front_matter("# No front matter\n").is_err());
        assert!(front_matter("---\nname: x\n").is_err());
    }

    #[test]
    fn names_follow_the_format_and_the_folder() {
        assert_eq!(name_problem("pdf-tools", Some("pdf-tools")), None);
        assert!(name_problem("PDF", None).is_some());
        assert!(name_problem("a--b", None).is_some());
        assert!(name_problem("-a", None).is_some());
        assert!(name_problem("tools", Some("pdf-tools")).is_some());
        assert_eq!(name_problem("root-skill", None), None);
    }

    #[test]
    fn oikomes_own_skill_is_valid() {
        let (fields, body) = front_matter(OIKOME_SKILL).unwrap();
        assert_eq!(fields["name"], OIKOME_SKILL_NAME);
        assert_eq!(fields["description"], OIKOME_SKILL_DESCRIPTION);
        assert_eq!(name_problem(OIKOME_SKILL_NAME, None), None);
        assert!(body.contains("list_collections"));
    }
}
