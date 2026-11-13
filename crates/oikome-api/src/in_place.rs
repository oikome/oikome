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

//! Folders viewed in place in the owner's API (architecture v3 §7.7, 3.9):
//! a collection that reads a folder where it is, browsed from its index,
//! with photos' thumbnails and originals for the apps.
//!
//! The folder sources of M2 (`photos-folder`, `notes-folder`) copied every
//! file into a SQLite file; [`Api::view_folders_in_place`] turns each into a
//! collection of the same name that views the same folder, once.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use base64::Engine as _;
use oikome_catalog::{InPlaceSource, Location, LocationFormat};
use oikome_connect::SourceConfig;
use oikome_core::CreateSpaceRequest;
use oikome_node::inplace;

use crate::sources::slug;
use crate::{
    encode_rows, Api, ApiError, Caller, ContentCapabilities, ContentView, Empty, FileContent,
    FileFolder, FileItem, FileKind, FileListing, FileRef, FileSort, InPlace, ListFiles,
    ReadFileRows, Rows, ShowFolder, SpaceRef, SpaceSummary, ViewInPlace, DEFAULT_PAGE_ROWS,
    MAX_PAGE_ROWS, MAX_RESULT_BYTES,
};

/// Files listed per page at most.
const MAX_LIST: u64 = 1000;
const DEFAULT_LIST: u64 = 200;
/// The most of an original sent for one ranged request.
const MAX_RANGE: u64 = 8 << 20;

fn common_root(mut roots: Vec<std::path::PathBuf>) -> Result<std::path::PathBuf, ApiError> {
    let mut common = roots
        .pop()
        .ok_or_else(|| ApiError::invalid("choose at least one file"))?;
    for root in roots {
        while !root.starts_with(&common) {
            common = common
                .parent()
                .ok_or_else(|| ApiError::invalid("these files have no common folder"))?
                .to_path_buf();
        }
    }
    Ok(common)
}

fn canonical_files(files: &[String], private: &Path) -> Result<Vec<std::path::PathBuf>, ApiError> {
    if files.is_empty() {
        return Err(ApiError::invalid("choose at least one file"));
    }
    let private = std::fs::canonicalize(private).map_err(ApiError::internal)?;
    files
        .iter()
        .map(|path| {
            if path.is_empty()
                || path.chars().any(char::is_control)
                || !Path::new(path).is_absolute()
            {
                return Err(ApiError::invalid(format!(
                    "{path} is not an absolute file path"
                )));
            }
            let meta = std::fs::symlink_metadata(path)
                .map_err(|e| ApiError::invalid(format!("{path} cannot be read: {e}")))?;
            let file = std::fs::canonicalize(path)
                .map_err(|e| ApiError::invalid(format!("{path} cannot be read: {e}")))?;
            if !meta.file_type().is_file()
                || !file.is_file()
                || file.starts_with(&private)
                || file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_none_or(|name| name.starts_with('.'))
                || file
                    .components()
                    .any(|part| part.as_os_str() == oikome_catalog::registry::OIKOME_DIR)
            {
                return Err(ApiError::invalid(format!(
                    "{path} is not a file Oikome can add"
                )));
            }
            Ok(file)
        })
        .collect()
}

fn new_file_sources(
    files: &[std::path::PathBuf],
) -> Result<(String, Vec<InPlaceSource>), ApiError> {
    let common = common_root(
        files
            .iter()
            .filter_map(|file| file.parent().map(Path::to_path_buf))
            .collect(),
    )?;
    let files = files
        .iter()
        .map(|file| {
            file.strip_prefix(&common)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .map_err(|e| ApiError::invalid(e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root = common.to_string_lossy().into_owned();
    Ok((
        root.clone(),
        vec![InPlaceSource {
            root,
            files: Some(files),
        }],
    ))
}

fn add_selected_source(sources: &mut Vec<InPlaceSource>, file: &Path) -> Result<(), ApiError> {
    let root = file
        .parent()
        .ok_or_else(|| ApiError::invalid("the selected file has no parent folder"))?;
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ApiError::invalid("the selected file's name is not text"))?
        .to_string();
    let root_text = root.to_string_lossy();
    if let Some(source) = sources
        .iter_mut()
        .find(|source| source.root == root_text.as_ref())
    {
        if let Some(files) = &mut source.files {
            if !files.contains(&name) {
                files.push(name);
            }
        }
        return Ok(());
    }
    sources.push(InPlaceSource {
        root: root.to_string_lossy().into_owned(),
        files: Some(vec![name]),
    });
    Ok(())
}

/// An original image, audio or video file, or part of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Original {
    pub bytes: Vec<u8>,
    /// Where `bytes` start in the file, and its whole size.
    pub start: u64,
    pub total: u64,
    pub content_type: &'static str,
}

/// The bytes of the file or folder at `path` and when the newest file in it
/// changed (milliseconds since the Unix epoch); `None` if there is nothing
/// there. Links are not followed, and a folder of more files than
/// [`MEASURE_FILES`] is measured that far.
fn measure(path: &Path) -> Option<(u64, Option<u64>)> {
    let ms = |meta: &std::fs::Metadata| {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| u64::try_from(d.as_millis()).ok())
    };
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.is_file() {
        return Some((meta.len(), ms(&meta)));
    }
    if !meta.is_dir() {
        return None;
    }
    let (mut bytes, mut newest, mut seen) = (0u64, None, 0usize);
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                folders.push(entry.path());
            } else if meta.is_file() {
                bytes += meta.len();
                newest = newest.max(ms(&meta));
                seen += 1;
                if seen >= MEASURE_FILES {
                    return Some((bytes, newest));
                }
            }
        }
    }
    Some((bytes, newest))
}

/// Files measured at most for one collection's size.
const MEASURE_FILES: usize = 100_000;

fn kind_of(kind: &str) -> FileKind {
    match kind {
        inplace::IMAGE => FileKind::Image,
        inplace::NOTE => FileKind::Note,
        inplace::TABLE => FileKind::Table,
        _ => FileKind::Other,
    }
}

fn content_of(
    name: &str,
    kind: FileKind,
    modified_ms: u64,
    size: u64,
    local: bool,
    text: Option<&str>,
) -> FileContent {
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let format = match extension.as_str() {
        "md" | "markdown" => "markdown",
        "" => "file",
        other => other,
    };
    let media_type = match format {
        "markdown" => Some("text/markdown"),
        "txt" => Some("text/plain"),
        "csv" => Some("text/csv"),
        "tsv" => Some("text/tab-separated-values"),
        "json" | "jsonl" | "ndjson" => Some("application/json"),
        "parquet" => Some("application/vnd.apache.parquet"),
        _ => original_type(name),
    }
    .map(str::to_owned);
    let table_readable = !matches!(kind, FileKind::Table)
        || !matches!(format, "json" | "jsonl" | "ndjson")
        || text.is_none_or(|text| {
            oikome_connect::rows::read_page(
                oikome_engine::FileFormat::Json,
                name,
                text.as_bytes(),
                0,
                1,
                false,
            )
            .is_ok()
        });
    let original_view = match media_type.as_deref() {
        Some("image/svg+xml") => Some(ContentView::Image),
        Some(media) if media.starts_with("audio/") => Some(ContentView::Audio),
        Some(media) if media.starts_with("video/") => Some(ContentView::Video),
        _ => None,
    };
    let views = match kind {
        FileKind::Note if format == "markdown" => vec![ContentView::Read, ContentView::Source],
        FileKind::Note => vec![ContentView::Source],
        FileKind::Table if !table_readable => vec![ContentView::Source],
        FileKind::Table if matches!(format, "json" | "jsonl" | "ndjson") => {
            vec![ContentView::Table, ContentView::Source]
        }
        FileKind::Table => vec![ContentView::Table],
        FileKind::Image => vec![ContentView::Image],
        FileKind::Other => original_view.into_iter().collect(),
    };
    let unavailable_reason = if !table_readable {
        Some(format!(
            "This {format} file cannot be parsed as a table; its source is available"
        ))
    } else {
        views
            .is_empty()
            .then(|| format!("No built-in preview is available for this {format} file"))
    };
    FileContent {
        format: format.to_owned(),
        media_type,
        views,
        source: "in_place".into(),
        version: format!("{modified_ms}:{size}"),
        capabilities: ContentCapabilities {
            open_default: local,
            reveal: local,
            edit: false,
        },
        unavailable_reason,
    }
}

fn item_of(entry: inplace::FileEntry, local: bool) -> FileItem {
    let kind = kind_of(&entry.kind);
    let modified_ms = u64::try_from(entry.modified_ms).unwrap_or(0);
    let content = content_of(
        &entry.name,
        kind,
        modified_ms,
        entry.size,
        local,
        entry.text.as_deref(),
    );
    FileItem {
        kind,
        path: entry.path,
        name: entry.name,
        size: entry.size,
        modified: entry.modified,
        modified_ms,
        title: entry.title,
        taken: entry.taken,
        written: entry.written,
        place: entry.place,
        credit: entry.credit,
        text: entry.text,
        thumbnail: entry.thumbnail,
        content,
    }
}

/// The media type of an original Oikome's image or native media views show.
fn original_type(name: &str) -> Option<&'static str> {
    let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        _ => return None,
    })
}

impl Api {
    /// The Location of a collection that views a folder in place.
    fn in_place_of(&self, space: &str) -> Result<Location, ApiError> {
        self.session(None)
            .in_place_location(space)?
            .ok_or_else(|| ApiError::not_found(format!("{space} does not view a folder in place")))
    }

    /// What a collection's summary says of the folder it views, if it does.
    /// A collection's bytes and when it last changed, where this computer
    /// can tell: a folder viewed in place from its index, a SQLite file
    /// from the file, Iceberg and Lance tables from their files here.
    pub(crate) fn footprint(&self, location: &str) -> (Option<u64>, Option<u64>) {
        let Ok(location) = self.catalog.get_location(location) else {
            return (None, None);
        };
        let root = Path::new(self.catalog.root_path());
        let measured = match &location.format {
            LocationFormat::InPlace { .. } => inplace::totals(&self.catalog, &location)
                .ok()
                .map(|(_, bytes, newest)| (bytes, newest.and_then(|ms| u64::try_from(ms).ok()))),
            LocationFormat::SqliteFile { path } => {
                // The database and its write-ahead log.
                let file = root.join(path);
                let mut wal = file.clone().into_os_string();
                wal.push("-wal");
                let parts = [measure(&file), measure(Path::new(&wal))];
                (parts.iter().any(Option::is_some)).then(|| {
                    parts
                        .iter()
                        .flatten()
                        .fold((0, None), |(bytes, newest), (b, n)| {
                            (bytes + b, newest.max(*n))
                        })
                })
            }
            LocationFormat::IcebergParquet { namespace } => measure(&root.join(namespace)),
            LocationFormat::Lance { root: dir } => measure(&root.join(dir)),
            _ => None,
        };
        match measured {
            Some((bytes, newest)) => (Some(bytes), newest),
            None => (None, None),
        }
    }

    pub(crate) fn in_place_summary(&self, location: &str) -> Option<InPlace> {
        let location = self.catalog.get_location(location).ok()?;
        let LocationFormat::InPlace { place, path, .. } = &location.format else {
            return None;
        };
        let status = inplace::status(&self.catalog, &location).unwrap_or_default();
        Some(InPlace {
            place: place.clone(),
            path: path.clone(),
            files: status.files,
            scanned_at_ms: status.scanned_at_ms,
            reachable: status.reachable,
        })
    }

    async fn summary_of(&self, caller: &Caller, name: &str) -> Result<SpaceSummary, ApiError> {
        self.list_spaces(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| ApiError::internal("the collection is missing"))
    }

    /// Deliberately copy browser files into a new collection or the current
    /// folder. An empty analytical Space becomes file-backed on this first
    /// import; a non-empty one is never converted implicitly. Existing files
    /// are never replaced.
    pub async fn import_files(
        &self,
        caller: &Caller,
        req: crate::ImportFiles,
    ) -> Result<SpaceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        if req.files.is_empty() {
            return Err(ApiError::invalid("choose at least one file"));
        }
        let imports = std::path::PathBuf::from(self.catalog.root_path()).join("Imports");
        let (space_name, folder, create_folder, mode) = match (req.name, req.space) {
            (Some(name), None) => {
                oikome_core::validate_space_name(&name)?;
                if self.catalog.get_space(&name).is_ok() || self.catalog.get_location(&name).is_ok()
                {
                    return Err(ApiError::new(
                        crate::ErrorCode::Conflict,
                        format!("{name} is taken; choose another name"),
                    ));
                }
                let folder = imports.join(&name);
                if folder.exists() {
                    return Err(ApiError::new(
                        crate::ErrorCode::Conflict,
                        format!("the import folder for {name} already exists"),
                    ));
                }
                (name, folder, true, 0)
            }
            (None, Some(name)) => {
                let space = self.catalog.get_space(&name)?;
                let location = self.catalog.get_location(&space.location)?;
                match location.format {
                    LocationFormat::InPlace {
                        place: None, path, ..
                    } => {
                        let root = std::fs::canonicalize(path).map_err(|e| {
                            ApiError::invalid(format!("this folder cannot be reached: {e}"))
                        })?;
                        let within = req.folder.unwrap_or_default();
                        if !within.is_empty() && !inplace::is_path_within(&within) {
                            return Err(ApiError::invalid(
                                "choose a folder inside this collection",
                            ));
                        }
                        let folder = std::fs::canonicalize(root.join(within)).map_err(|e| {
                            ApiError::invalid(format!("this folder cannot be reached: {e}"))
                        })?;
                        if !folder.starts_with(&root) || !folder.is_dir() {
                            return Err(ApiError::invalid(
                                "choose a folder inside this collection",
                            ));
                        }
                        (name, folder, false, 2)
                    }
                    LocationFormat::InPlace { place: Some(_), .. } => {
                        return Err(ApiError::invalid(
                            "imports are available only in folders on this computer",
                        ))
                    }
                    LocationFormat::IcebergParquet { .. } => {
                        if req
                            .folder
                            .as_deref()
                            .is_some_and(|folder| !folder.is_empty())
                        {
                            return Err(ApiError::invalid("this collection has no subfolders"));
                        }
                        if space.kind != oikome_core::SpaceKind::Analytical
                            || self.catalog.get_source(&name).is_ok()
                            || !self
                                .session(None)
                                .readable_relations(&name)
                                .await?
                                .is_empty()
                        {
                            return Err(ApiError::new(
                                crate::ErrorCode::Conflict,
                                "only an empty collection can become a file collection on its first import",
                            ));
                        }
                        let folder = imports.join(&name);
                        if folder.exists() {
                            return Err(ApiError::new(
                                crate::ErrorCode::Conflict,
                                format!("the import folder for {name} already exists"),
                            ));
                        }
                        (name, folder, true, 1)
                    }
                    _ => {
                        return Err(ApiError::invalid(
                            "imports are available only in local file collections",
                        ))
                    }
                }
            }
            _ => {
                return Err(ApiError::invalid(
                    "choose either a new collection name or an existing collection",
                ))
            }
        };
        let decoded = Self::decode_import(req.files)?;
        if create_folder {
            if let Some(parent) = folder.parent() {
                std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
            }
            std::fs::create_dir(&folder).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    ApiError::new(
                        crate::ErrorCode::Conflict,
                        "this import folder already exists",
                    )
                } else {
                    ApiError::internal(e)
                }
            })?;
        }
        Self::write_import(&folder, &decoded, create_folder)?;
        match mode {
            0 => {
                if let Err(e) = self
                    .session(None)
                    .create_space(CreateSpaceRequest {
                        name: space_name.clone(),
                        place: Some(oikome_catalog::space::PLACE_LOCAL.to_string()),
                        path: Some(folder.to_string_lossy().into_owned()),
                        ..Default::default()
                    })
                    .await
                {
                    Self::discard_import(&folder, &decoded, create_folder);
                    return Err(e.into());
                }
            }
            1 => {
                if let Err(e) = self.catalog.make_space_in_place(
                    &space_name,
                    folder.to_string_lossy().into_owned(),
                    None,
                ) {
                    Self::discard_import(&folder, &decoded, create_folder);
                    return Err(e.into());
                }
                self.session(None).refresh_in_place(&space_name).await?;
            }
            _ => {
                let space = self.catalog.get_space(&space_name)?;
                let location = self.catalog.get_location(&space.location)?;
                if let LocationFormat::InPlace {
                    path,
                    sources: Some(mut sources),
                    ..
                } = location.format
                {
                    for (name, _) in &decoded {
                        add_selected_source(&mut sources, &folder.join(name))?;
                    }
                    if let Err(e) = self.catalog.add_in_place_files(&space_name, path, sources) {
                        Self::discard_import(&folder, &decoded, create_folder);
                        return Err(e.into());
                    }
                }
                self.session(None).refresh_in_place(&space_name).await?;
            }
        }
        self.summary_of(caller, &space_name).await
    }

    fn decode_import(files: Vec<crate::ImportFile>) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
        let mut names = BTreeSet::new();
        let mut decoded = Vec::with_capacity(files.len());
        let mut total = 0usize;
        for file in files {
            let name = file.name;
            if name.is_empty()
                || name.len() > 255
                || matches!(name.as_str(), "." | "..")
                || name != name.trim()
                || name.contains(['/', '\\'])
                || name.chars().any(char::is_control)
                || !names.insert(name.clone())
            {
                return Err(ApiError::invalid(format!(
                    "{name:?} is not a unique file name without a directory"
                )));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(file.data_base64)
                .map_err(|_| ApiError::invalid(format!("{name} is not valid base64")))?;
            total = total.saturating_add(bytes.len());
            if total > 32 << 20 {
                return Err(ApiError::invalid("one import is limited to 32 MiB"));
            }
            decoded.push((name, bytes));
        }
        Ok(decoded)
    }

    fn write_import(
        folder: &Path,
        decoded: &[(String, Vec<u8>)],
        remove_folder: bool,
    ) -> Result<(), ApiError> {
        let mut written = Vec::new();
        for (name, bytes) in decoded {
            let path = folder.join(name);
            let write = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut file| file.write_all(bytes));
            if let Err(e) = write {
                for path in written {
                    let _ = std::fs::remove_file(path);
                }
                if remove_folder {
                    let _ = std::fs::remove_dir(folder);
                }
                return Err(if e.kind() == std::io::ErrorKind::AlreadyExists {
                    ApiError::new(crate::ErrorCode::Conflict, format!("{name} already exists"))
                } else {
                    ApiError::internal(e)
                });
            }
            written.push(path);
        }
        Ok(())
    }

    fn discard_import(folder: &Path, decoded: &[(String, Vec<u8>)], remove_folder: bool) {
        for (name, _) in decoded {
            let _ = std::fs::remove_file(folder.join(name));
        }
        if remove_folder {
            let _ = std::fs::remove_dir(folder);
        }
    }

    /// Add original local files to a collection without copying them. Each
    /// source keeps its own root and optional selected-file list, so adding a
    /// file never exposes unrelated siblings from its parent folder.
    pub async fn add_in_place_files(
        &self,
        caller: &Caller,
        req: crate::AddInPlaceFiles,
    ) -> Result<SpaceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let files = canonical_files(&req.files, Path::new(self.catalog.oikome_dir().as_path()))?;
        let space = self.catalog.get_space(&req.space)?;
        let location = self.catalog.get_location(&space.location)?;
        match location.format {
            LocationFormat::InPlace {
                place: None,
                path,
                sources,
            } => {
                let mut sources = sources.unwrap_or_else(|| {
                    vec![InPlaceSource {
                        root: path.clone(),
                        files: None,
                    }]
                });
                for file in &files {
                    add_selected_source(&mut sources, file)?;
                }
                let mut roots = vec![std::path::PathBuf::from(path)];
                roots.extend(
                    files
                        .iter()
                        .filter_map(|file| file.parent().map(Path::to_path_buf)),
                );
                let root = common_root(roots)?.to_string_lossy().into_owned();
                self.catalog.add_in_place_files(&req.space, root, sources)?;
            }
            LocationFormat::InPlace { place: Some(_), .. } => {
                return Err(ApiError::invalid(
                    "file references can be added only on this computer",
                ));
            }
            LocationFormat::IcebergParquet { .. } => {
                if space.kind != oikome_core::SpaceKind::Analytical
                    || self.catalog.get_source(&req.space).is_ok()
                    || !self
                        .session(None)
                        .readable_relations(&req.space)
                        .await?
                        .is_empty()
                {
                    return Err(ApiError::new(
                        crate::ErrorCode::Conflict,
                        "only an empty collection can become a file collection",
                    ));
                }
                let (root, sources) = new_file_sources(&files)?;
                self.catalog
                    .make_space_in_place(&req.space, root, Some(sources))?;
            }
            _ => {
                return Err(ApiError::invalid(
                    "this collection cannot hold original file references",
                ))
            }
        }
        let space = self.catalog.get_space(&req.space)?;
        let location = self.catalog.get_location(&space.location)?;
        match std::fs::remove_dir_all(self.catalog.index_dir(&location)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ApiError::internal(e)),
        }
        self.session(None).refresh_in_place(&req.space).await?;
        self.summary_of(caller, &req.space).await
    }

    /// A new collection that views a folder where it is: on this computer
    /// (a full path), or within a storage. Nothing is copied; its files are
    /// read now and kept up with every minute.
    pub async fn view_in_place(
        &self,
        caller: &Caller,
        req: ViewInPlace,
    ) -> Result<SpaceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (path, include) = match req.files.as_deref() {
            Some(files) => {
                if req.place.as_deref().is_some_and(|place| {
                    !place.is_empty() && place != oikome_catalog::space::PLACE_LOCAL
                }) {
                    return Err(ApiError::invalid(
                        "selected file references are available only on this computer",
                    ));
                }
                let files = canonical_files(files, Path::new(self.catalog.oikome_dir().as_path()))?;
                let (root, mut sources) = new_file_sources(&files)?;
                let include = sources.pop().and_then(|source| source.files);
                (root, include)
            }
            None => {
                let path = req.path.trim().to_string();
                if path.is_empty() {
                    return Err(ApiError::invalid("choose a folder"));
                }
                (path, None)
            }
        };
        let place = req
            .place
            .clone()
            .filter(|p| !p.is_empty() && p != oikome_catalog::space::PLACE_LOCAL);
        if let Some(storage) = &place {
            self.catalog.get_storage(storage)?;
        }
        let name = match req.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => {
                oikome_core::validate_space_name(name)?;
                if self.catalog.get_space(name).is_ok() || self.catalog.get_location(name).is_ok() {
                    return Err(ApiError::new(
                        crate::ErrorCode::Conflict,
                        format!("{name} is taken; choose another name"),
                    ));
                }
                name.to_string()
            }
            _ => self.free_folder_name(&path),
        };
        self.session(None)
            .create_space(CreateSpaceRequest {
                name: name.clone(),
                place: Some(
                    place
                        .clone()
                        .unwrap_or_else(|| oikome_catalog::space::PLACE_LOCAL.to_string()),
                ),
                path: Some(path.to_string()),
                include,
                ..Default::default()
            })
            .await?;
        self.summary_of(caller, &name).await
    }

    /// A free collection name made from a folder's own.
    fn free_folder_name(&self, path: &str) -> String {
        let last = path
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default();
        let base = match slug(last) {
            s if s.starts_with(|c: char| c.is_ascii_alphabetic()) => s,
            s if s.is_empty() => "folder".to_string(),
            s => format!("folder-{s}"),
        };
        let taken = |name: &str| {
            self.catalog.get_space(name).is_ok()
                || self.catalog.get_location(name).is_ok()
                || self.catalog.get_source(name).is_ok()
        };
        std::iter::once(base.clone())
            .chain((2..).map(|i| format!("{base}-{i}")))
            .find(|n| !taken(n))
            .unwrap_or(base)
    }

    /// Read a folder viewed in place again now.
    pub async fn refresh_space(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<SpaceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.in_place_of(&req.space)?;
        self.session(None).refresh_in_place(&req.space).await?;
        self.summary_of(caller, &req.space).await
    }

    /// Open a folder viewed in place on this computer in its file manager,
    /// or one file of it in its folder.
    pub async fn show_folder(&self, caller: &Caller, req: ShowFolder) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        let location = self.in_place_of(&req.space)?;
        let LocationFormat::InPlace {
            place: None, path, ..
        } = &location.format
        else {
            return Err(ApiError::invalid(format!(
                "{} is on another place; open it there",
                req.space
            )));
        };
        let within = req.path.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let target = match within {
            Some(within) => {
                if !inplace::is_path_within(within) {
                    return Err(ApiError::invalid(format!("{within} is not a file in it")));
                }
                if inplace::file(&self.catalog, &location, within)?.is_none() {
                    return Err(ApiError::not_found(format!(
                        "{} has no file {}",
                        req.space, within
                    )));
                }
                Path::new(path).join(within)
            }
            None => Path::new(path).to_path_buf(),
        };
        if !target.exists() {
            return Err(ApiError::invalid(format!(
                "{} isn't there right now",
                target.display()
            )));
        }
        crate::sources::reveal(&target, target.is_dir()).map_err(ApiError::internal)?;
        Ok(Empty {})
    }

    /// Open an indexed file on this computer with its default application.
    pub async fn open_with(&self, caller: &Caller, req: FileRef) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        let LocationFormat::InPlace {
            place: None, path, ..
        } = &location.format
        else {
            return Err(ApiError::invalid(format!(
                "{} is not on this computer; open it there",
                req.space
            )));
        };
        if !inplace::is_path_within(&req.path) {
            return Err(ApiError::invalid(format!(
                "{} is not a file in it",
                req.path
            )));
        }
        if inplace::file(&self.catalog, &location, &req.path)?.is_none() {
            return Err(ApiError::not_found(format!(
                "{} has no file {}",
                req.space, req.path
            )));
        }
        let target = Path::new(path).join(&req.path);
        if !target.is_file() {
            return Err(ApiError::invalid(format!(
                "{} isn't there right now",
                target.display()
            )));
        }
        crate::sources::open(&target).map_err(ApiError::internal)?;
        Ok(Empty {})
    }

    /// A page of the rows of one data file (a CSV, TSV, Parquet or JSON, at
    /// any folder) of a collection that views a folder in place: the path is
    /// checked first, the file is read where it is (a NAS's or a drive's
    /// through its cache) and parsed by its own reader, no SQL involved.
    pub async fn read_file_rows(
        &self,
        caller: &Caller,
        req: ReadFileRows,
    ) -> Result<Rows, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        let limit = req
            .limit
            .unwrap_or(DEFAULT_PAGE_ROWS)
            .clamp(1, MAX_PAGE_ROWS);
        let offset = req.offset.unwrap_or(0);
        if inplace::file(&self.catalog, &location, &req.path)?.is_none() {
            return Err(ApiError::not_found(format!(
                "{} has no file {}",
                req.space, req.path
            )));
        }
        let parsed = inplace::file_rows(
            &self.catalog,
            &self.engines,
            &location,
            &req.path,
            offset,
            limit as usize,
        )
        .await?;
        let mut rows = encode_rows(&parsed.schema, &parsed.rows, usize::MAX, MAX_RESULT_BYTES)?;
        rows.truncated |= parsed.cut;
        Ok(rows)
    }

    /// One folder of a collection viewed in place, from its index (so also
    /// while the place is offline): the folders in it and a page of files.
    pub async fn list_files(
        &self,
        caller: &Caller,
        req: ListFiles,
    ) -> Result<FileListing, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        let local = matches!(
            &location.format,
            LocationFormat::InPlace { place: None, .. }
        );
        let folder = req.folder.unwrap_or_default();
        let folder = folder.trim_matches('/');
        if !folder.is_empty() && !inplace::is_path_within(folder) {
            return Err(ApiError::invalid(format!("{folder} is not a folder in it")));
        }
        let limit = req.limit.unwrap_or(DEFAULT_LIST).clamp(1, MAX_LIST);
        let offset = req.offset.unwrap_or(0);
        let listing = if req.deep == Some(true) {
            let kind = req.kind.map(|k| match k {
                FileKind::Image => inplace::IMAGE,
                FileKind::Note => inplace::NOTE,
                FileKind::Table => inplace::TABLE,
                FileKind::Other => inplace::OTHER,
            });
            inplace::list_deep(&self.catalog, &location, kind, offset, limit)?
        } else {
            let order = match req.sort {
                None => inplace::FileOrder::Kinds,
                Some(FileSort::Name) => inplace::FileOrder::Name,
                Some(FileSort::Modified) => inplace::FileOrder::Modified,
                Some(FileSort::Size) => inplace::FileOrder::Size,
                Some(FileSort::Kind) => inplace::FileOrder::Kind,
            };
            let descending = req.descending.unwrap_or(false);
            let mut listing = inplace::list(
                &self.catalog,
                &location,
                folder,
                (order, descending),
                offset,
                limit,
            )?;
            if let Some(kind) = req.kind {
                listing.files.retain(|f| kind_of(&f.kind) == kind);
            }
            listing
        };
        Ok(FileListing {
            folders: listing
                .folders
                .into_iter()
                .map(|f| FileFolder {
                    name: f.name,
                    files: f.files,
                    size: f.size,
                    modified_ms: f.modified_ms.and_then(|ms| u64::try_from(ms).ok()),
                    photos: f.photos,
                    notes: f.notes,
                    cover: f
                        .cover
                        .into_iter()
                        .map(|entry| item_of(entry, local))
                        .collect(),
                })
                .collect(),
            files: listing
                .files
                .into_iter()
                .map(|entry| item_of(entry, local))
                .collect(),
            total: listing.total,
        })
    }

    /// One file of a collection viewed in place, with all of a note's text.
    pub async fn read_file(&self, caller: &Caller, req: FileRef) -> Result<FileItem, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        let local = matches!(
            &location.format,
            LocationFormat::InPlace { place: None, .. }
        );
        inplace::file(&self.catalog, &location, &req.path)?
            .map(|entry| item_of(entry, local))
            .ok_or_else(|| ApiError::not_found(format!("{} has no file {}", req.space, req.path)))
    }

    /// A photo's thumbnail (JPEG), from the index.
    pub async fn file_thumbnail(&self, caller: &Caller, req: FileRef) -> Result<Vec<u8>, ApiError> {
        caller.require_owner()?;
        let location = self.in_place_of(&req.space)?;
        inplace::thumb(&self.catalog, &location, &req.path)?
            .ok_or_else(|| ApiError::not_found("no thumbnail"))
    }

    /// An image, audio or video original, or the one `range` asked for: only
    /// an indexed file with a built-in original view, read where it is.
    pub async fn file_original(
        &self,
        caller: &Caller,
        req: FileRef,
        range: Option<(u64, Option<u64>)>,
    ) -> Result<Original, ApiError> {
        caller.require_owner()?;
        let location = self.in_place_of(&req.space)?;
        let file = inplace::file(&self.catalog, &location, &req.path)?.ok_or_else(|| {
            ApiError::not_found(format!("{} has no file {}", req.space, req.path))
        })?;
        let content_type = original_type(&file.name)
            .ok_or_else(|| ApiError::not_found("not an original Oikome shows"))?;
        let total = file.size;
        let (start, end) = match range {
            None => (0, total),
            Some((start, end)) => {
                let end = end.map_or(total, |e| e.saturating_add(1)).min(total);
                if start >= end {
                    return Err(ApiError::invalid("the range is outside the file"));
                }
                (start, end.min(start.saturating_add(MAX_RANGE)))
            }
        };
        if range.is_none() && total > inplace::MAX_IMAGE_BYTES {
            return Err(ApiError::invalid("ask for a large original in ranges"));
        }
        let place = inplace::place_of(&self.catalog, &self.engines, &location)?;
        if !place.reachable().await {
            return Err(ApiError::from(inplace::offline(&place.label())));
        }
        let bytes = place.read(&file.path, start..end, end - start).await?;
        Ok(Original {
            start,
            total,
            content_type,
            bytes,
        })
    }

    /// Turn the folder sources of photos and notes into collections that
    /// view the same folders in place, once: the copies in SQLite are
    /// deleted (the files stay where they are), the collection keeps its
    /// name, and whoever it was granted to still is. Views, column grants
    /// and row policies written for the old `photos` and `notes` tables
    /// can't apply to `files` and go with them.
    pub async fn view_folders_in_place(&self) -> Result<(), ApiError> {
        self.refresh()?;
        if self.settings()?.folders_in_place {
            return Ok(());
        }
        let session = self.session(None);
        for source in self.catalog.list_sources() {
            let path = match serde_json::from_value::<SourceConfig>(source.config.clone()) {
                Ok(SourceConfig::PhotosFolder { path } | SourceConfig::NotesFolder { path }) => {
                    path
                }
                _ => continue,
            };
            let name = source.name.clone();
            let granted: Vec<String> = self
                .catalog
                .list_space_grants()
                .into_iter()
                .filter(|g| g.space == name)
                .map(|g| g.principal)
                .collect();
            session.remove_source(&name)?;
            if let Ok(space) = self.catalog.get_space(&name) {
                let location = self.catalog.get_location(&space.location)?;
                session.drop_space(&name).await?;
                if let Ok(file) = self.catalog.sqlite_path(&location) {
                    for suffix in ["", "-wal", "-shm"] {
                        let mut path = file.clone().into_os_string();
                        path.push(suffix);
                        let _ = std::fs::remove_file(path);
                    }
                }
                self.catalog.forget_location(&location.id)?;
            }
            if !Path::new(&path).is_dir() {
                tracing::warn!(%name, %path, "a folder source's folder is gone; not viewed in place");
                continue;
            }
            session
                .create_space(CreateSpaceRequest {
                    name: name.clone(),
                    place: Some(oikome_catalog::space::PLACE_LOCAL.to_string()),
                    path: Some(path),
                    ..Default::default()
                })
                .await?;
            if !granted.is_empty() {
                session.grant_space(&name, &granted)?;
            }
            tracing::info!(%name, "a folder source is viewed in place now");
        }
        self.update_settings(|s| s.folders_in_place = true)?;
        Ok(())
    }
}
