// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/marketplace.rs` (a marketplace entry: its
// local, `url` and `git-subdir` sources and their normalization, its
// policy and category, the fallback manifest built from its other fields)
// at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: an entry is read field by field from its
// JSON object; npm sources are skipped as unsupported (not ported yet); a
// source is recorded with the install, so it is serializable.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{InstallationPolicy, Marketplace, MarketplacePlugin};
use crate::manifest::{find_manifest, load_manifest, parse_manifest, ManifestFormat};
use crate::{ManifestInterface, ManifestPaths, PackageManifest, PluginId};

/// Where a package comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PackageSource {
    Local {
        path: PathBuf,
    },
    Git {
        url: String,
        /// The package's directory in the repository.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha: Option<String>,
    },
}

/// The manifest an entry describes, in Codex's legacy format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestFallback {
    pub contents: String,
    /// Whether the entry holds more than its name; without, nothing is
    /// written into a package that has no manifest.
    pub has_metadata: bool,
}

/// The fields of an entry that are not the fallback manifest's.
const ENTRY_FIELDS: [&str; 4] = ["name", "source", "policy", "category"];

pub(super) fn resolve(
    marketplace: &Marketplace,
    value: &Value,
) -> Result<MarketplacePlugin, String> {
    let Value::Object(entry) = value else {
        return Err("an entry is an object".to_string());
    };
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or("the entry has no name")?;
    let id = PluginId::new(name, &marketplace.name).map_err(|error| error.to_string())?;
    let source = source(&marketplace.root, &marketplace.path, entry.get("source"))?;
    let category = entry
        .get("category")
        .and_then(Value::as_str)
        .map(str::to_string);
    let fallback = fallback(name, category.as_deref(), entry);
    let manifest = match &source {
        PackageSource::Local { path } if find_manifest(path).is_some() => {
            load_manifest(path).ok().map(|loaded| loaded.manifest)
        }
        PackageSource::Local { path } if fallback.has_metadata => parse_fallback(path, &fallback),
        PackageSource::Git { .. } if fallback.has_metadata => {
            parse_fallback(Path::new(""), &fallback).map(without_paths)
        }
        PackageSource::Local { .. } | PackageSource::Git { .. } => None,
    };
    let mut interface = manifest
        .as_ref()
        .and_then(|manifest| manifest.interface.clone());
    if let Some(category) = category {
        interface
            .get_or_insert_with(ManifestInterface::default)
            .category = Some(category);
    }
    Ok(MarketplacePlugin {
        id,
        installation: installation(entry),
        interface,
        keywords: manifest
            .as_ref()
            .map(|manifest| manifest.keywords.clone())
            .unwrap_or_default(),
        local_version: manifest
            .as_ref()
            .and_then(|manifest| manifest.version.clone()),
        source,
        fallback,
        manifest,
    })
}

fn installation(entry: &Map<String, Value>) -> InstallationPolicy {
    match entry
        .get("policy")
        .and_then(|policy| policy.get("installation"))
        .and_then(Value::as_str)
    {
        Some("NOT_AVAILABLE") => InstallationPolicy::NotAvailable,
        Some("INSTALLED_BY_DEFAULT") => InstallationPolicy::InstalledByDefault,
        _ => InstallationPolicy::Available,
    }
}

fn parse_fallback(root: &Path, fallback: &ManifestFallback) -> Option<PackageManifest> {
    parse_manifest(root, ManifestFormat::Codex, &fallback.contents)
        .ok()
        .map(|(manifest, _)| manifest)
}

/// A source not yet fetched has no root for its paths to be under.
fn without_paths(mut manifest: PackageManifest) -> PackageManifest {
    manifest.paths = ManifestPaths::default();
    if let Some(interface) = manifest.interface.as_mut() {
        interface.composer_icon = None;
        interface.logo = None;
        interface.logo_dark = None;
        interface.screenshots.clear();
    }
    manifest
}

fn source(root: &Path, manifest: &Path, raw: Option<&Value>) -> Result<PackageSource, String> {
    let string = |object: &Map<String, Value>, key: &str| {
        object.get(key).and_then(Value::as_str).map(str::to_string)
    };
    let selector = |object: &Map<String, Value>, key: &str| {
        string(object, key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    match raw {
        Some(Value::String(path)) => {
            local_path(root, manifest, path).map(|path| PackageSource::Local { path })
        }
        Some(Value::Object(object)) => match object.get("source").and_then(Value::as_str) {
            Some("local") => {
                let path = string(object, "path").ok_or("a local source has a path")?;
                local_path(root, manifest, &path).map(|path| PackageSource::Local { path })
            }
            Some(kind @ ("url" | "git-subdir")) => {
                let url = string(object, "url").ok_or("a git source has a url")?;
                let path = string(object, "path");
                if kind == "git-subdir" && path.is_none() {
                    return Err("a git-subdir source has a path".to_string());
                }
                Ok(PackageSource::Git {
                    url: git_url(root, &url)?,
                    path: path.as_deref().map(subdirectory).transpose()?,
                    ref_name: selector(object, "ref"),
                    sha: selector(object, "sha"),
                })
            }
            _ => Err("the source is not supported".to_string()),
        },
        _ => Err("the source is not supported".to_string()),
    }
}

/// A local source: `./` and a normalized path under the marketplace root
/// (Cursor's marketplaces may leave out the `./`), or the root itself.
fn local_path(root: &Path, manifest: &Path, path: &str) -> Result<PathBuf, String> {
    match path {
        "" => return Err("a local source path must not be empty".to_string()),
        "." | "./" => return Ok(root.to_path_buf()),
        _ => {}
    }
    let relative = path
        .strip_prefix("./")
        .or_else(|| {
            manifest
                .ends_with(".cursor-plugin/marketplace.json")
                .then_some(path)
        })
        .ok_or("a local source path must start with `./`")?;
    if !is_normal(relative) {
        return Err("a local source path must stay within the marketplace root".to_string());
    }
    Ok(root.join(relative))
}

fn subdirectory(path: &str) -> Result<String, String> {
    let path = path.trim();
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.is_empty() {
        return Err("a git source path must not be empty".to_string());
    }
    if !is_normal(path) {
        return Err("a git source path must stay within the repository".to_string());
    }
    Ok(path.to_string())
}

fn is_normal(relative: &str) -> bool {
    !relative.split(['/', '\\']).any(|part| part == "..")
        && Path::new(relative)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn git_url(root: &Path, url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("a git source url must not be empty".to_string());
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(github_url(url));
    }
    if ["./", "../", ".\\", "..\\"]
        .iter()
        .any(|prefix| url.starts_with(prefix))
    {
        return relative_git_url(root, url);
    }
    if url.starts_with("file://")
        || url.starts_with('/')
        || url.starts_with("ssh://")
        || (url.starts_with("git@") && url.contains(':'))
    {
        return Ok(url.to_string());
    }
    github_shorthand(url).ok_or_else(|| format!("invalid git source url: {url}"))
}

fn relative_git_url(root: &Path, url: &str) -> Result<String, String> {
    let mut resolved = root.to_path_buf();
    for segment in url.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                return Err(
                    "a relative git source url must stay within the marketplace root".to_string(),
                )
            }
            segment => resolved.push(segment),
        }
    }
    Ok(resolved.display().to_string())
}

pub(super) fn github_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if url.starts_with("https://github.com/") && !url.ends_with(".git") {
        format!("{url}.git")
    } else {
        url.to_string()
    }
}

/// `owner/repo` (with or without `.git`) on GitHub.
pub(super) fn github_shorthand(source: &str) -> Option<String> {
    let mut segments = source.split('/');
    let (owner, repo) = (segments.next()?, segments.next()?);
    let segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    if segments.next().is_some() || !segment(owner) || !segment(repo) {
        return None;
    }
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (!repo.is_empty()).then(|| format!("https://github.com/{owner}/{repo}.git"))
}

fn fallback(name: &str, category: Option<&str>, entry: &Map<String, Value>) -> ManifestFallback {
    let fields = entry
        .iter()
        .filter(|(key, _)| !ENTRY_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<String, Value>>();
    let mut manifest = fields.clone();
    manifest.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(category) = category {
        manifest.insert("category".to_string(), Value::String(category.to_string()));
    }
    if let Some(interface) = fallback_interface(&fields, category) {
        manifest.insert("interface".to_string(), interface);
    }
    let contents = serde_json::to_string_pretty(&Value::Object(manifest))
        .unwrap_or_else(|_| Value::Object(Map::new()).to_string());
    ManifestFallback {
        contents,
        has_metadata: !fields.is_empty() || category.is_some(),
    }
}

/// Claude's entry fields (`displayName`, `author.name`, `homepage`) and the
/// category as Codex's interface.
fn fallback_interface(fields: &Map<String, Value>, category: Option<&str>) -> Option<Value> {
    let mut interface = fields
        .get("interface")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let has_website_alias = interface.contains_key("websiteURL");
    let mut fill = |key: &str, value: Option<&str>| {
        if let Some(value) = value {
            interface
                .entry(key.to_string())
                .or_insert_with(|| Value::String(value.to_string()));
        }
    };
    fill(
        "displayName",
        fields.get("displayName").and_then(Value::as_str),
    );
    let author = fields
        .get("author")
        .and_then(|author| author.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    fill("developerName", author);
    if !has_website_alias {
        fill("websiteUrl", fields.get("homepage").and_then(Value::as_str));
    }
    if let Some(category) = category.map(str::trim).filter(|value| !value.is_empty()) {
        interface.insert("category".to_string(), Value::String(category.to_string()));
    }
    (!interface.is_empty()).then_some(Value::Object(interface))
}
