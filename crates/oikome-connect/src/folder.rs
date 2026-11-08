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

//! Files in a folder as a table, the way a notes app keeps a vault: the
//! files stay the owner's, editable with any app, and each pull reads the
//! folder again.
//!
//! - [`PhotosFolder`]: every image (JPEG, PNG, GIF, WebP) is a row of
//!   `photos`, its title the file's name and its time the file's. An
//!   optional `captions.csv` beside them (`file`, and any of `title`,
//!   `place`, `taken`, `credit`) names them.
//! - [`NotesFolder`]: every Markdown or text file is a row of `notes`,
//!   written when its front matter's `date` says, or when the file was last
//!   changed.
//!
//! Only the folder's own files are read (not its subfolders, hidden files
//! or links). A pull lists every file, so a file removed from the folder is
//! removed from the table; an unchanged folder (same names, sizes and
//! times) is not read again.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use arrow_array::{Array, StringArray};
use arrow_csv::reader::Format;
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;

use crate::{ColumnDef, ColumnKind, ConnectError, Connector, Pull, Row, TableDef, Value};

pub const PHOTOS_KIND: &str = "photos-folder";
pub const NOTES_KIND: &str = "notes-folder";

/// The file that names the photos of a folder.
pub const CAPTIONS: &str = "captions.csv";

const PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];
const NOTE_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];
/// Larger files are skipped rather than read whole.
const MAX_PHOTO_BYTES: u64 = 32 << 20;
const MAX_NOTE_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 10_000;

/// A file of the folder, as listed.
struct Entry {
    name: String,
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

/// The folder's own regular files with one of `extensions`, by name.
fn list(dir: &Path, extensions: &[&str], max_bytes: u64) -> Result<Vec<Entry>, ConnectError> {
    let read =
        std::fs::read_dir(dir).map_err(|e| ConnectError::Io(format!("{}: {e}", dir.display())))?;
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|e| ConnectError::Io(format!("{}: {e}", dir.display())))?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let wanted = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| extensions.contains(&e.to_ascii_lowercase().as_str()));
        if !wanted {
            continue;
        }
        // Links are not followed: only what is in the folder itself.
        let meta = item
            .metadata()
            .map_err(|e| ConnectError::Io(format!("{name}: {e}")))?;
        if !meta.is_file() || meta.len() > max_bytes {
            continue;
        }
        entries.push(Entry {
            name,
            path: item.path(),
            len: meta.len(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
        if entries.len() > MAX_FILES {
            return Err(ConnectError::Format(format!(
                "{} has more than {MAX_FILES} files",
                dir.display()
            )));
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// A cursor that changes when a file is added, removed or changed.
fn cursor_of(entries: &[Entry], extra: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    for e in entries {
        let ns = e
            .modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        hasher.update(format!("{}\0{}\0{ns}\n", e.name, e.len).as_bytes());
    }
    hasher.update(extra);
    hasher.finalize().to_hex().to_string()
}

/// A file time as this computer's clock reads it: `2025-11-07 09:40:00`.
/// A file time as local wall time, `2025-11-07 09:40:00`.
pub fn wall_time(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// `2025-11-07`, `2025-11-07 09:40` or `2025-11-07T09:40:00` as written in
/// a note or a caption, in one form; anything else is not a time.
pub fn written_time(text: &str) -> Option<String> {
    let text = text.trim().trim_matches(|c| c == '"' || c == '\'');
    let (day, time) = match text.split_once([' ', 'T']) {
        Some((day, time)) => (day, Some(time.trim_end_matches('Z'))),
        None => (text, None),
    };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let mut parts = day.split('-');
    let ok_day = matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(y), Some(m), Some(d), None) if digits(y, 4) && digits(m, 2) && digits(d, 2)
    );
    if !ok_day {
        return None;
    }
    let Some(time) = time else {
        return Some(day.to_string());
    };
    let pieces: Vec<&str> = time.split(':').collect();
    match pieces.as_slice() {
        [h, m] if digits(h, 2) && digits(m, 2) => Some(format!("{day} {h}:{m}:00")),
        [h, m, s] if digits(h, 2) && digits(m, 2) && digits(&s[..s.len().min(2)], 2) => {
            Some(format!("{day} {h}:{m}:{}", &s[..2]))
        }
        _ => None,
    }
}

fn text_or_null(text: Option<String>) -> Value {
    text.filter(|t| !t.is_empty())
        .map_or(Value::Null, Value::Text)
}

/// Photos in a folder.
pub struct PhotosFolder {
    dir: PathBuf,
}

impl PhotosFolder {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn table() -> TableDef {
        let column = |name: &str, kind| ColumnDef {
            name: name.to_string(),
            kind,
        };
        TableDef {
            name: "photos".to_string(),
            columns: vec![
                column("file", ColumnKind::Text),
                column("title", ColumnKind::Text),
                column("place", ColumnKind::Text),
                column("taken", ColumnKind::Timestamp),
                column("photo", ColumnKind::Blob),
                column("credit", ColumnKind::Text),
            ],
            key: vec!["file".to_string()],
        }
    }

    /// `captions.csv`, by file name: title, place, taken and credit.
    fn captions(&self, bytes: &[u8]) -> Result<HashMap<String, [Option<String>; 4]>, ConnectError> {
        captions_of(bytes)
    }
}

#[async_trait]
impl Connector for PhotosFolder {
    fn kind(&self) -> &'static str {
        PHOTOS_KIND
    }

    fn tables(&self) -> Vec<TableDef> {
        vec![Self::table()]
    }

    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError> {
        let dir = self.dir.clone();
        let entries =
            tokio::task::spawn_blocking(move || list(&dir, PHOTO_EXTENSIONS, MAX_PHOTO_BYTES))
                .await
                .map_err(|e| ConnectError::Io(e.to_string()))??;
        let captions_bytes = match tokio::fs::read(self.dir.join(CAPTIONS)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(ConnectError::Io(format!("{CAPTIONS}: {e}"))),
        };
        let hash = cursor_of(&entries, &captions_bytes);
        if cursor == Some(hash.as_str()) {
            return Ok(Pull::unchanged(Some(hash)));
        }
        let captions = self.captions(&captions_bytes)?;
        let mut rows: Vec<Row> = Vec::with_capacity(entries.len());
        for entry in &entries {
            let bytes = tokio::fs::read(&entry.path)
                .await
                .map_err(|e| ConnectError::Io(format!("{}: {e}", entry.name)))?;
            let [title, place, taken, credit] =
                captions.get(&entry.name).cloned().unwrap_or_default();
            let stem = Path::new(&entry.name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned());
            rows.push(vec![
                Value::Text(entry.name.clone()),
                text_or_null(title.or(stem)),
                text_or_null(place),
                Value::Text(
                    taken
                        .as_deref()
                        .and_then(written_time)
                        .unwrap_or_else(|| wall_time(entry.modified)),
                ),
                Value::Blob(bytes),
                text_or_null(credit),
            ]);
        }
        Ok(Pull {
            upserts: vec![("photos".to_string(), rows)],
            deletes: Vec::new(),
            cursor: Some(hash),
            complete: true,
        })
    }
}

/// Notes in a folder: Markdown or text files.
pub struct NotesFolder {
    dir: PathBuf,
}

impl NotesFolder {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn table() -> TableDef {
        let column = |name: &str, kind| ColumnDef {
            name: name.to_string(),
            kind,
        };
        TableDef {
            name: "notes".to_string(),
            columns: vec![
                column("file", ColumnKind::Text),
                column("written", ColumnKind::Timestamp),
                column("note", ColumnKind::Text),
            ],
            key: vec!["file".to_string()],
        }
    }
}

/// A note's text without its front matter, and the time the front matter
/// gives (`date`, `written` or `created`).
pub fn split_front_matter(text: &str) -> (Option<String>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, text);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, text);
    };
    let body = rest[end + 4..].trim_start_matches(['-']).trim_start();
    let date = rest[..end].lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        matches!(
            key.trim().to_ascii_lowercase().as_str(),
            "date" | "written" | "created"
        )
        .then(|| written_time(value))
        .flatten()
    });
    (date, body)
}

#[async_trait]
impl Connector for NotesFolder {
    fn kind(&self) -> &'static str {
        NOTES_KIND
    }

    fn tables(&self) -> Vec<TableDef> {
        vec![Self::table()]
    }

    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError> {
        let dir = self.dir.clone();
        let entries =
            tokio::task::spawn_blocking(move || list(&dir, NOTE_EXTENSIONS, MAX_NOTE_BYTES))
                .await
                .map_err(|e| ConnectError::Io(e.to_string()))??;
        let hash = cursor_of(&entries, &[]);
        if cursor == Some(hash.as_str()) {
            return Ok(Pull::unchanged(Some(hash)));
        }
        let mut rows: Vec<Row> = Vec::with_capacity(entries.len());
        for entry in &entries {
            let bytes = tokio::fs::read(&entry.path)
                .await
                .map_err(|e| ConnectError::Io(format!("{}: {e}", entry.name)))?;
            let text = String::from_utf8_lossy(&bytes);
            let (date, body) = split_front_matter(&text);
            rows.push(vec![
                Value::Text(entry.name.clone()),
                Value::Text(date.unwrap_or_else(|| wall_time(entry.modified))),
                Value::Text(body.trim().to_string()),
            ]);
        }
        Ok(Pull {
            upserts: vec![("notes".to_string(), rows)],
            deletes: Vec::new(),
            cursor: Some(hash),
            complete: true,
        })
    }
}

/// `captions.csv`, by file name: title, place, taken and credit. Also read
/// for folders viewed in place (§7.7).
pub fn captions_of(bytes: &[u8]) -> Result<HashMap<String, [Option<String>; 4]>, ConnectError> {
    let mut out = HashMap::new();
    if bytes.is_empty() {
        return Ok(out);
    }
    let bad = |e: &dyn std::fmt::Display| ConnectError::Format(format!("{CAPTIONS}: {e}"));
    let (header, _) = Format::default()
        .with_header(true)
        .infer_schema(Cursor::new(bytes), Some(1))
        .map_err(|e| bad(&e))?;
    let names: Vec<String> = header
        .fields()
        .iter()
        .map(|f| f.name().trim().to_ascii_lowercase())
        .collect();
    let at = |name: &str| names.iter().position(|n| n == name);
    let file = at("file").ok_or_else(|| bad(&"it needs a 'file' column"))?;
    let wanted = [at("title"), at("place"), at("taken"), at("credit")];
    let schema = Arc::new(Schema::new(
        names
            .iter()
            .map(|n| Field::new(n, DataType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let reader = arrow_csv::ReaderBuilder::new(schema)
        .with_header(true)
        .build(Cursor::new(bytes))
        .map_err(|e| bad(&e))?;
    for batch in reader {
        let batch = batch.map_err(|e| bad(&e))?;
        let column = |i: usize| batch.column(i).as_any().downcast_ref::<StringArray>();
        let text = |i: usize, row: usize| {
            column(i)
                .filter(|c| !c.is_null(row))
                .map(|c| c.value(row).trim().to_string())
                .filter(|t| !t.is_empty())
        };
        for row in 0..batch.num_rows() {
            let Some(name) = text(file, row) else {
                continue;
            };
            out.insert(name, wanted.map(|c| c.and_then(|c| text(c, row))));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_in_the_forms_people_write() {
        assert_eq!(written_time("2025-11-07").as_deref(), Some("2025-11-07"));
        assert_eq!(
            written_time("2025-11-07 09:40").as_deref(),
            Some("2025-11-07 09:40:00")
        );
        assert_eq!(
            written_time("2025-11-07T09:40:12Z").as_deref(),
            Some("2025-11-07 09:40:12")
        );
        assert_eq!(
            written_time("'2025-11-07 09:40:12'").as_deref(),
            Some("2025-11-07 09:40:12")
        );
        for bad in [
            "",
            "yesterday",
            "2025-1-7",
            "2025-11-07 9:40",
            "2025-11-07 09:40:xx",
        ] {
            assert_eq!(written_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn front_matter_gives_the_time_and_is_left_out() {
        let (date, body) = split_front_matter(
            "---\ndate: 2025-11-07 08:15\ntags: [lisbon]\n---\n\nPastéis first.\n",
        );
        assert_eq!(date.as_deref(), Some("2025-11-07 08:15:00"));
        assert_eq!(body.trim(), "Pastéis first.");
        let (date, body) = split_front_matter("No front matter.");
        assert_eq!((date, body), (None, "No front matter."));
    }

    #[tokio::test]
    async fn a_folder_of_photos_and_notes_is_read_and_re_read() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 0];
        std::fs::write(dir.path().join("Tram 28E.jpg"), jpeg).unwrap();
        std::fs::write(dir.path().join("Belém.JPG"), jpeg).unwrap();
        std::fs::write(dir.path().join(".hidden.jpg"), jpeg).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a photo").unwrap();
        std::fs::write(
            dir.path().join(CAPTIONS),
            "file,title,place,taken,credit\nTram 28E.jpg,Tram 28E,Graça,2025-11-08 10:15,\"CC BY 4.0\"\n",
        )
        .unwrap();
        let photos = PhotosFolder::new(dir.path().into());
        let pull = photos.pull(None).await.unwrap();
        let rows = &pull.upserts[0].1;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], Value::Text("Belém.JPG".into()));
        assert_eq!(rows[0][1], Value::Text("Belém".into()));
        assert_eq!(rows[1][2], Value::Text("Graça".into()));
        assert_eq!(rows[1][3], Value::Text("2025-11-08 10:15:00".into()));
        assert_eq!(rows[1][4], Value::Blob(jpeg.to_vec()));
        assert!(pull.complete);
        let again = photos.pull(pull.cursor.as_deref()).await.unwrap();
        assert!(
            again.upserts.is_empty(),
            "an unchanged folder is not read again"
        );

        let notes = NotesFolder::new(dir.path().into());
        let pull = notes.pull(None).await.unwrap();
        let rows = &pull.upserts[0].1;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][2], Value::Text("not a photo".into()));
    }
}
