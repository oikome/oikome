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

//! Dashboard plugins (architecture v3 §13.4, M2): cards on Home that others
//! (or Oikome) made. A plugin is a folder of web files with a manifest
//! (`oikome-plugin.json`); installed, its files are kept in
//! `.oikome/plugins/<id>/` and served by the web server under
//! `/plugins/<id>/`, sandboxed and with no network.
//!
//! Each installed plugin is a principal of its own: a DID made for it at
//! install, holding a whole-Space grant on each collection its owner chose.
//! The app reads a collection for a plugin through `plugin_read`, which runs
//! the read in a session *as the plugin*, so the node's own grants decide;
//! each read, and each refusal, goes to the access log.

use std::path::{Path, PathBuf};

use oikome_identity::NodeIdentity;
use oikome_node::{AccessEntry, OikomeSession, Outcome, QueryResult, SessionConfig};
use oikome_semantic::rewrite::quote_ident;
use serde::{Deserialize, Serialize};

use crate::invites::read_private;
use crate::{
    encode_rows, now_ms, Api, ApiError, Caller, Empty, ErrorCode, InstallPlugin, Plugin,
    PluginCollections, PluginColumn, PluginFolder, PluginRead, PluginRef, PluginShape, Rows,
};

/// The bridge every plugin page loads first (`/plugins/sdk.js`).
pub const PLUGIN_SDK: &str = include_str!("../../../packages/plugin-sdk/sdk.js");

const MANIFEST: &str = "oikome-plugin.json";
const PLUGINS_FILE: &str = "plugins.json";
const PLUGINS_DIR: &str = "plugins";
/// A plugin's files: how many, how large in all, how deep.
const MAX_FILES: usize = 200;
const MAX_BYTES: u64 = 8 << 20;
const MAX_DEPTH: usize = 4;
/// The kinds of file a plugin may have, by extension.
const KINDS: [&str; 13] = [
    "html", "js", "css", "json", "svg", "png", "jpg", "jpeg", "webp", "gif", "woff2", "txt", "md",
];
const MAX_INSTALLED: usize = 50;
/// What one read gives a plugin: rows, and bytes (photos are large).
const MAX_ROWS: u32 = 1000;
const MAX_READ_BYTES: usize = 32 << 20;
pub(crate) const READ_METHOD: &str = "plugin_read";
/// A plugin's reads of one collection go to Activity at most this often;
/// refusals, always.
const READ_LOG_EVERY_MS: u64 = 10 * 60 * 1000;

/// Oikome's own plugins, in this repository's `plugins/`.
const BUILT_IN: [&[(&str, &[u8])]; 2] = [
    &[
        (
            MANIFEST,
            include_bytes!("../../../plugins/spending-by-category/oikome-plugin.json"),
        ),
        (
            "index.html",
            include_bytes!("../../../plugins/spending-by-category/index.html"),
        ),
        (
            "main.js",
            include_bytes!("../../../plugins/spending-by-category/main.js"),
        ),
        (
            "style.css",
            include_bytes!("../../../plugins/spending-by-category/style.css"),
        ),
    ],
    &[
        (
            MANIFEST,
            include_bytes!("../../../plugins/photo-memories/oikome-plugin.json"),
        ),
        (
            "index.html",
            include_bytes!("../../../plugins/photo-memories/index.html"),
        ),
        (
            "main.js",
            include_bytes!("../../../plugins/photo-memories/main.js"),
        ),
        (
            "style.css",
            include_bytes!("../../../plugins/photo-memories/style.css"),
        ),
    ],
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    name: String,
    by: String,
    version: String,
    #[serde(default)]
    description: String,
    reads: ManifestReads,
    #[serde(default)]
    card: ManifestCard,
    #[serde(default = "index_html")]
    entry: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestReads {
    what: String,
    shape: PluginShape,
    #[serde(default)]
    columns: Vec<PluginColumn>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestCard {
    width: Option<u8>,
    height: Option<u32>,
}

fn index_html() -> String {
    "index.html".to_string()
}

/// A plugin's files: each one's path in its folder, and its bytes.
type Files = Vec<(String, Vec<u8>)>;

/// An installed plugin, as kept in `.oikome/plugins.json`.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Installed {
    id: String,
    /// The principal it reads as.
    did: String,
    name: String,
    by: String,
    version: String,
    description: String,
    reads: String,
    shape: PluginShape,
    #[serde(default)]
    columns: Vec<PluginColumn>,
    width: u8,
    height: u32,
    entry: String,
    built_in: bool,
    collections: Vec<String>,
    installed_at_ms: u64,
}

impl Installed {
    fn plugin(&self) -> Plugin {
        Plugin {
            id: self.id.clone(),
            name: self.name.clone(),
            by: self.by.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            reads: self.reads.clone(),
            shape: self.shape,
            columns: self.columns.clone(),
            width: self.width,
            height: self.height,
            entry: self.entry.clone(),
            built_in: self.built_in,
            installed: true,
            collections: self.collections.clone(),
        }
    }
}

/// Text from a manifest: trimmed, one line, within `max` characters.
fn field(value: &str, what: &str, max: usize, required: bool) -> Result<String, ApiError> {
    let value = value.trim();
    if (required && value.is_empty())
        || value.chars().count() > max
        || value.chars().any(char::is_control)
    {
        return Err(ApiError::invalid(format!(
            "the plugin's {what} must be {}up to {max} characters on one line",
            if required { "1 and " } else { "" }
        )));
    }
    Ok(value.to_string())
}

fn valid_id(id: &str) -> bool {
    id.len() <= 40
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A path inside a plugin: plain names (letters, digits, `.`, `_`, `-`), not
/// hidden, not too deep, of a kind a plugin may have. Nothing that could
/// leave its folder.
pub(crate) fn safe_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    let plain = |s: &&str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    let kind = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| KINDS.contains(&e.to_ascii_lowercase().as_str()));
    !path.is_empty()
        && path.len() <= 200
        && segments.len() <= MAX_DEPTH
        && segments.iter().all(plain)
        && kind
}

/// A manifest's contents, checked, as a plugin not yet installed.
fn read_manifest(bytes: &[u8], built_in: bool) -> Result<Plugin, ApiError> {
    let m: Manifest = serde_json::from_slice(bytes)
        .map_err(|e| ApiError::invalid(format!("{MANIFEST} can't be read: {e}")))?;
    if !valid_id(&m.id) {
        return Err(ApiError::invalid(
            "the plugin's id must be lowercase letters, digits and -, starting with a letter, up to 40",
        ));
    }
    let width = m.card.width.unwrap_or(4);
    let height = m.card.height.unwrap_or(240);
    if !(4..=12).contains(&width) || !(120..=600).contains(&height) {
        return Err(ApiError::invalid(
            "a plugin's card is 4 to 12 columns wide and 120 to 600 pixels high",
        ));
    }
    if !safe_path(&m.entry) || !m.entry.ends_with(".html") {
        return Err(ApiError::invalid(
            "the plugin's entry must be an .html file in its folder",
        ));
    }
    let version = field(&m.version, "version", 20, true)?;
    if !version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return Err(ApiError::invalid("the plugin's version must be like 1.2.0"));
    }
    Ok(Plugin {
        name: field(&m.name, "name", 60, true)?,
        by: field(&m.by, "maker", 60, true)?,
        version,
        description: field(&m.description, "description", 300, false)?,
        reads: field(&m.reads.what, "description of what it reads", 200, true)?,
        shape: m.reads.shape,
        columns: m.reads.columns.into_iter().take(8).collect(),
        width,
        height,
        entry: m.entry,
        built_in,
        installed: false,
        collections: Vec::new(),
        id: m.id,
    })
}

/// The files of a plugin folder on this computer, checked: no links, no
/// hidden files, only kinds a plugin may have, within the limits.
fn read_folder(root: &Path) -> Result<Files, ApiError> {
    fn walk(
        root: &Path,
        dir: &Path,
        depth: usize,
        out: &mut Files,
        total: &mut u64,
    ) -> Result<(), ApiError> {
        let entries = std::fs::read_dir(dir).map_err(ApiError::internal)?;
        for entry in entries {
            let entry = entry.map_err(ApiError::internal)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let meta = entry
                .path()
                .symlink_metadata()
                .map_err(ApiError::internal)?;
            let rel = entry
                .path()
                .strip_prefix(root)
                .map_err(ApiError::internal)?
                .to_string_lossy()
                .replace('\\', "/");
            if meta.file_type().is_symlink() {
                return Err(ApiError::invalid(format!(
                    "{rel} is a link; a plugin's folder holds its files themselves"
                )));
            }
            if meta.is_dir() {
                if depth + 1 >= MAX_DEPTH {
                    return Err(ApiError::invalid("the plugin's folders go too deep"));
                }
                walk(root, &entry.path(), depth + 1, out, total)?;
                continue;
            }
            if !safe_path(&rel) {
                return Err(ApiError::invalid(format!(
                    "{rel} isn't a file a plugin may have (names of letters, digits, . _ -; \
                     {} files)",
                    KINDS.join(" ")
                )));
            }
            *total += meta.len();
            if out.len() >= MAX_FILES || *total > MAX_BYTES {
                return Err(ApiError::invalid(format!(
                    "a plugin has up to {MAX_FILES} files and {} MB in all",
                    MAX_BYTES >> 20
                )));
            }
            out.push((
                rel,
                std::fs::read(entry.path()).map_err(ApiError::internal)?,
            ));
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, 0, &mut out, &mut 0)?;
    Ok(out)
}

/// The plugin page `html` with the bridge as its first script.
fn with_sdk(html: &str) -> String {
    const TAG: &str = "<script src=\"/plugins/sdk.js\"></script>";
    let lower = html.to_ascii_lowercase();
    let at = ["<head", "<html"]
        .iter()
        .find_map(|tag| {
            let start = lower.find(tag)?;
            Some(start + lower[start..].find('>')? + 1)
        })
        .unwrap_or(0);
    format!("{}{TAG}{}", &html[..at], &html[at..])
}

impl Api {
    fn plugins_path(&self) -> PathBuf {
        self.catalog.oikome_dir().join(PLUGINS_FILE)
    }

    fn plugins_dir(&self) -> PathBuf {
        self.catalog.oikome_dir().join(PLUGINS_DIR)
    }

    fn installed(&self) -> Result<Vec<Installed>, ApiError> {
        read_private(&self.plugins_path())
    }

    fn installed_plugin(&self, id: &str) -> Result<Installed, ApiError> {
        self.installed()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| ApiError::not_found(format!("there is no plugin {id}")))
    }

    fn built_in(id: &str) -> Option<&'static [(&'static str, &'static [u8])]> {
        BUILT_IN.iter().copied().find(|files| {
            files
                .iter()
                .find(|(name, _)| *name == MANIFEST)
                .and_then(|(_, bytes)| read_manifest(bytes, true).ok())
                .is_some_and(|p| p.id == id)
        })
    }

    /// Plugins installed here, then Oikome's own not added yet.
    pub async fn list_plugins(&self, caller: &Caller, _: Empty) -> Result<Vec<Plugin>, ApiError> {
        caller.require_owner()?;
        let installed = self.installed()?;
        let mut out: Vec<Plugin> = installed.iter().map(Installed::plugin).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        for files in BUILT_IN {
            let Some((_, bytes)) = files.iter().find(|(name, _)| *name == MANIFEST) else {
                continue;
            };
            let plugin = read_manifest(bytes, true)?;
            if !installed.iter().any(|p| p.id == plugin.id) {
                out.push(plugin);
            }
        }
        Ok(out)
    }

    /// What a folder on this computer holds as a plugin, before adding it.
    pub async fn inspect_plugin(
        &self,
        caller: &Caller,
        req: PluginFolder,
    ) -> Result<Plugin, ApiError> {
        caller.require_owner()?;
        let (plugin, _) = self.plugin_in_folder(&req.path)?;
        let mut plugin = plugin;
        if let Ok(installed) = self.installed_plugin(&plugin.id) {
            plugin.installed = true;
            plugin.collections = installed.collections;
        }
        Ok(plugin)
    }

    fn plugin_in_folder(&self, path: &str) -> Result<(Plugin, Files), ApiError> {
        let given = Path::new(path.trim());
        if !given.is_absolute() {
            return Err(ApiError::invalid("choose a folder on this computer"));
        }
        let root = given
            .canonicalize()
            .map_err(|_| ApiError::invalid(format!("{} is not there", given.display())))?;
        if !root.is_dir() {
            return Err(ApiError::invalid(format!(
                "{} is not a folder",
                root.display()
            )));
        }
        if !root.join(MANIFEST).is_file() {
            return Err(ApiError::invalid(format!(
                "{} has no {MANIFEST}, so it isn't a plugin",
                root.display()
            )));
        }
        let files = read_folder(&root)?;
        let manifest = files
            .iter()
            .find(|(name, _)| name == MANIFEST)
            .map(|(_, bytes)| bytes.as_slice())
            .unwrap_or_default();
        let plugin = read_manifest(manifest, false)?;
        if !files.iter().any(|(name, _)| *name == plugin.entry) {
            return Err(ApiError::invalid(format!(
                "the plugin's entry, {}, isn't in its folder",
                plugin.entry
            )));
        }
        Ok((plugin, files))
    }

    /// Add one of Oikome's plugins, or one from a folder, letting it read
    /// these collections. Adding a plugin already here updates its files and
    /// what it may read.
    pub async fn install_plugin(
        &self,
        caller: &Caller,
        req: InstallPlugin,
    ) -> Result<Plugin, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (plugin, files) = match (&req.built_in, &req.path) {
            (Some(id), None) => {
                let files = Self::built_in(id)
                    .ok_or_else(|| ApiError::not_found(format!("Oikome has no plugin {id}")))?;
                let manifest = files
                    .iter()
                    .find(|(name, _)| *name == MANIFEST)
                    .map(|(_, bytes)| *bytes)
                    .unwrap_or_default();
                let files = files
                    .iter()
                    .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
                    .collect();
                (read_manifest(manifest, true)?, files)
            }
            (None, Some(path)) => self.plugin_in_folder(path)?,
            _ => {
                return Err(ApiError::invalid(
                    "add one of Oikome's plugins or a plugin's folder",
                ))
            }
        };
        let collections = self.readable_by_plugins(&req.collections)?;
        let existing = self.installed_plugin(&plugin.id).ok();
        if existing.is_none() && self.installed()?.len() >= MAX_INSTALLED {
            return Err(ApiError::invalid(format!(
                "up to {MAX_INSTALLED} plugins can be added; remove one first"
            )));
        }
        self.put_files(&plugin.id, files)?;
        let did = match &existing {
            Some(p) => p.did.clone(),
            None => NodeIdentity::generate()
                .map_err(ApiError::internal)?
                .did()
                .to_string(),
        };
        let installed = Installed {
            id: plugin.id.clone(),
            did,
            name: plugin.name,
            by: plugin.by,
            version: plugin.version,
            description: plugin.description,
            reads: plugin.reads,
            shape: plugin.shape,
            columns: plugin.columns,
            width: plugin.width,
            height: plugin.height,
            entry: plugin.entry,
            built_in: plugin.built_in,
            collections: existing.map(|p| p.collections).unwrap_or_default(),
            installed_at_ms: now_ms(),
        };
        let installed = self.grant_plugin(installed, collections)?;
        Ok(installed.plugin())
    }

    /// Change what an installed plugin may read.
    pub async fn set_plugin_collections(
        &self,
        caller: &Caller,
        req: PluginCollections,
    ) -> Result<Plugin, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let installed = self.installed_plugin(&req.id)?;
        let collections = self.readable_by_plugins(&req.collections)?;
        Ok(self.grant_plugin(installed, collections)?.plugin())
    }

    /// Remove a plugin: its grants end and its files go.
    pub async fn remove_plugin(
        &self,
        caller: &Caller,
        req: PluginRef,
    ) -> Result<Vec<Plugin>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let installed = self.installed_plugin(&req.id)?;
        let session = self.session(None);
        for space in &installed.collections {
            if self.catalog.get_space(space).is_ok() {
                session.revoke_space(space, std::slice::from_ref(&installed.did))?;
            }
        }
        self.change_private(&self.plugins_path(), |all: &mut Vec<Installed>| {
            all.retain(|p| p.id != req.id)
        })?;
        self.remember_name(&installed.did, &installed.name);
        let dir = self.plugins_dir().join(&req.id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(ApiError::internal)?;
        }
        self.list_plugins(caller, Empty {}).await
    }

    /// The rows of a collection, read as the plugin: refused (and logged)
    /// unless the plugin may read it.
    pub async fn plugin_read(&self, caller: &Caller, req: PluginRead) -> Result<Rows, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let plugin = self.installed_plugin(&req.plugin)?;
        let log = |outcome: Outcome, reason: Option<String>| {
            if let Some(log) = &self.access_log {
                log.record(AccessEntry {
                    at_ms: now_ms(),
                    caller: Some(plugin.did.clone()),
                    method: READ_METHOD.to_string(),
                    spaces: vec![req.space.clone()],
                    token_id: None,
                    outcome,
                    reason,
                    detail: None,
                    merge: false,
                });
            }
        };
        if !plugin.collections.contains(&req.space) {
            log(
                Outcome::Denied,
                Some("not one of its collections".to_string()),
            );
            return Err(ApiError::permission_denied(format!(
                "{} may not read {}",
                plugin.name, req.space
            )));
        }
        // As the plugin: the node's grants decide what it reads.
        let session = OikomeSession::new(
            self.catalog.clone(),
            self.engines.clone(),
            SessionConfig {
                default_space: None,
                principal: Some(plugin.did.clone()),
            },
        );
        let read = async {
            let relations = session.readable_relations(&req.space).await?;
            let Some(table) = relations.first() else {
                return Ok(None);
            };
            let limit = req.limit.unwrap_or(MAX_ROWS).clamp(1, MAX_ROWS);
            let sql = format!(
                "SELECT * FROM {}.{} LIMIT {limit}",
                quote_ident(&req.space),
                quote_ident(table)
            );
            match session.sql(&sql).await? {
                QueryResult::Batches { schema, batches } => Ok(Some(encode_rows(
                    &schema,
                    &batches,
                    limit as usize,
                    MAX_READ_BYTES,
                )?)),
                _ => Err(ApiError::internal("a query returned no rows")),
            }
        };
        // Home reads its cards each time it is looked at; once in a while is
        // what Activity needs to say.
        let fresh = || {
            let now = now_ms();
            let mut last = self
                .plugin_reads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let key = (plugin.did.clone(), req.space.clone());
            let due = last
                .get(&key)
                .is_none_or(|at| now.saturating_sub(*at) >= READ_LOG_EVERY_MS);
            if due {
                last.insert(key, now);
            }
            due
        };
        match read.await {
            Ok(Some(rows)) => {
                if fresh() {
                    log(Outcome::Allowed, None);
                }
                Ok(rows)
            }
            Ok(None) => {
                log(Outcome::Allowed, None);
                Err(ApiError::not_found(format!(
                    "{} holds no table yet",
                    req.space
                )))
            }
            Err(e) => {
                let e: ApiError = e;
                let outcome = if e.code == ErrorCode::PermissionDenied {
                    Outcome::Denied
                } else {
                    Outcome::Failed
                };
                log(outcome, Some(e.message.clone()));
                Err(e)
            }
        }
    }

    /// A file of an installed plugin, for the web server: its bytes, with
    /// the bridge put first in the plugin's page. `None` for anything else.
    pub fn plugin_file(&self, id: &str, path: &str) -> Option<Vec<u8>> {
        if !valid_id(id) || !safe_path(path) {
            return None;
        }
        let plugin = self.installed_plugin(id).ok()?;
        let dir = self.plugins_dir().join(id);
        let file = dir.join(path).canonicalize().ok()?;
        if !file.starts_with(dir.canonicalize().ok()?) || !file.is_file() {
            return None;
        }
        let bytes = std::fs::read(&file).ok()?;
        if path == plugin.entry {
            return Some(with_sdk(&String::from_utf8_lossy(&bytes)).into_bytes());
        }
        Some(bytes)
    }

    /// Plugin DIDs and names, for the access log.
    pub(crate) fn plugin_names(&self) -> Vec<(String, String)> {
        self.installed()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.did, p.name))
            .collect()
    }

    /// The collections given, checked: each one a collection of this data
    /// center's own (a plugin can't read one shared with you).
    fn readable_by_plugins(&self, names: &[String]) -> Result<Vec<String>, ApiError> {
        let mut out: Vec<String> = Vec::new();
        for name in names {
            let space = self.catalog.get_space(name)?;
            if space.kind == oikome_core::SpaceKind::Remote {
                return Err(ApiError::invalid(format!(
                    "{name} is shared with you; a plugin reads only your own collections"
                )));
            }
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        Ok(out)
    }

    /// Give `plugin` grants on exactly `collections`, and keep it.
    fn grant_plugin(
        &self,
        mut plugin: Installed,
        collections: Vec<String>,
    ) -> Result<Installed, ApiError> {
        let session = self.session(None);
        let did = std::slice::from_ref(&plugin.did);
        for space in &plugin.collections {
            if !collections.contains(space) && self.catalog.get_space(space).is_ok() {
                session.revoke_space(space, did)?;
            }
        }
        for space in &collections {
            if !plugin.collections.contains(space)
                || !self.catalog.has_space_grant(&plugin.did, space)
            {
                session.grant_space(space, did)?;
            }
        }
        plugin.collections = collections;
        let kept = plugin.clone();
        self.change_private(&self.plugins_path(), |all: &mut Vec<Installed>| {
            all.retain(|p| p.id != kept.id);
            all.push(kept);
        })?;
        Ok(plugin)
    }

    /// Put a plugin's files in place at once: written beside, then swapped.
    fn put_files(&self, id: &str, files: Files) -> Result<(), ApiError> {
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = self.plugins_dir();
        let tmp = dir.join(format!(".{id}.{}-{n}.new", std::process::id()));
        let old = dir.join(format!(".{id}.{}-{n}.old", std::process::id()));
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&tmp)?;
            for (name, bytes) in &files {
                let path = tmp.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, bytes)?;
            }
            let target = dir.join(id);
            if target.exists() {
                std::fs::rename(&target, &old)?;
            }
            std::fs::rename(&tmp, &target)?;
            if old.exists() {
                std::fs::remove_dir_all(&old)?;
            }
            Ok(())
        };
        write().map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp);
            ApiError::internal(e)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_paths_stay_in_their_folder() {
        assert!(safe_path("index.html"));
        assert!(safe_path("img/a-b_c.PNG"));
        for bad in [
            "",
            "../x.js",
            "a/../b.js",
            ".hidden.js",
            "a//b.js",
            "x.exe",
            "a\\b.js",
            "/x.js",
            "a/b/c/d/e.js",
        ] {
            assert!(!safe_path(bad), "{bad}");
        }
    }

    #[test]
    fn the_bridge_comes_first() {
        let html =
            with_sdk("<!doctype html><HTML lang=en><Head><script src=x.js></script></head></html>");
        let sdk = html.find("/plugins/sdk.js").unwrap();
        assert!(sdk < html.find("x.js").unwrap());
        assert!(
            html.starts_with("<!doctype html><HTML lang=en><Head><script src=\"/plugins/sdk.js\">")
        );
        assert!(with_sdk("<p>no head</p>").starts_with("<script src=\"/plugins/sdk.js\"></script>"));
    }

    #[test]
    fn oikomes_own_plugins_read() {
        for files in BUILT_IN {
            let (_, manifest) = files.iter().find(|(n, _)| *n == MANIFEST).unwrap();
            let plugin = read_manifest(manifest, true).unwrap();
            assert!(
                files.iter().any(|(n, _)| *n == plugin.entry),
                "{}",
                plugin.id
            );
        }
    }
}
