// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/utils/plugins/src/plugin_namespace.rs` (the manifest probe:
// the Agent Plugins root `plugin.json` by its `$schema`, then
// `.codex-plugin`, `.claude-plugin` and `.cursor-plugin`, refusing a
// symbolic link or a non-file) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `.hanafubuki-plugin/plugin.json`, this
// project's own format, is probed first; the probe returns the format with
// the path.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::ManifestFormat;

/// The Agent Plugins manifest schema this store reads.
pub const AGENT_PLUGINS_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
const AGENT_PLUGINS_SCHEMA_PREFIX: &str = "https://agent-plugins.org/schemas/";

/// The manifest formats in the order they are probed after ours: a root
/// `plugin.json` counts only when its `$schema` is an Agent Plugins one.
const DIRECTORY_FORMATS: [ManifestFormat; 3] = [
    ManifestFormat::Codex,
    ManifestFormat::Claude,
    ManifestFormat::Cursor,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchemaStatus {
    Supported,
    Unsupported,
    Unrelated,
}

pub(crate) fn agent_plugins_schema_status(contents: &str) -> SchemaStatus {
    let Ok(value) = serde_json::from_str::<Value>(contents) else {
        return SchemaStatus::Unrelated;
    };
    match value.get("$schema").and_then(Value::as_str) {
        Some(AGENT_PLUGINS_SCHEMA) => SchemaStatus::Supported,
        Some(schema) if schema.starts_with(AGENT_PLUGINS_SCHEMA_PREFIX) => {
            SchemaStatus::Unsupported
        }
        _ => SchemaStatus::Unrelated,
    }
}

/// The manifest under `root` and its format. A manifest, or its directory,
/// that is a symbolic link or not a regular file stops the probe: it must
/// not hide a manifest of lower precedence behind a benign one.
pub fn find_manifest(root: &Path) -> Option<(ManifestFormat, PathBuf)> {
    match directory_manifest(root, ManifestFormat::Hanafubuki) {
        Probe::Found(path) => return Some((ManifestFormat::Hanafubuki, path)),
        Probe::Refused => return None,
        Probe::Absent => {}
    }
    let root_manifest = root.join(ManifestFormat::AgentPlugins.relative_path());
    match fs::symlink_metadata(&root_manifest) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => return None,
        Ok(_) => {
            let related = fs::read_to_string(&root_manifest).is_ok_and(|contents| {
                agent_plugins_schema_status(&contents) != SchemaStatus::Unrelated
            });
            if related {
                return Some((ManifestFormat::AgentPlugins, root_manifest));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    for format in DIRECTORY_FORMATS {
        match directory_manifest(root, format) {
            Probe::Found(path) => return Some((format, path)),
            Probe::Refused => return None,
            Probe::Absent => {}
        }
    }
    None
}

enum Probe {
    Found(PathBuf),
    Absent,
    Refused,
}

fn directory_manifest(root: &Path, format: ManifestFormat) -> Probe {
    let path = root.join(format.relative_path());
    let Some(directory) = path.parent() else {
        return Probe::Refused;
    };
    match fs::symlink_metadata(directory) {
        Ok(metadata) if !metadata.is_dir() => return Probe::Refused,
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Probe::Absent,
        Err(_) => return Probe::Refused,
    }
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => Probe::Found(path),
        Ok(_) => Probe::Refused,
        Err(error) if error.kind() == ErrorKind::NotFound => Probe::Absent,
        Err(_) => Probe::Refused,
    }
}
