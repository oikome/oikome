// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

//! One-click MCP setup for code CLIs on the node's computer. The owner never
//! has to edit JSON or TOML, and existing settings beside Oikome stay intact.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value as TomlValue};

use crate::{ApiError, AssistantApp, ErrorCode};

fn read(path: &Path) -> Result<String, ApiError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(ApiError::internal(e)),
    }
}

fn conflict(path: &Path) -> ApiError {
    ApiError::new(
        ErrorCode::Conflict,
        format!("{} already has an Oikome MCP entry", path.display()),
    )
    .because("assistant_config_exists")
    .with("path", path.display().to_string())
}

fn write(path: &Path, contents: String) -> Result<(), ApiError> {
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = path.parent().unwrap_or(path);
    std::fs::create_dir_all(dir).map_err(ApiError::internal)?;
    let existing = std::fs::metadata(path).ok().map(|m| m.permissions());
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("oikome.{}-{n}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(existing.as_ref().map_or(0o600, |p| p.mode() & 0o777));
        }
        let mut file = options.open(&tmp).map_err(ApiError::internal)?;
        file.write_all(contents.as_bytes())
            .map_err(ApiError::internal)?;
        file.sync_all().map_err(ApiError::internal)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ApiError::internal(e)
    })
}

fn check_url(url: &str) -> Result<(), ApiError> {
    let valid = url
        .strip_prefix("http://")
        .and_then(|rest| rest.strip_suffix("/mcp"))
        .is_some_and(|authority| {
            let (host, port) = authority.rsplit_once(':').unwrap_or_default();
            matches!(host, "127.0.0.1" | "localhost" | "[::1]")
                && port.parse::<u16>().is_ok_and(|port| port > 0)
        });
    if valid {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "the MCP address must be an HTTP loopback address ending in /mcp",
        ))
    }
}

fn config_path(app: AssistantApp) -> Result<PathBuf, ApiError> {
    let (variable, directory, file) = match app {
        AssistantApp::KimiCode => ("KIMI_CODE_HOME", ".kimi-code", "mcp.json"),
        AssistantApp::Codex => ("CODEX_HOME", ".codex", "config.toml"),
        _ => {
            return Err(ApiError::invalid(
                "one-click setup is available for Kimi Code and Codex",
            ));
        }
    };
    if let Some(root) = std::env::var_os(variable) {
        return Ok(PathBuf::from(root).join(file));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join(directory).join(file))
        .ok_or_else(|| {
            ApiError::invalid("this computer has no home directory for the client's configuration")
        })
}

fn install_kimi(path: &Path, url: &str, token: &str, replace: bool) -> Result<(), ApiError> {
    let text = read(path)?;
    let mut document = if text.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(&text)
            .map_err(|e| ApiError::invalid(format!("{} is not valid JSON: {e}", path.display())))?
    };
    let root = document.as_object_mut().ok_or_else(|| {
        ApiError::invalid(format!("{} must contain a JSON object", path.display()))
    })?;
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| ApiError::invalid("mcpServers must be a JSON object"))?;
    if servers.contains_key("oikome") && !replace {
        return Err(conflict(path));
    }
    let server = servers
        .entry("oikome")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| ApiError::invalid("the Oikome MCP entry must be a JSON object"))?;
    server.insert("url".into(), Value::String(url.into()));
    server.insert(
        "headers".into(),
        json!({ "Authorization": format!("Bearer {token}") }),
    );
    write(
        path,
        serde_json::to_string_pretty(&document).map_err(ApiError::internal)? + "\n",
    )
}

fn install_codex(path: &Path, url: &str, token: &str, replace: bool) -> Result<(), ApiError> {
    let text = read(path)?;
    let mut document = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse::<DocumentMut>()
            .map_err(|e| ApiError::invalid(format!("{} is not valid TOML: {e}", path.display())))?
    };
    let servers = document
        .entry("mcp_servers")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| ApiError::invalid("mcp_servers must be a TOML table"))?;
    if servers.contains_key("oikome") && !replace {
        return Err(conflict(path));
    }
    let server = servers
        .entry("oikome")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| ApiError::invalid("the Oikome MCP entry must be a TOML table"))?;
    server["url"] = toml_edit::value(url);
    let mut headers = InlineTable::new();
    headers.insert("Authorization", TomlValue::from(format!("Bearer {token}")));
    server["http_headers"] = Item::Value(TomlValue::InlineTable(headers));
    write(path, document.to_string())
}

pub(super) fn install(
    app: AssistantApp,
    url: &str,
    token: &str,
    replace: bool,
) -> Result<(), ApiError> {
    check_url(url)?;
    let path = config_path(app)?;
    match app {
        AssistantApp::KimiCode => install_kimi(&path, url, token, replace),
        AssistantApp::Codex => install_codex(&path, url, token, replace),
        _ => Err(ApiError::invalid("this client has no one-click setup")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_configuration_merges_and_preserves_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let kimi = dir.path().join("kimi.json");
        write(&kimi, "{\n  \"enabled\": true,\n  \"mcpServers\": {\"other\": {\"url\": \"https://other\"}},\n  \"custom\": 1\n}\n".into()).unwrap();
        install_kimi(&kimi, "http://127.0.0.1:7342/mcp", "one", false).unwrap();
        let mut saved: Value = serde_json::from_str(&read(&kimi).unwrap()).unwrap();
        assert_eq!(saved["custom"], 1);
        assert_eq!(saved["mcpServers"]["other"]["url"], "https://other");
        assert_eq!(
            saved["mcpServers"]["oikome"]["headers"]["Authorization"],
            "Bearer one"
        );
        assert_eq!(
            install_kimi(&kimi, "http://127.0.0.1:7342/mcp", "two", false)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        saved["mcpServers"]["oikome"]["deferred"] = Value::Bool(true);
        write(&kimi, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
        install_kimi(&kimi, "http://127.0.0.1:7442/mcp", "two", true).unwrap();
        let saved: Value = serde_json::from_str(&read(&kimi).unwrap()).unwrap();
        assert_eq!(saved["mcpServers"]["oikome"]["deferred"], true);
        assert_eq!(
            saved["mcpServers"]["oikome"]["headers"]["Authorization"],
            "Bearer two"
        );

        let codex = dir.path().join("config.toml");
        write(
            &codex,
            "# mine\nmodel = \"mine\"\n\n[mcp_servers.other]\nurl = \"https://other\"\n".into(),
        )
        .unwrap();
        install_codex(&codex, "http://127.0.0.1:7342/mcp", "one", false).unwrap();
        assert!(read(&codex).unwrap().contains("# mine\nmodel = \"mine\""));
        assert_eq!(
            install_codex(&codex, "http://127.0.0.1:7342/mcp", "two", false)
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        let mut document = read(&codex).unwrap().parse::<DocumentMut>().unwrap();
        document["mcp_servers"]["oikome"]["enabled_tools"] =
            toml_edit::value(toml_edit::Array::from_iter(["search"]));
        write(&codex, document.to_string()).unwrap();
        install_codex(&codex, "http://127.0.0.1:7442/mcp", "two", true).unwrap();
        let document = read(&codex).unwrap();
        assert!(document.contains("enabled_tools = [\"search\"]"));
        assert!(document.contains("url = \"http://127.0.0.1:7442/mcp\""));
        assert!(document.contains("[mcp_servers.other]"));
        assert!(!document.contains("--profile"));
    }

    #[test]
    fn only_loopback_mcp_addresses_are_written() {
        for url in [
            "http://127.0.0.1:7342/mcp",
            "http://localhost:1/mcp",
            "http://[::1]:65535/mcp",
        ] {
            check_url(url).unwrap();
        }
        for url in [
            "https://127.0.0.1:7342/mcp",
            "http://example.com/mcp",
            "http://127.0.0.1/mcp",
            "http://127.0.0.1:0/mcp",
            "http://127.0.0.1:7342/other",
        ] {
            check_url(url).unwrap_err();
        }
    }
}
