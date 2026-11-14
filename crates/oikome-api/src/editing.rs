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

//! Safe editing of local Markdown and plain text. The index supplies a
//! preview, but saving always resolves the indexed path again and compares
//! the real content hash. A remembered confirmation is valid only for the
//! same identity, action, scope, file, version and permissions.

use std::io::Write;
use std::path::{Path, PathBuf};

use oikome_catalog::{Location, LocationFormat};
use oikome_node::inplace;
use serde::{Deserialize, Serialize};

use crate::{
    Api, ApiError, Caller, ConfirmationScope, Empty, ErrorCode, FileItem, FileRef,
    RevokeSaveConfirmation, SaveConfirmationAction, SaveTextFile, SavedConfirmation,
    TextSaveTarget,
};

/// JSON bodies are limited to 1 MiB; leave room for escaping and the request.
const MAX_TEXT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SaveConfirmation {
    action: SaveConfirmationAction,
    scope: ConfirmationScope,
    identity: String,
    directory: String,
    path: String,
    version: String,
    permissions: u32,
}

impl SaveConfirmation {
    fn id(&self) -> String {
        let key = format!(
            "{:?}:{:?}:{}:{}:{}",
            self.action, self.scope, self.identity, self.directory, self.path
        );
        blake3::hash(key.as_bytes()).to_hex()[..16].to_string()
    }

    fn summary(&self) -> SavedConfirmation {
        SavedConfirmation {
            id: self.id(),
            action: self.action,
            scope: self.scope,
            directory: self.directory.clone(),
            path: self.path.clone(),
        }
    }

    fn matches(
        &self,
        action: SaveConfirmationAction,
        identity: &str,
        directory: &Path,
        path: &Path,
        version: &str,
        permissions: u32,
    ) -> bool {
        self.action == action
            && self.identity == identity
            && self.directory == directory.to_string_lossy()
            && self.path == path.to_string_lossy()
            && self.version == version
            && self.permissions == permissions
    }
}

struct SourceFile {
    root: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
    version: String,
    permissions: std::fs::Permissions,
}

fn permission_key(permissions: &std::fs::Permissions) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.mode()
    }
    #[cfg(not(unix))]
    {
        u32::from(permissions.readonly())
    }
}

fn content_hash(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn editable_name(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "txt"
    )
}

fn local_root(location: &Location) -> Result<PathBuf, ApiError> {
    let LocationFormat::InPlace {
        place: None, path, ..
    } = &location.format
    else {
        return Err(ApiError::permission_denied(
            "this file's place is read-only in Oikome",
        ));
    };
    let root = std::fs::canonicalize(path)
        .map_err(|e| ApiError::invalid(format!("this folder cannot be reached: {e}")))?;
    if !root.is_dir() {
        return Err(ApiError::invalid("this collection's folder is unavailable"));
    }
    Ok(root)
}

fn existing_file(root: &Path, relative: &str) -> Result<PathBuf, ApiError> {
    if !inplace::is_path_within(relative) {
        return Err(ApiError::invalid(format!(
            "{relative} is not a file in this collection"
        )));
    }
    let candidate = root.join(relative);
    let metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|e| ApiError::invalid(format!("{} cannot be read: {e}", candidate.display())))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ApiError::invalid(format!(
            "{} is not a regular file Oikome can edit",
            candidate.display()
        )));
    }
    let path = std::fs::canonicalize(&candidate)
        .map_err(|e| ApiError::invalid(format!("{} cannot be read: {e}", candidate.display())))?;
    if !path.starts_with(root) {
        return Err(ApiError::permission_denied(
            "the file resolves outside this collection",
        ));
    }
    Ok(path)
}

fn read_source(api: &Api, location: &Location, relative: &str) -> Result<SourceFile, ApiError> {
    if inplace::file(&api.catalog, location, relative)?.is_none() {
        return Err(ApiError::not_found(format!(
            "this collection has no file {relative}"
        )));
    }
    let root = local_root(location)?;
    let path = existing_file(&root, relative)?;
    let bytes = std::fs::read(&path)
        .map_err(|e| ApiError::invalid(format!("{} cannot be read: {e}", path.display())))?;
    let permissions = std::fs::metadata(&path)
        .map_err(ApiError::internal)?
        .permissions();
    let version = content_hash(&bytes);
    Ok(SourceFile {
        root,
        path,
        bytes,
        version,
        permissions,
    })
}

fn conflict(path: &Path, version: &str) -> ApiError {
    ApiError::new(
        ErrorCode::Conflict,
        format!("{} changed after it was opened", path.display()),
    )
    .because("version_conflict")
    .with("path", path.to_string_lossy())
    .with("version", version)
}

fn confirmation_required(action: SaveConfirmationAction, path: &Path, version: &str) -> ApiError {
    let (reason, message) = match action {
        SaveConfirmationAction::SourceSave => (
            "source_confirmation_required",
            "changing the source file needs confirmation",
        ),
        SaveConfirmationAction::Overwrite => (
            "overwrite_confirmation_required",
            "replacing an existing copy needs confirmation",
        ),
    };
    ApiError::new(ErrorCode::Conflict, message)
        .because(reason)
        .with("path", path.to_string_lossy())
        .with("targetVersion", version)
}

fn confirmation_matches(
    api: &Api,
    action: SaveConfirmationAction,
    identity: &str,
    path: &Path,
    version: &str,
    permissions: u32,
) -> Result<bool, ApiError> {
    let directory = path
        .parent()
        .ok_or_else(|| ApiError::invalid("the file has no parent folder"))?;
    Ok(api
        .settings()?
        .save_confirmations
        .iter()
        .any(|saved| saved.matches(action, identity, directory, path, version, permissions)))
}

fn encode_text(source: &[u8], edited: &str) -> Result<Vec<u8>, ApiError> {
    let bom = source.starts_with(&[0xef, 0xbb, 0xbf]);
    let original = std::str::from_utf8(if bom { &source[3..] } else { source })
        .map_err(|_| ApiError::invalid("only UTF-8 text can be edited"))?;
    let ending = if original.contains("\r\n") {
        "\r\n"
    } else if original.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    let normalized = edited.replace("\r\n", "\n").replace('\r', "\n");
    let body = if ending == "\n" {
        normalized
    } else {
        normalized.replace('\n', ending)
    };
    let mut bytes = Vec::with_capacity(body.len() + 3);
    if bom {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(body.as_bytes());
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(ApiError::invalid(
            "editing is limited to 512 KiB of UTF-8 text",
        ));
    }
    Ok(bytes)
}

fn temporary_path(target: &Path) -> PathBuf {
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("text");
    target.with_file_name(format!(
        ".oikome-save-{}-{n}-{name}.tmp",
        std::process::id()
    ))
}

fn replace_file(
    target: &Path,
    bytes: &[u8],
    expected_version: &str,
    permissions: &std::fs::Permissions,
) -> Result<(), ApiError> {
    let tmp = temporary_path(target);
    let write = (|| -> Result<(), ApiError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = options.open(&tmp).map_err(ApiError::internal)?;
        file.write_all(bytes).map_err(ApiError::internal)?;
        file.sync_all().map_err(ApiError::internal)?;
        std::fs::set_permissions(&tmp, permissions.clone()).map_err(ApiError::internal)?;
        let current = std::fs::read(target).map_err(ApiError::internal)?;
        if content_hash(&current) != expected_version {
            return Err(conflict(target, &content_hash(&current)));
        }
        std::fs::rename(&tmp, target).map_err(ApiError::internal)?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write
}

fn create_file(target: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(confirmation_required(
                SaveConfirmationAction::Overwrite,
                target,
                "",
            ));
        }
        Err(e) => return Err(ApiError::internal(e)),
    };
    let write = file.write_all(bytes).and_then(|()| file.sync_all());
    if let Err(e) = write {
        drop(file);
        let _ = std::fs::remove_file(target);
        return Err(ApiError::internal(e));
    }
    Ok(())
}

fn copy_name(request: &SaveTextFile) -> Result<String, ApiError> {
    let name = request.copy_name.as_deref().unwrap_or("").trim();
    if name.is_empty()
        || name.len() > 255
        || matches!(name, "." | "..")
        || name.starts_with('.')
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
        || !editable_name(name)
    {
        return Err(ApiError::invalid(
            "choose a Markdown or plain-text file name without a directory",
        ));
    }
    Ok(name.to_string())
}

/// Check the real local file when it is opened, rather than trusting the index.
pub(crate) fn freshen_file(api: &Api, location: &Location, item: &mut FileItem) {
    item.content.capabilities.edit = false;
    item.content.capabilities.save_copy = false;
    item.content.edit_unavailable_reason = None;
    if !editable_name(&item.name) {
        item.content.edit_unavailable_reason =
            Some("Only Markdown and plain text can be edited".into());
        return;
    }
    if !matches!(location.format, LocationFormat::InPlace { place: None, .. }) {
        item.content.edit_unavailable_reason =
            Some("This place is read-only in Oikome; open the file there to edit it".into());
        return;
    }
    let source = match read_source(api, location, &item.path) {
        Ok(source) => source,
        Err(e) => {
            item.content.edit_unavailable_reason = Some(e.message);
            return;
        }
    };
    item.content.version = source.version;
    if source.bytes.len() > MAX_TEXT_BYTES {
        item.content.edit_unavailable_reason =
            Some("Editing is limited to 512 KiB of UTF-8 text".into());
        return;
    }
    let Ok(text) = std::str::from_utf8(&source.bytes) else {
        item.content.edit_unavailable_reason = Some("Only UTF-8 text can be edited".into());
        return;
    };
    item.text = Some(text.to_string());
    item.content.capabilities.edit = !source.permissions.readonly();
    let copy_possible = source
        .path
        .parent()
        .and_then(|parent| std::fs::metadata(parent).ok())
        .is_some_and(|metadata| !metadata.permissions().readonly());
    item.content.capabilities.save_copy = copy_possible;
    item.content.edit_unavailable_reason = match (source.permissions.readonly(), copy_possible) {
        (true, true) => Some("The original is read-only; save a copy to keep changes".into()),
        (_, false) => {
            Some("The folder is read-only; neither the original nor a copy can be saved".into())
        }
        _ => None,
    };
}

impl Api {
    /// Save local Markdown or plain text, either to its source or to a copy
    /// beside it. Every call checks the source hash and current permissions.
    pub async fn save_text_file(
        &self,
        caller: &Caller,
        req: SaveTextFile,
    ) -> Result<FileItem, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        if !editable_name(&req.path) {
            return Err(ApiError::invalid(
                "only Markdown and plain text can be edited",
            ));
        }
        let source = read_source(self, &location, &req.path)?;
        if source.bytes.len() > MAX_TEXT_BYTES || std::str::from_utf8(&source.bytes).is_err() {
            return Err(ApiError::invalid(
                "editing requires at most 512 KiB of UTF-8 text",
            ));
        }
        if source.version != req.version {
            return Err(conflict(&source.path, &source.version));
        }
        let bytes = encode_text(&source.bytes, &req.text)?;
        let identity = self.node_did()?;
        let parent = source
            .path
            .parent()
            .ok_or_else(|| ApiError::invalid("the source has no parent folder"))?;
        let parent_permissions = std::fs::metadata(parent)
            .map_err(ApiError::internal)?
            .permissions();
        if parent_permissions.readonly() {
            return Err(ApiError::permission_denied(
                "the file's folder is read-only",
            ));
        }

        let (target, target_relative, target_version, target_permissions, action, existed) =
            match req.target {
                TextSaveTarget::Original => {
                    if source.permissions.readonly() {
                        return Err(ApiError::permission_denied(
                            "the original is read-only; save a copy instead",
                        ));
                    }
                    (
                        source.path.clone(),
                        req.path.clone(),
                        source.version.clone(),
                        source.permissions.clone(),
                        SaveConfirmationAction::SourceSave,
                        true,
                    )
                }
                TextSaveTarget::Copy => {
                    let name = copy_name(&req)?;
                    let target = parent.join(&name);
                    if target == source.path {
                        return Err(ApiError::invalid(
                            "choose another name, or save the original explicitly",
                        ));
                    }
                    let relative = Path::new(&req.path)
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .join(&name)
                        .to_string_lossy()
                        .replace('\\', "/");
                    if target.exists() {
                        let relative_file = existing_file(&source.root, &relative)?;
                        if inplace::file(&self.catalog, &location, &relative)?.is_none() {
                            return Err(ApiError::new(
                                ErrorCode::Conflict,
                                "the target is not indexed; refresh the collection before replacing it",
                            ));
                        }
                        let target_bytes =
                            std::fs::read(&relative_file).map_err(ApiError::internal)?;
                        let permissions = std::fs::metadata(&relative_file)
                            .map_err(ApiError::internal)?
                            .permissions();
                        (
                            relative_file,
                            relative,
                            content_hash(&target_bytes),
                            permissions,
                            SaveConfirmationAction::Overwrite,
                            true,
                        )
                    } else {
                        (
                            target,
                            relative,
                            String::new(),
                            source.permissions.clone(),
                            SaveConfirmationAction::Overwrite,
                            false,
                        )
                    }
                }
            };

        if existed {
            if target_permissions.readonly() {
                return Err(ApiError::permission_denied(
                    "the target is read-only; choose another name",
                ));
            }
            let remembered = confirmation_matches(
                self,
                action,
                &identity,
                &target,
                &target_version,
                permission_key(&target_permissions),
            )?;
            if !remembered {
                if !req.confirm {
                    return Err(confirmation_required(action, &target, &target_version));
                }
                if action == SaveConfirmationAction::Overwrite
                    && req.target_version.as_deref() != Some(target_version.as_ref())
                {
                    return Err(conflict(&target, &target_version));
                }
            }
            replace_file(&target, &bytes, &target_version, &target_permissions)?;
        } else {
            create_file(&target, &bytes)?;
        }

        if req.target == TextSaveTarget::Copy && !existed {
            let LocationFormat::InPlace { path, sources, .. } = &location.format else {
                unreachable!("local_root checked the location")
            };
            let mut sources = sources.clone().unwrap_or_else(|| {
                vec![oikome_catalog::InPlaceSource {
                    root: path.clone(),
                    files: None,
                }]
            });
            crate::in_place::add_selected_source(&mut sources, &target)?;
            if let Err(e) = self
                .catalog
                .add_in_place_files(&req.space, path.clone(), sources)
            {
                let _ = std::fs::remove_file(&target);
                return Err(e.into());
            }
        }

        let new_version = content_hash(&bytes);
        let new_permissions = std::fs::metadata(&target)
            .map_err(ApiError::internal)?
            .permissions();
        let new_permission_key = permission_key(&new_permissions);
        let directory = target.parent().unwrap_or(parent);
        self.update_settings(|settings| {
            for saved in &mut settings.save_confirmations {
                if saved.action == action
                    && saved.identity == identity
                    && saved.directory == directory.to_string_lossy()
                    && saved.path == target.to_string_lossy()
                {
                    saved.version = new_version.clone();
                    saved.permissions = new_permission_key;
                }
            }
            if existed {
                if let Some(scope) = req.remember {
                    let saved = SaveConfirmation {
                        action,
                        scope,
                        identity: identity.clone(),
                        directory: directory.to_string_lossy().into_owned(),
                        path: target.to_string_lossy().into_owned(),
                        version: new_version.clone(),
                        permissions: new_permission_key,
                    };
                    settings.save_confirmations.retain(|old| {
                        !(old.action == action
                            && old.scope == scope
                            && old.identity == identity
                            && old.directory == directory.to_string_lossy()
                            && old.path == target.to_string_lossy())
                    });
                    settings.save_confirmations.push(saved);
                }
            }
        })?;
        self.session(None).refresh_in_place(&req.space).await?;
        self.read_file(
            caller,
            FileRef {
                space: req.space,
                path: target_relative,
            },
        )
        .await
    }

    /// The owner's remembered source-save and overwrite confirmations.
    pub async fn list_save_confirmations(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<SavedConfirmation>, ApiError> {
        caller.require_owner()?;
        let mut saved: Vec<_> = self
            .settings()?
            .save_confirmations
            .iter()
            .map(SaveConfirmation::summary)
            .collect();
        saved.sort_by(|a, b| {
            (&a.directory, &a.path, a.action, a.scope).cmp(&(
                &b.directory,
                &b.path,
                b.action,
                b.scope,
            ))
        });
        Ok(saved)
    }

    /// Ask again for one remembered confirmation.
    pub async fn revoke_save_confirmation(
        &self,
        caller: &Caller,
        req: RevokeSaveConfirmation,
    ) -> Result<Vec<SavedConfirmation>, ApiError> {
        caller.require_owner()?;
        let removed = self.update_settings(|settings| {
            let before = settings.save_confirmations.len();
            settings
                .save_confirmations
                .retain(|saved| saved.id() != req.id);
            settings.save_confirmations.len() != before
        })?;
        if !removed {
            return Err(ApiError::not_found(
                "that remembered confirmation was already revoked",
            ));
        }
        self.list_save_confirmations(caller, Empty {}).await
    }
}
