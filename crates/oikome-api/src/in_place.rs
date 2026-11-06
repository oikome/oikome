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

use std::path::Path;

use oikome_catalog::{Location, LocationFormat};
use oikome_connect::SourceConfig;
use oikome_core::CreateSpaceRequest;
use oikome_node::inplace;

use crate::sources::slug;
use crate::{
    Api, ApiError, Caller, Empty, FileFolder, FileItem, FileKind, FileListing, FileRef, InPlace,
    ListFiles, SpaceRef, SpaceSummary, ViewInPlace,
};

/// Files listed per page at most.
const MAX_LIST: u64 = 1000;
const DEFAULT_LIST: u64 = 200;
/// The most of an original sent for one ranged request.
const MAX_RANGE: u64 = 8 << 20;

/// An original photo, or part of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Original {
    pub bytes: Vec<u8>,
    /// Where `bytes` start in the file, and its whole size.
    pub start: u64,
    pub total: u64,
    pub content_type: &'static str,
}

fn kind_of(kind: &str) -> FileKind {
    match kind {
        inplace::IMAGE => FileKind::Image,
        inplace::NOTE => FileKind::Note,
        inplace::TABLE => FileKind::Table,
        _ => FileKind::Other,
    }
}

fn item_of(entry: inplace::FileEntry) -> FileItem {
    FileItem {
        kind: kind_of(&entry.kind),
        path: entry.path,
        name: entry.name,
        size: entry.size,
        modified: entry.modified,
        title: entry.title,
        taken: entry.taken,
        written: entry.written,
        place: entry.place,
        credit: entry.credit,
        text: entry.text,
        thumbnail: entry.thumbnail,
    }
}

/// The media type of an image Oikome shows, by its name.
fn image_type(name: &str) -> Option<&'static str> {
    let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
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
    pub(crate) fn in_place_summary(&self, location: &str) -> Option<InPlace> {
        let location = self.catalog.get_location(location).ok()?;
        let LocationFormat::InPlace { place, path } = &location.format else {
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
        let path = req.path.trim().to_string();
        if path.is_empty() {
            return Err(ApiError::invalid("choose a folder"));
        }
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
                path: Some(path),
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

    /// Open a folder viewed in place on this computer in its file manager.
    pub async fn show_folder(&self, caller: &Caller, req: SpaceRef) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        let location = self.in_place_of(&req.space)?;
        let LocationFormat::InPlace { place: None, path } = &location.format else {
            return Err(ApiError::invalid(format!(
                "{} is on another place; open it there",
                req.space
            )));
        };
        let path = Path::new(path);
        if !path.is_dir() {
            return Err(ApiError::invalid(format!(
                "{} isn't there right now",
                path.display()
            )));
        }
        crate::sources::reveal(path, true).map_err(ApiError::internal)?;
        Ok(Empty {})
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
            let mut listing = inplace::list(&self.catalog, &location, folder, offset, limit)?;
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
                })
                .collect(),
            files: listing.files.into_iter().map(item_of).collect(),
            total: listing.total,
        })
    }

    /// One file of a collection viewed in place, with all of a note's text.
    pub async fn read_file(&self, caller: &Caller, req: FileRef) -> Result<FileItem, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        inplace::file(&self.catalog, &location, &req.path)?
            .map(item_of)
            .ok_or_else(|| ApiError::not_found(format!("{} has no file {}", req.space, req.path)))
    }

    /// A photo's thumbnail (JPEG), from the index.
    pub async fn file_thumbnail(&self, caller: &Caller, req: FileRef) -> Result<Vec<u8>, ApiError> {
        caller.require_owner()?;
        let location = self.in_place_of(&req.space)?;
        inplace::thumb(&self.catalog, &location, &req.path)?
            .ok_or_else(|| ApiError::not_found("no thumbnail"))
    }

    /// A photo as it is in its folder, or the `range` of it asked for: only a
    /// file the index lists as a photo, read where it is.
    pub async fn file_original(
        &self,
        caller: &Caller,
        req: FileRef,
        range: Option<(u64, Option<u64>)>,
    ) -> Result<Original, ApiError> {
        caller.require_owner()?;
        let location = self.in_place_of(&req.space)?;
        let file = inplace::file(&self.catalog, &location, &req.path)?
            .filter(|f| f.kind == inplace::IMAGE)
            .ok_or_else(|| {
                ApiError::not_found(format!("{} has no photo {}", req.space, req.path))
            })?;
        let content_type =
            image_type(&file.name).ok_or_else(|| ApiError::not_found("not a photo shown"))?;
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
            return Err(ApiError::invalid("ask for a large photo in ranges"));
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
