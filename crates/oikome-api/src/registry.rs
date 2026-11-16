// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

//! The independent official plugin catalog. Release indexes and packages are
//! bounded downloads; the selected package's digest and manifest must match
//! before the ordinary installation lifecycle sees its files.

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use sha2::Digest;

use crate::plugins::{read_manifest, safe_path, Files};
use crate::{Api, ApiError, Caller, Empty, Plugin, RegistryPlugin, RegistryPluginRef};

const DEFAULT_INDEX: &str =
    "https://github.com/oikome/oikome-plugins/releases/latest/download/index.json";
const DEFAULT_DOWNLOAD: &str = "https://github.com/oikome/oikome-plugins/releases/download/";
const MAX_INDEX_BYTES: usize = 2 << 20;
const MAX_PACKAGE_BYTES: u64 = 9 << 20;
const MAX_FILES: usize = 200;
const MAX_UNPACKED_BYTES: usize = 8 << 20;

#[derive(Clone)]
pub(crate) struct Endpoints {
    pub(crate) index: String,
    pub(crate) download_prefix: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            index: std::env::var("OIKOME_PLUGIN_REGISTRY_INDEX")
                .unwrap_or_else(|_| DEFAULT_INDEX.into()),
            download_prefix: std::env::var("OIKOME_PLUGIN_DOWNLOAD_PREFIX")
                .unwrap_or_else(|_| DEFAULT_DOWNLOAD.into()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Index {
    schema_version: u32,
    plugins: Vec<IndexEntry>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IndexEntry {
    manifest: Value,
    download: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone)]
struct Entry {
    plugin: Plugin,
    download: String,
    sha256: String,
    bytes: u64,
}

fn invalid(detail: impl Into<String>) -> ApiError {
    ApiError::invalid(detail).because("plugin_registry_invalid")
}

async fn download(client: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, ApiError> {
    let response = client.get(url).send().await.map_err(|e| {
        ApiError::invalid(format!(
            "the official plugin catalog could not be reached: {e}"
        ))
        .because("plugin_registry_unavailable")
    })?;
    if !response.status().is_success() {
        return Err(ApiError::invalid(format!(
            "the official plugin catalog answered {}",
            response.status()
        ))
        .because("plugin_registry_unavailable"));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(invalid("the official plugin download is too large"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            ApiError::invalid(format!("the official plugin download was interrupted: {e}"))
                .because("plugin_registry_unavailable")
        })?;
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(invalid("the official plugin download is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

impl Api {
    async fn registry_entries(&self) -> Result<Vec<Entry>, ApiError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("Oikome/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ApiError::internal)?;
        let bytes = download(&client, &self.registry.index, MAX_INDEX_BYTES).await?;
        let index: Index = serde_json::from_slice(&bytes)
            .map_err(|e| invalid(format!("the official plugin index is not valid JSON: {e}")))?;
        if index.schema_version != 1 || index.plugins.is_empty() || index.plugins.len() > 64 {
            return Err(invalid(
                "the official plugin index has an unsupported shape",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut out = Vec::new();
        for entry in index.plugins {
            if !entry.download.starts_with(&self.registry.download_prefix)
                || entry.sha256.len() != 64
                || !entry
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
                || entry.bytes == 0
                || entry.bytes > MAX_PACKAGE_BYTES
            {
                return Err(invalid("an official plugin index entry is not valid"));
            }
            let manifest = serde_json::to_vec(&entry.manifest).map_err(ApiError::internal)?;
            let plugin = read_manifest(&manifest, false)?;
            if !ids.insert(plugin.id.clone()) {
                return Err(invalid("an official plugin id appears twice in the index"));
            }
            out.push(Entry {
                plugin,
                download: entry.download,
                sha256: entry.sha256,
                bytes: entry.bytes,
            });
        }
        Ok(out)
    }

    /// The independent official repository's packages, with local state.
    pub async fn list_registry_plugins(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<RegistryPlugin>, ApiError> {
        caller.require_owner()?;
        let mut out = Vec::new();
        for entry in self.registry_entries().await? {
            let mut plugin = entry.plugin;
            if let Ok(installed) = self.installed_plugin(&plugin.id) {
                plugin.installed = true;
                plugin.collections = installed.plugin().collections;
            }
            out.push(RegistryPlugin {
                plugin,
                download: entry.download,
                sha256: entry.sha256,
                bytes: entry.bytes,
            });
        }
        Ok(out)
    }

    /// Download an exact catalog version, verify it, then install it normally.
    pub async fn install_registry_plugin(
        &self,
        caller: &Caller,
        req: RegistryPluginRef,
    ) -> Result<Plugin, ApiError> {
        caller.require_owner()?;
        let entry = self
            .registry_entries()
            .await?
            .into_iter()
            .find(|entry| entry.plugin.id == req.id && entry.plugin.version == req.version)
            .ok_or_else(|| {
                ApiError::not_found("that official plugin version is not in the catalog")
                    .because("plugin_registry_version_unknown")
            })?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("Oikome/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ApiError::internal)?;
        let bytes = download(&client, &entry.download, MAX_PACKAGE_BYTES as usize).await?;
        if bytes.len() as u64 != entry.bytes {
            return Err(invalid(
                "the official plugin's size does not match its index entry",
            ));
        }
        let digest = sha2::Sha256::digest(&bytes);
        if format!("{digest:x}") != entry.sha256 {
            return Err(
                invalid("the official plugin's SHA-256 does not match its index entry")
                    .because("plugin_registry_checksum"),
            );
        }
        let (plugin, files) = read_zip(&bytes)?;
        if plugin != entry.plugin {
            return Err(invalid(
                "the official plugin's manifest does not match its index entry",
            ));
        }
        self.refresh()?;
        self.install_package(plugin, files)
    }
}

fn read_zip(bytes: &[u8]) -> Result<(Plugin, Files), ApiError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| invalid(format!("the official plugin is not a valid ZIP: {e}")))?;
    if archive.len() > MAX_FILES {
        return Err(invalid("the official plugin contains too many files"));
    }
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(ApiError::internal)?;
        if !entry.is_file() {
            continue;
        }
        let name = entry
            .name()
            .map_err(|e| invalid(format!("a plugin filename is not valid UTF-8: {e}")))?
            .into_owned();
        if !safe_path(&name) || !names.insert(name.clone()) {
            return Err(invalid(format!("{name} is not a safe, unique plugin file")));
        }
        let mut contents = Vec::new();
        entry
            .by_ref()
            .take(MAX_UNPACKED_BYTES as u64 + 1)
            .read_to_end(&mut contents)
            .map_err(ApiError::internal)?;
        total = total.saturating_add(contents.len());
        if total > MAX_UNPACKED_BYTES {
            return Err(invalid(
                "the official plugin's unpacked files are too large",
            ));
        }
        files.push((name, contents));
    }
    let manifest = files
        .iter()
        .find(|(name, _)| name == "oikome-plugin.json")
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or_else(|| invalid("the official plugin ZIP has no manifest"))?;
    let plugin = read_manifest(manifest, false)?;
    for card in &plugin.capabilities.cards {
        if !files.iter().any(|(name, _)| *name == card.entry) {
            return Err(invalid(format!(
                "the official plugin's entry, {}, isn't in its ZIP",
                card.entry
            )));
        }
    }
    Ok((plugin, files))
}
