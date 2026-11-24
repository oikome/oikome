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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oikome_identity::NodeIdentity;
use oikome_node::{AccessEntry, OikomeSession, Outcome, QueryResult, SessionConfig};
use oikome_semantic::rewrite::quote_ident;
use serde::{Deserialize, Serialize};

use crate::invites::read_private;
use crate::{
    encode_rows, now_ms, Api, ApiError, Caller, Empty, ErrorCode, InstallPlugin,
    McpClientCapability, McpClientConfig, McpConfigFormat, Plugin, PluginCapabilities,
    PluginCapabilityText, PluginCardCapability, PluginCollections, PluginColumn, PluginFolder,
    PluginRead, PluginRef, PluginShape, PluginText, Rows, SourceKind,
};

/// The bridge every plugin page loads first (`/plugins/sdk.js`).
pub const PLUGIN_SDK: &str = include_str!("../../../packages/plugin-sdk/sdk.js");

const MANIFEST: &str = "oikome-plugin.json";
const PLUGINS_FILE: &str = "plugins.json";
const REMOVED_BUILT_INS_FILE: &str = "removed-built-in-plugins.json";
const PLUGINS_DIR: &str = "plugins";
/// A plugin's files: how many, how large in all, how deep.
const MAX_FILES: usize = 200;
const MAX_BYTES: u64 = 8 << 20;
const MAX_DEPTH: usize = 4;
/// The kinds of file a plugin may have, by extension.
const KINDS: [&str; 14] = [
    "html", "js", "css", "json", "svg", "png", "jpg", "jpeg", "webp", "gif", "woff2", "txt", "md",
    // A preview's code, run in the sandbox only.
    "wasm",
];
const MAX_INSTALLED: usize = 50;
/// What one read gives a plugin: rows, and bytes (photos are large).
const MAX_ROWS: u32 = 1000;
const MAX_READ_BYTES: usize = 32 << 20;
pub(crate) const READ_METHOD: &str = "plugin_read";
/// A plugin's reads of one collection go to Activity at most this often;
/// refusals, always.
const READ_LOG_EVERY_MS: u64 = 10 * 60 * 1000;

/// Official plugin packages, embedded as files and installed through exactly
/// the same manifest parser and lifecycle as packages from a folder.
const BUILT_IN: [&[(&str, &[u8])]; 8] = [
    &[(
        MANIFEST,
        include_bytes!("../../../plugins/file-import/oikome-plugin.json"),
    )],
    &[(
        MANIFEST,
        include_bytes!("../../../plugins/rustfs/oikome-plugin.json"),
    )],
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
    &[
        (
            MANIFEST,
            include_bytes!("../../../plugins/flomo/oikome-plugin.json"),
        ),
        (
            "index.html",
            include_bytes!("../../../plugins/flomo/index.html"),
        ),
        ("main.js", include_bytes!("../../../plugins/flomo/main.js")),
        (
            "style.css",
            include_bytes!("../../../plugins/flomo/style.css"),
        ),
    ],
    &[(
        MANIFEST,
        include_bytes!("../../../plugins/claude-code/oikome-plugin.json"),
    )],
    &[(
        MANIFEST,
        include_bytes!("../../../plugins/kimi-code/oikome-plugin.json"),
    )],
    &[(
        MANIFEST,
        include_bytes!("../../../plugins/codex/oikome-plugin.json"),
    )],
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
    /// PR 23 and earlier: one card at the top level.
    #[serde(default)]
    reads: Option<ManifestReads>,
    #[serde(default)]
    card: ManifestCard,
    #[serde(default = "index_html")]
    entry: String,
    #[serde(default)]
    capabilities: PluginCapabilities,
    /// Its name and descriptions in other languages (M8).
    #[serde(default)]
    translations: BTreeMap<String, ManifestText>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestText {
    name: Option<String>,
    description: Option<String>,
    reads: Option<String>,
    #[serde(default)]
    cards: BTreeMap<String, ManifestText>,
    #[serde(default)]
    sources: BTreeMap<String, ManifestText>,
    #[serde(default)]
    mcp_clients: BTreeMap<String, ManifestText>,
}

/// Languages a manifest may name: about as many as the apps have.
const MAX_LANGUAGES: usize = 40;

/// A language tag as BCP 47 writes them: `de`, `pt-BR`, `zh-Hans`.
fn valid_language(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 20
        && tag.split('-').all(|part| {
            !part.is_empty() && part.len() <= 8 && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
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
pub(crate) type Files = Vec<(String, Vec<u8>)>;

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
    /// Collections removed while this plugin actually held a grant. Only
    /// these can be restored if the owner later brings back that name.
    #[serde(default)]
    restore_collections: Vec<String>,
    installed_at_ms: u64,
    #[serde(default)]
    capabilities: Option<PluginCapabilities>,
    /// Records from before independent card placement default to their old
    /// behavior only when Home has never been arranged.
    #[serde(default = "legacy_home_by_default")]
    home_by_default: bool,
    /// Its words in other languages (M8); none for one added before.
    #[serde(default)]
    translations: BTreeMap<String, PluginText>,
}

fn legacy_home_by_default() -> bool {
    true
}

impl Installed {
    fn capabilities(&self) -> PluginCapabilities {
        self.capabilities
            .clone()
            .unwrap_or_else(|| PluginCapabilities {
                cards: vec![PluginCardCapability {
                    id: "card".into(),
                    name: self.name.clone(),
                    reads: self.reads.clone(),
                    shape: self.shape,
                    columns: self.columns.clone(),
                    width: self.width,
                    height: self.height,
                    entry: self.entry.clone(),
                }],
                ..PluginCapabilities::default()
            })
    }

    pub(crate) fn plugin(&self) -> Plugin {
        let capabilities = self.capabilities();
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
            capabilities,
            home_by_default: self.home_by_default,
            // Official packages take updated translations from their manifest.
            translations: self
                .built_in
                .then(|| Api::built_in_manifest(&self.id))
                .flatten()
                .map(|plugin| plugin.translations)
                .unwrap_or_else(|| self.translations.clone()),
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
        return Err(unfit(format!(
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

fn config_name(value: &str, what: &str) -> Result<(), ApiError> {
    if value.len() <= 40
        && value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Ok(())
    } else {
        Err(unfit(format!(
            "an MCP client's {what} must be a short ASCII configuration name"
        )))
    }
}

fn config_file_part(value: &str, what: &str) -> Result<(), ApiError> {
    if !value.is_empty()
        && value.len() <= 80
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        Ok(())
    } else {
        Err(unfit(format!(
            "an MCP client's {what} must be one plain file or directory name"
        )))
    }
}

fn validate_client_config(config: &McpClientConfig) -> Result<(), ApiError> {
    if config.home_env.len() > 80
        || !config.home_env.ends_with("_HOME")
        || !config
            .home_env
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(unfit(
            "an MCP client's homeEnv must be a client-specific variable ending in _HOME",
        ));
    }
    config_file_part(&config.directory, "directory")?;
    config_file_part(&config.file, "file")?;
    config_name(&config.root, "root")?;
    config_name(&config.headers, "headers")?;
    match config.format {
        McpConfigFormat::Json if config.file.ends_with(".json") => {}
        McpConfigFormat::Toml if config.file.ends_with(".toml") => {}
        McpConfigFormat::Json => return Err(unfit("a JSON MCP config must name a .json file")),
        McpConfigFormat::Toml => return Err(unfit("a TOML MCP config must name a .toml file")),
    }
    Ok(())
}

/// A manifest's contents, checked, as a plugin not yet installed.
pub(crate) fn read_manifest(bytes: &[u8], built_in: bool) -> Result<Plugin, ApiError> {
    let m: Manifest = serde_json::from_slice(bytes)
        .map_err(|e| unfit(format!("{MANIFEST} can't be read: {e}")))?;
    if !valid_id(&m.id) {
        return Err(unfit(
            "the plugin's id must be lowercase letters, digits and -, starting with a letter, up to 40",
        ));
    }
    let name = field(&m.name, "name", 60, true)?;
    let mut capabilities = m.capabilities;
    if capabilities.cards.is_empty() {
        if let Some(reads) = m.reads {
            capabilities.cards.push(PluginCardCapability {
                id: "card".into(),
                name: name.clone(),
                reads: reads.what,
                shape: reads.shape,
                columns: reads.columns,
                width: m.card.width.unwrap_or(4),
                height: m.card.height.unwrap_or(240),
                entry: m.entry,
            });
        }
    }
    if capabilities.cards.len()
        + capabilities.sources.len()
        + capabilities.preview.len()
        + capabilities.mcp_clients.len()
        == 0
    {
        return Err(unfit("a plugin must declare at least one capability"));
    }
    for (kind, count) in [
        ("cards", capabilities.cards.len()),
        ("sources", capabilities.sources.len()),
        ("preview", capabilities.preview.len()),
        ("mcpClients", capabilities.mcp_clients.len()),
    ] {
        if count > 16 {
            return Err(unfit(format!("a plugin declares up to 16 {kind}")));
        }
    }
    let mut card_ids = std::collections::BTreeSet::new();
    for card in &mut capabilities.cards {
        if !valid_id(&card.id) || !card_ids.insert(card.id.clone()) {
            return Err(unfit(
                "a card's id must be unique lowercase letters, digits and -",
            ));
        }
        card.name = field(&card.name, "card name", 60, true)?;
        card.reads = field(&card.reads, "description of what it reads", 200, true)?;
        card.columns = card.columns.iter().copied().take(8).collect();
        if !(4..=12).contains(&card.width) || !(120..=600).contains(&card.height) {
            return Err(unfit(
                "a plugin's card is 4 to 12 columns wide and 120 to 600 pixels high",
            ));
        }
        if !safe_path(&card.entry) || !card.entry.ends_with(".html") {
            return Err(unfit(
                "the plugin's entry must be an .html file in its folder",
            ));
        }
    }
    let mut source_ids = std::collections::BTreeSet::new();
    for source in &mut capabilities.sources {
        if !valid_id(&source.id) || !source_ids.insert(source.id.clone()) {
            return Err(unfit(
                "a source's id must be unique lowercase letters, digits and -",
            ));
        }
        source.name = field(&source.name, "source name", 60, true)?;
    }
    let mut preview_ids = std::collections::BTreeSet::new();
    for preview in &mut capabilities.preview {
        if !valid_id(&preview.id)
            || !preview_ids.insert(preview.id.clone())
            || preview.formats.is_empty()
            || preview.formats.len() > 32
        {
            return Err(unfit("a preview needs a unique id and 1 to 32 formats"));
        }
        preview.name = field(&preview.name, "preview name", 60, true)?;
        if preview.formats.iter().any(|format| {
            format.is_empty()
                || format.len() > 32
                || !format
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        }) {
            return Err(unfit(
                "preview formats must be lowercase extensions without a dot",
            ));
        }
        if let Some(entry) = &preview.entry {
            if !safe_path(entry) || !entry.ends_with(".wasm") {
                return Err(unfit("a preview's entry is a .wasm file of the package"));
            }
        }
    }
    let mut client_ids = std::collections::BTreeSet::new();
    for client in &mut capabilities.mcp_clients {
        if !valid_id(&client.id) || !client_ids.insert(client.id.clone()) {
            return Err(unfit(
                "an MCP client's id must be unique lowercase letters, digits and -",
            ));
        }
        client.name = field(&client.name, "MCP client name", 60, true)?;
        match (&mut client.command, &client.config) {
            (Some(command), _) => {
                *command = field(command, "MCP command", 500, true)?;
                if !command.contains("{url}") || !command.contains("{token}") {
                    return Err(unfit("an MCP command needs {url} and {token} placeholders"));
                }
            }
            (None, Some(config)) => validate_client_config(config)?,
            (None, None) => {
                return Err(unfit("an MCP client needs a command or a configuration"));
            }
        }
    }
    if m.translations.len() > MAX_LANGUAGES {
        return Err(unfit(format!(
            "a plugin's translations name up to {MAX_LANGUAGES} languages"
        )));
    }
    let mut translations = BTreeMap::new();
    for (tag, text) in m.translations {
        if !valid_language(&tag) {
            return Err(unfit(format!(
                "{tag} isn't a language tag, such as de or zh-Hans"
            )));
        }
        let optional = |value: Option<String>, what: &str, max: usize| {
            value.map(|v| field(&v, what, max, true)).transpose()
        };
        let capabilities = |values: BTreeMap<String, ManifestText>| {
            values
                .into_iter()
                .map(|(id, text)| {
                    Ok((
                        id,
                        PluginCapabilityText {
                            name: optional(text.name, "capability name", 60)?,
                            reads: optional(text.reads, "description of what it reads", 200)?,
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, ApiError>>()
        };
        translations.insert(
            tag,
            PluginText {
                name: optional(text.name, "name", 60)?,
                description: optional(text.description, "description", 300)?,
                reads: optional(text.reads, "description of what it reads", 200)?,
                cards: capabilities(text.cards)?,
                sources: capabilities(text.sources)?,
                mcp_clients: capabilities(text.mcp_clients)?,
            },
        );
    }
    let version = field(&m.version, "version", 20, true)?;
    if !version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return Err(unfit("the plugin's version must be like 1.2.0"));
    }
    let first = capabilities.cards.first().cloned();
    Ok(Plugin {
        name,
        by: field(&m.by, "maker", 60, true)?,
        version,
        description: field(&m.description, "description", 300, false)?,
        reads: first.as_ref().map(|c| c.reads.clone()).unwrap_or_default(),
        shape: first.as_ref().map(|c| c.shape).unwrap_or(PluginShape::Any),
        columns: first
            .as_ref()
            .map(|c| c.columns.clone())
            .unwrap_or_default(),
        width: first.as_ref().map(|c| c.width).unwrap_or(4),
        height: first.as_ref().map(|c| c.height).unwrap_or(240),
        entry: first.map(|c| c.entry).unwrap_or_default(),
        built_in,
        installed: false,
        collections: Vec::new(),
        capabilities,
        home_by_default: false,
        translations,
        id: m.id,
    })
}

/// A plugin that can't be added as it is: the detail is for its maker, in
/// English, like a SQL error.
fn unfit(detail: impl Into<String>) -> ApiError {
    let detail = detail.into();
    ApiError::invalid(detail.clone())
        .because("plugin_invalid")
        .with("detail", detail)
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
                return Err(unfit(format!(
                    "{rel} is a link; a plugin's folder holds its files themselves"
                )));
            }
            if meta.is_dir() {
                if depth + 1 >= MAX_DEPTH {
                    return Err(unfit("the plugin's folders go too deep"));
                }
                walk(root, &entry.path(), depth + 1, out, total)?;
                continue;
            }
            if !safe_path(&rel) {
                return Err(unfit(format!(
                    "{rel} isn't a file a plugin may have (names of letters, digits, . _ -; \
                     {} files)",
                    KINDS.join(" ")
                )));
            }
            *total += meta.len();
            if out.len() >= MAX_FILES || *total > MAX_BYTES {
                return Err(unfit(format!(
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

    fn removed_built_ins_path(&self) -> PathBuf {
        self.catalog.oikome_dir().join(REMOVED_BUILT_INS_FILE)
    }

    fn installed(&self) -> Result<Vec<Installed>, ApiError> {
        read_private(&self.plugins_path())
    }

    pub(crate) fn source_plugin(
        &self,
        kind: SourceKind,
    ) -> Result<Option<(String, String)>, ApiError> {
        self.ensure_built_ins()?;
        for installed in self.installed()? {
            let package = installed.plugin();
            if let Some(source) = package
                .capabilities
                .sources
                .iter()
                .find(|source| source.kind == kind)
            {
                return Ok(Some((package.id, source.id.clone())));
            }
        }
        Ok(None)
    }

    pub(crate) fn installed_plugin(&self, id: &str) -> Result<Installed, ApiError> {
        self.installed()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                ApiError::not_found(format!("there is no plugin {id}"))
                    .because("plugin_unknown")
                    .with("id", id)
            })
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

    /// One of Oikome's own plugins, as its manifest in this build says.
    fn built_in_manifest(id: &str) -> Option<Plugin> {
        Self::built_in(id)?
            .iter()
            .find(|(name, _)| *name == MANIFEST)
            .and_then(|(_, bytes)| read_manifest(bytes, true).ok())
    }

    /// Resolve an MCP adapter only from an installed package. No client has
    /// a Rust implementation or an uninstalled official fallback.
    pub(crate) fn mcp_client(
        &self,
        plugin: &str,
        client: &str,
    ) -> Result<McpClientCapability, ApiError> {
        self.ensure_built_ins()?;
        let package = self.installed_plugin(plugin)?.plugin();
        package
            .capabilities
            .mcp_clients
            .into_iter()
            .find(|capability| capability.id == client)
            .ok_or_else(|| {
                ApiError::not_found(format!("plugin {plugin} has no MCP client {client}"))
                    .because("mcp_client_unknown")
                    .with("id", client)
            })
    }

    /// Install this build's official packages on a new node. A legacy node
    /// with no removal record keeps its old choices; a deliberate removal
    /// stays removed, including after a restart.
    pub(crate) fn ensure_built_ins(&self) -> Result<(), ApiError> {
        // One at a time: listing files and listing plugins both come here,
        // and two installs of one package would move its folder twice.
        static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
        let new_node = !self.plugins_path().exists();
        let has_removal_record = self.removed_built_ins_path().exists();
        if !new_node && !has_removal_record {
            return Ok(());
        }
        if new_node {
            self.change_private(&self.removed_built_ins_path(), |_: &mut Vec<String>| {})?;
        }
        let removed: Vec<String> = read_private(&self.removed_built_ins_path())?;
        for files in BUILT_IN {
            let Some((_, bytes)) = files.iter().find(|(name, _)| *name == MANIFEST) else {
                continue;
            };
            let plugin = read_manifest(bytes, true)?;
            if removed.contains(&plugin.id) || self.installed_plugin(&plugin.id).is_ok() {
                continue;
            }
            let files = files
                .iter()
                .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
                .collect();
            self.install_package(plugin, files)?;
        }
        Ok(())
    }

    /// Installed plugins, then official packages the owner removed.
    pub async fn list_plugins(&self, caller: &Caller, _: Empty) -> Result<Vec<Plugin>, ApiError> {
        caller.require_owner()?;
        self.ensure_built_ins()?;
        let installed = self.installed()?;
        // Only a collection removed while its grant existed is restorable.
        // The editable list of collections alone is never proof of a grant.
        let session = self.session(None);
        let mut restored = BTreeMap::new();
        for plugin in &installed {
            for collection in &plugin.restore_collections {
                if self.catalog.get_space(collection).is_ok()
                    && !self.catalog.has_space_grant(&plugin.did, collection)
                {
                    session.grant_space(collection, std::slice::from_ref(&plugin.did))?;
                    restored
                        .entry(plugin.id.clone())
                        .or_insert_with(Vec::new)
                        .push(collection.clone());
                }
            }
        }
        if !restored.is_empty() {
            self.change_private(&self.plugins_path(), |all: &mut Vec<Installed>| {
                for plugin in all {
                    if let Some(names) = restored.get(&plugin.id) {
                        plugin
                            .restore_collections
                            .retain(|name| !names.contains(name));
                    }
                }
            })?;
        }
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

    /// Remember a grant lost because its collection was removed. This is
    /// recorded before the collection is dropped, not inferred from card data.
    pub(crate) fn note_removed_collection(&self, space: &str) -> Result<(), ApiError> {
        let affected = self.installed()?.iter().any(|plugin| {
            plugin.collections.iter().any(|name| name == space)
                && self.catalog.has_space_grant(&plugin.did, space)
        });
        if affected {
            self.change_private(&self.plugins_path(), |all: &mut Vec<Installed>| {
                for plugin in all {
                    if plugin.collections.iter().any(|name| name == space)
                        && !plugin.restore_collections.iter().any(|name| name == space)
                    {
                        plugin.restore_collections.push(space.to_string());
                    }
                }
            })?;
        }
        Ok(())
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
            return Err(
                ApiError::invalid("choose a folder on this computer").because("plugin_folder")
            );
        }
        let root = given.canonicalize().map_err(|_| {
            ApiError::invalid(format!("{} is not there", given.display()))
                .because("plugin_not_there")
                .with("path", given.display().to_string())
        })?;
        if !root.is_dir() {
            return Err(
                ApiError::invalid(format!("{} is not a folder", root.display()))
                    .because("plugin_not_folder")
                    .with("path", root.display().to_string()),
            );
        }
        if !root.join(MANIFEST).is_file() {
            return Err(ApiError::invalid(format!(
                "{} has no {MANIFEST}, so it isn't a plugin",
                root.display()
            ))
            .because("plugin_no_manifest")
            .with("path", root.display().to_string()));
        }
        let files = read_folder(&root)?;
        let manifest = files
            .iter()
            .find(|(name, _)| name == MANIFEST)
            .map(|(_, bytes)| bytes.as_slice())
            .unwrap_or_default();
        let plugin = read_manifest(manifest, false)?;
        for card in &plugin.capabilities.cards {
            if !files.iter().any(|(name, _)| *name == card.entry) {
                return Err(unfit(format!(
                    "the plugin's entry, {}, isn't in its folder",
                    card.entry
                )));
            }
        }
        Ok((plugin, files))
    }

    /// Register an official or external package. Installation has no data,
    /// source or Home side effects. Updating preserves existing card grants
    /// and the layout behavior the installed record already had.
    pub async fn install_plugin(
        &self,
        caller: &Caller,
        req: InstallPlugin,
    ) -> Result<Plugin, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (plugin, files) = match (&req.built_in, &req.path) {
            (Some(id), None) => {
                let files = Self::built_in(id).ok_or_else(|| {
                    ApiError::not_found(format!("Oikome has no plugin {id}"))
                        .because("plugin_not_built_in")
                        .with("id", id)
                })?;
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
                return Err(
                    ApiError::invalid("add one of Oikome's plugins or a plugin's folder")
                        .because("plugin_which"),
                )
            }
        };
        self.install_package(plugin, files)
    }

    /// Install already validated package files through the common lifecycle.
    pub(crate) fn install_package(&self, plugin: Plugin, files: Files) -> Result<Plugin, ApiError> {
        let existing = self.installed_plugin(&plugin.id).ok();
        if existing.is_none() && self.installed()?.len() >= MAX_INSTALLED {
            return Err(ApiError::invalid(format!(
                "up to {MAX_INSTALLED} plugins can be added; remove one first"
            ))
            .because("plugins_too_many")
            .with("max", MAX_INSTALLED.to_string()));
        }
        // A preview's code is checked before the package is kept: it must be
        // there, and be a component the sandbox loads.
        for preview in &plugin.capabilities.preview {
            let Some(entry) = &preview.entry else {
                continue;
            };
            let wasm = files
                .iter()
                .find(|(name, _)| name == entry)
                .map(|(_, bytes)| bytes)
                .ok_or_else(|| unfit(format!("the package has no {entry}")))?;
            crate::previews::load(wasm).map_err(|e| unfit(e.to_string()))?;
        }
        self.put_files(&plugin.id, files)?;
        let did = match &existing {
            Some(p) => p.did.clone(),
            None => NodeIdentity::generate()
                .map_err(ApiError::internal)?
                .did()
                .to_string(),
        };
        let collections = if plugin.capabilities.cards.is_empty() {
            Vec::new()
        } else {
            existing
                .as_ref()
                .map(|p| p.collections.clone())
                .unwrap_or_default()
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
            // Keep the old grants as the starting set so an update that
            // removes every card also revokes them.
            collections: existing
                .as_ref()
                .map(|p| p.collections.clone())
                .unwrap_or_default(),
            restore_collections: if plugin.capabilities.cards.is_empty() {
                Vec::new()
            } else {
                existing
                    .as_ref()
                    .map(|p| p.restore_collections.clone())
                    .unwrap_or_default()
            },
            installed_at_ms: now_ms(),
            capabilities: Some(plugin.capabilities),
            home_by_default: existing.map(|p| p.home_by_default).unwrap_or(false),
            translations: plugin.translations,
        };
        let installed = self.grant_plugin(installed, collections)?;
        if Self::built_in(&installed.id).is_some() {
            self.change_private(
                &self.removed_built_ins_path(),
                |removed: &mut Vec<String>| {
                    removed.retain(|id| id != &installed.id);
                },
            )?;
        }
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
        if installed.capabilities().cards.is_empty() {
            return Err(
                ApiError::invalid("this plugin has no card that reads collections")
                    .because("plugin_has_no_card"),
            );
        }
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
        if Self::built_in(&installed.id).is_some() {
            self.change_private(
                &self.removed_built_ins_path(),
                |removed: &mut Vec<String>| {
                    if !removed.contains(&req.id) {
                        removed.push(req.id.clone());
                    }
                },
            )?;
        }
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
        let in_place = self.session(None).in_place_location(&req.space)?;
        let read = async {
            if let Some(location) = &in_place {
                return self
                    .photos_for_plugin(&session, &req.space, location, req.limit)
                    .await
                    .map(Some);
            }
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

    /// The formats installed plugins make previews of, each with the name
    /// of the plugin that does (the first installed that declares it).
    pub(crate) fn preview_formats(&self) -> Result<crate::in_place::PreviewFormats, ApiError> {
        self.ensure_built_ins()?;
        let mut formats = crate::in_place::PreviewFormats::new();
        for plugin in self.installed()? {
            for preview in plugin.capabilities().preview {
                if preview.entry.is_none() {
                    continue;
                }
                for format in preview.formats {
                    formats.entry(format).or_insert_with(|| plugin.id.clone());
                }
            }
        }
        Ok(formats)
    }

    /// The installed plugin that makes previews of files with `extension`
    /// (lower case), and its code: the first installed that declares it.
    pub(crate) fn previewer_of(
        &self,
        extension: &str,
    ) -> Result<Option<crate::previews::Previewer>, ApiError> {
        self.ensure_built_ins()?;
        for plugin in self.installed()? {
            for preview in plugin.capabilities().preview {
                let Some(entry) = preview.entry else {
                    continue;
                };
                if !preview.formats.iter().any(|format| format == extension) || !safe_path(&entry) {
                    continue;
                }
                let wasm = self.plugins_dir().join(&plugin.id).join(&entry);
                return Ok(Some(crate::previews::Previewer {
                    plugin: plugin.id,
                    version: plugin.version,
                    wasm,
                }));
            }
        }
        Ok(None)
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
        if plugin
            .capabilities()
            .cards
            .iter()
            .any(|card| card.entry == path)
        {
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
                ))
                .because("plugin_shared")
                .with("name", name.as_str()));
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

impl Api {
    /// A plugin's read of a folder viewed in place: its photos as the
    /// photos of a folder source were (`file`, `title`, `place`, `taken`,
    /// `credit`, `photo`), read as the plugin, so its grants decide, with
    /// each photo's thumbnail from the index.
    async fn photos_for_plugin(
        &self,
        session: &OikomeSession,
        space: &str,
        location: &oikome_catalog::Location,
        limit: Option<u32>,
    ) -> Result<Rows, ApiError> {
        use std::sync::Arc;

        use arrow_array::{Array, BinaryArray, RecordBatch, StringArray};
        use arrow_schema::{DataType, Field, Schema};
        let limit = limit.unwrap_or(MAX_ROWS).clamp(1, MAX_ROWS);
        let sql = format!(
            "SELECT path, title, place, taken, credit FROM {}.{} WHERE kind = 'image' \
             ORDER BY taken DESC LIMIT {limit}",
            quote_ident(space),
            quote_ident(oikome_node::inplace::FILES)
        );
        let QueryResult::Batches { batches, .. } = session.sql(&sql).await? else {
            return Err(ApiError::internal("a query returned no rows"));
        };
        let schema = Arc::new(Schema::new(vec![
            Field::new("file", DataType::Utf8, false),
            Field::new("title", DataType::Utf8, true),
            Field::new("place", DataType::Utf8, true),
            Field::new("taken", DataType::Utf8, true),
            Field::new("credit", DataType::Utf8, true),
            Field::new("photo", DataType::Binary, true),
        ]));
        let mut out = Vec::new();
        for batch in batches {
            let text = |i: usize| -> Vec<Option<String>> {
                let column = batch.column(i);
                let strings = column.as_any().downcast_ref::<StringArray>();
                (0..batch.num_rows())
                    .map(|r| {
                        strings
                            .filter(|s| !s.is_null(r))
                            .map(|s| s.value(r).to_string())
                    })
                    .collect()
            };
            let paths = text(0);
            let photos: Vec<Option<Vec<u8>>> = paths
                .iter()
                .map(|p| {
                    p.as_deref().and_then(|p| {
                        oikome_node::inplace::thumb(&self.catalog, location, p)
                            .ok()
                            .flatten()
                    })
                })
                .collect();
            out.push(
                RecordBatch::try_new(
                    schema.clone(),
                    vec![
                        Arc::new(StringArray::from(
                            paths
                                .iter()
                                .map(|p| p.clone().unwrap_or_default())
                                .collect::<Vec<_>>(),
                        )),
                        Arc::new(StringArray::from(text(1))),
                        Arc::new(StringArray::from(text(2))),
                        Arc::new(StringArray::from(text(3))),
                        Arc::new(StringArray::from(text(4))),
                        Arc::new(BinaryArray::from_iter(photos.iter().map(|p| p.as_deref()))),
                    ],
                )
                .map_err(ApiError::internal)?,
            );
        }
        let schema: arrow_schema::SchemaRef = schema;
        encode_rows(&schema, &out, limit as usize, MAX_READ_BYTES)
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
            assert_eq!(plugin.translations.len(), 7, "{} translations", plugin.id);
            for text in plugin.translations.values() {
                assert!(text.description.is_some(), "{} description", plugin.id);
                for card in &plugin.capabilities.cards {
                    assert!(text.cards.contains_key(&card.id) || card.name == plugin.name);
                }
                for source in &plugin.capabilities.sources {
                    assert!(text.sources.contains_key(&source.id) || source.name == plugin.name);
                }
            }
            for card in plugin.capabilities.cards {
                assert!(
                    files.iter().any(|(n, _)| *n == card.entry),
                    "{}: {}",
                    plugin.id,
                    card.id
                );
            }
        }
    }
}
