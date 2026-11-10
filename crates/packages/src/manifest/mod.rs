// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/plugin/src/manifest.rs` (the parsed manifest: metadata, the
// component paths, the interface) and the loading half of
// `codex-rs/core-plugins/src/manifest.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: resources are paths under the package
// root; apps and `extensions["com.openai"]` are not ported; the manifest
// carries its format, the declared `commands`, this project's code blocks
// and the issues its reading found; no manifest cache.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

mod agent_plugins;
mod code;
mod fields;
mod legacy;
mod probe;

pub use code::{
    ContributedSection, ContributedTool, PackageCode, PackageContributes, PackagePermissions,
    PackageRuntime, ProcessRuntime, RuntimeKind, WasmRuntime, WasmRuntimeLimits, WorkspaceAccess,
    DEFAULT_WASM_MEMORY_BYTES,
};
pub(crate) use probe::{agent_plugins_schema_status, SchemaStatus};
pub use probe::{find_manifest, AGENT_PLUGINS_SCHEMA};

/// Where a package's manifest is, which says how it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestFormat {
    /// `.hanafubuki-plugin/plugin.json`: Codex's fields, plus code.
    Hanafubuki,
    /// A root `plugin.json` with the Agent Plugins 1.0.0 `$schema`.
    AgentPlugins,
    Codex,
    Claude,
    Cursor,
}

impl ManifestFormat {
    pub const fn relative_path(self) -> &'static str {
        match self {
            Self::Hanafubuki => ".hanafubuki-plugin/plugin.json",
            Self::AgentPlugins => "plugin.json",
            Self::Codex => ".codex-plugin/plugin.json",
            Self::Claude => ".claude-plugin/plugin.json",
            Self::Cursor => ".cursor-plugin/plugin.json",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hanafubuki => "hanafubuki",
            Self::AgentPlugins => "agent-plugins",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
        }
    }
}

/// A package's manifest, its component paths resolved under the package
/// root. A field that cannot be used is left out with an issue; only a
/// manifest that is not a JSON object, or whose identity does not read,
/// fails.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    pub paths: ManifestPaths,
    pub interface: Option<ManifestInterface>,
    /// What runs, in this project's format only.
    pub code: Option<PackageCode>,
}

/// The components a manifest declares. When one is not declared, Claude's
/// default location applies (`skills/`, `.mcp.json`, `hooks/hooks.json`,
/// `commands/`); that is for the reader of each component to apply.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ManifestPaths {
    pub skills: Vec<PathBuf>,
    pub mcp_servers: Option<McpServersDeclaration>,
    pub hooks: Option<HooksDeclaration>,
    /// Read only at install, when commands become Skills.
    pub commands: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum McpServersDeclaration {
    Path(PathBuf),
    /// The servers written in the manifest, as `.mcp.json` holds them.
    Inline(Map<String, Value>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum HooksDeclaration {
    Paths(Vec<PathBuf>),
    /// Hook files written in the manifest, as `hooks.json` holds one.
    Inline(Vec<Map<String, Value>>),
}

/// Codex's `interface`: how a package presents itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestInterface {
    pub display_name: Option<String>,
    pub short_description: Option<String>,
    pub long_description: Option<String>,
    pub developer_name: Option<String>,
    pub category: Option<String>,
    pub capabilities: Vec<String>,
    pub website_url: Option<String>,
    pub privacy_policy_url: Option<String>,
    pub terms_of_service_url: Option<String>,
    pub default_prompt: Option<Vec<String>>,
    pub brand_color: Option<String>,
    pub composer_icon: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub logo_dark: Option<PathBuf>,
    pub screenshots: Vec<PathBuf>,
}

impl PackageManifest {
    /// The name a person reads: the interface's display name, else the
    /// manifest's.
    pub fn display_name(&self) -> &str {
        self.interface
            .as_ref()
            .and_then(|interface| interface.display_name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.name)
    }
}

/// A manifest as it was read from a package.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedManifest {
    pub format: ManifestFormat,
    pub path: PathBuf,
    pub manifest: PackageManifest,
    /// The fields and components left out, each with why.
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// No manifest the probe accepts.
    Missing,
    Unreadable(String),
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => formatter.write_str("missing plugin.json"),
            Self::Unreadable(message) => write!(formatter, "failed to read plugin.json: {message}"),
            Self::Invalid(message) => write!(formatter, "invalid plugin.json: {message}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Reads the manifest of the package at `root`.
pub fn load_manifest(root: &Path) -> Result<LoadedManifest, ManifestError> {
    let (format, path) = find_manifest(root).ok_or(ManifestError::Missing)?;
    let contents = read(&path)?;
    let overlay_path = root.join(ManifestFormat::Codex.relative_path());
    let overlay = (format == ManifestFormat::AgentPlugins)
        .then(|| fs::read_to_string(&overlay_path).ok())
        .flatten();
    let mut issues = Vec::new();
    let manifest = parse(root, format, &contents, overlay.as_deref(), &mut issues)?;
    Ok(LoadedManifest {
        format,
        path,
        manifest,
        issues,
    })
}

/// Reads `contents` as a manifest of `format` for the package at `root`:
/// a marketplace entry's fallback manifest before it is installed.
pub fn parse_manifest(
    root: &Path,
    format: ManifestFormat,
    contents: &str,
) -> Result<(PackageManifest, Vec<String>), ManifestError> {
    let mut issues = Vec::new();
    let manifest = parse(root, format, contents, None, &mut issues)?;
    Ok((manifest, issues))
}

fn parse(
    root: &Path,
    format: ManifestFormat,
    contents: &str,
    overlay: Option<&str>,
    issues: &mut Vec<String>,
) -> Result<PackageManifest, ManifestError> {
    let object = json_object(contents)?;
    match format {
        ManifestFormat::AgentPlugins => {
            agent_plugins::parse(root, &object, overlay.map(json_object), issues)
        }
        ManifestFormat::Hanafubuki => {
            let mut manifest = legacy::parse(root, &object, issues);
            manifest.code = code::parse(&object, issues);
            Ok(manifest)
        }
        ManifestFormat::Codex | ManifestFormat::Claude | ManifestFormat::Cursor => {
            Ok(legacy::parse(root, &object, issues))
        }
    }
}

fn read(path: &Path) -> Result<String, ManifestError> {
    fs::read_to_string(path).map_err(|error| ManifestError::Unreadable(error.to_string()))
}

fn json_object(contents: &str) -> Result<Map<String, Value>, ManifestError> {
    match serde_json::from_str::<Value>(contents) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(ManifestError::Invalid(
            "the manifest is not a JSON object".to_string(),
        )),
        Err(error) => Err(ManifestError::Invalid(error.to_string())),
    }
}
