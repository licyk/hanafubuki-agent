// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/manifest.rs` (the legacy manifest of
// `.codex-plugin`, `.claude-plugin` and `.cursor-plugin`: its fields, the
// `./` path rules, the interface and `defaultPrompt`) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: every field is read on its own, so a value
// of the wrong type costs only that field (Codex fails the manifest for
// some); a bad path is rejected the same way on every platform (`..` split
// on both separators, no root, no drive); the issues are returned rather
// than logged.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::fields::{kind, non_empty_trimmed, Fields};
use super::{HooksDeclaration, ManifestInterface, ManifestPaths, McpServersDeclaration};
use crate::PackageManifest;

const MAX_DEFAULT_PROMPT_COUNT: usize = 3;
const MAX_DEFAULT_PROMPT_CHARS: usize = 512;

pub(super) fn parse(
    root: &Path,
    object: &Map<String, Value>,
    issues: &mut Vec<String>,
) -> PackageManifest {
    let mut fields = Fields::new(object, "", issues);
    let name = fields
        .string("name")
        .filter(|name| !name.trim().is_empty())
        .or_else(|| root_name(root))
        .unwrap_or_default();
    let version = fields.string("version").and_then(non_empty_trimmed);
    let description = fields.string("description");
    let keywords = fields.strings("keywords");
    let paths = ManifestPaths {
        skills: resolve_paths(root, &mut fields, "skills"),
        mcp_servers: mcp_servers(root, &mut fields),
        hooks: hooks(root, &mut fields),
        commands: resolve_paths(root, &mut fields, "commands"),
        agents: resolve_paths(root, &mut fields, "agents"),
    };
    let interface = fields
        .object("interface")
        .and_then(|interface| self::interface(root, interface, fields.issues()));
    PackageManifest {
        name,
        version,
        description,
        keywords,
        paths,
        interface,
        code: None,
    }
}

fn root_name(root: &Path) -> Option<String> {
    root.file_name()?.to_str().map(str::to_string)
}

fn interface(
    root: &Path,
    object: &Map<String, Value>,
    issues: &mut Vec<String>,
) -> Option<ManifestInterface> {
    let mut fields = Fields::new(object, "interface.", issues);
    let aliased = |fields: &mut Fields<'_>, key: &str, alias: &str| {
        fields.string(key).or_else(|| fields.string(alias))
    };
    let interface = ManifestInterface {
        display_name: fields.string("displayName"),
        short_description: fields.string("shortDescription"),
        long_description: fields.string("longDescription"),
        developer_name: fields.string("developerName"),
        category: fields.string("category"),
        capabilities: fields.strings("capabilities"),
        website_url: aliased(&mut fields, "websiteUrl", "websiteURL"),
        privacy_policy_url: aliased(&mut fields, "privacyPolicyUrl", "privacyPolicyURL"),
        terms_of_service_url: aliased(&mut fields, "termsOfServiceUrl", "termsOfServiceURL"),
        default_prompt: default_prompts(&mut fields),
        brand_color: fields.string("brandColor"),
        composer_icon: resolve_path_field(root, &mut fields, "composerIcon"),
        logo: resolve_path_field(root, &mut fields, "logo"),
        logo_dark: resolve_path_field(root, &mut fields, "logoDark"),
        screenshots: resolve_paths(root, &mut fields, "screenshots"),
    };
    (interface != ManifestInterface::default()).then_some(interface)
}

fn mcp_servers(root: &Path, fields: &mut Fields<'_>) -> Option<McpServersDeclaration> {
    match fields.raw("mcpServers")? {
        Value::String(path) => {
            resolve_path(root, fields, "mcpServers", path).map(McpServersDeclaration::Path)
        }
        Value::Object(servers) => Some(McpServersDeclaration::Inline(servers.clone())),
        other => {
            let message = format!("expected a string or an object, found {}", kind(other));
            fields.issue("mcpServers", &message);
            None
        }
    }
}

/// Codex's four shapes: a path, paths, a hook file, hook files.
fn hooks(root: &Path, fields: &mut Fields<'_>) -> Option<HooksDeclaration> {
    let raw = fields.raw("hooks")?;
    let invalid = |fields: &mut Fields<'_>| {
        let message = format!(
            "expected a string, a list of strings, an object, or a list of objects; found {}",
            kind(raw)
        );
        fields.issue("hooks", &message);
        None
    };
    match raw {
        Value::String(path) => resolve_path(root, fields, "hooks", path)
            .map(|path| HooksDeclaration::Paths(vec![path])),
        Value::Object(file) => Some(HooksDeclaration::Inline(vec![file.clone()])),
        Value::Array(items) if items.iter().all(Value::is_string) => {
            let paths = items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|path| resolve_path(root, fields, "hooks", path))
                .collect::<Vec<_>>();
            (!paths.is_empty()).then_some(HooksDeclaration::Paths(paths))
        }
        Value::Array(items) if items.iter().all(Value::is_object) => {
            let files = items
                .iter()
                .filter_map(Value::as_object)
                .cloned()
                .collect::<Vec<_>>();
            (!files.is_empty()).then_some(HooksDeclaration::Inline(files))
        }
        _ => invalid(fields),
    }
}

/// `defaultPrompt`: a string, or at most three strings, each of at most 512
/// characters once its whitespace is collapsed.
fn default_prompts(fields: &mut Fields<'_>) -> Option<Vec<String>> {
    match fields.raw("defaultPrompt")? {
        Value::String(prompt) => default_prompt(fields, "defaultPrompt", prompt).map(|p| vec![p]),
        Value::Array(items) => {
            let mut prompts = Vec::new();
            for (index, item) in items.iter().enumerate() {
                if prompts.len() >= MAX_DEFAULT_PROMPT_COUNT {
                    let message =
                        format!("at most {MAX_DEFAULT_PROMPT_COUNT} prompts are supported");
                    fields.issue("defaultPrompt", &message);
                    break;
                }
                let field = format!("defaultPrompt[{index}]");
                match item {
                    Value::String(prompt) => prompts.extend(default_prompt(fields, &field, prompt)),
                    other => {
                        let message = format!("expected a string, found {}", kind(other));
                        fields.issue(&field, &message);
                    }
                }
            }
            (!prompts.is_empty()).then_some(prompts)
        }
        other => {
            let message = format!(
                "expected a string or a list of strings, found {}",
                kind(other)
            );
            fields.issue("defaultPrompt", &message);
            None
        }
    }
}

fn default_prompt(fields: &mut Fields<'_>, field: &str, prompt: &str) -> Option<String> {
    let prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if prompt.is_empty() {
        fields.issue(field, "the prompt must not be empty");
        return None;
    }
    if prompt.chars().count() > MAX_DEFAULT_PROMPT_CHARS {
        let message = format!("the prompt must be at most {MAX_DEFAULT_PROMPT_CHARS} characters");
        fields.issue(field, &message);
        return None;
    }
    Some(prompt)
}

/// A path or a list of paths.
pub(super) fn resolve_paths(root: &Path, fields: &mut Fields<'_>, key: &str) -> Vec<PathBuf> {
    match fields.raw(key) {
        None => Vec::new(),
        Some(Value::String(path)) => resolve_path(root, fields, key, path).into_iter().collect(),
        Some(Value::Array(_)) => fields
            .strings(key)
            .iter()
            .filter_map(|path| resolve_path(root, fields, key, path))
            .collect(),
        Some(other) => {
            let message = format!(
                "expected a string or a list of strings, found {}",
                kind(other)
            );
            fields.issue(key, &message);
            Vec::new()
        }
    }
}

fn resolve_path_field(root: &Path, fields: &mut Fields<'_>, key: &str) -> Option<PathBuf> {
    let path = fields.string(key)?;
    resolve_path(root, fields, key, &path)
}

/// A manifest path: `./` and a relative path that stays in the package.
pub(super) fn resolve_path(
    root: &Path,
    fields: &mut Fields<'_>,
    key: &str,
    path: &str,
) -> Option<PathBuf> {
    match relative_manifest_path(path) {
        Ok(None) => None,
        Ok(Some(relative)) => Some(root.join(relative)),
        Err(message) => {
            fields.issue(key, message);
            None
        }
    }
}

fn relative_manifest_path(path: &str) -> Result<Option<&str>, &'static str> {
    if path.is_empty() {
        return Ok(None);
    }
    let Some(relative) = path.strip_prefix("./") else {
        return Err("the path must start with `./` relative to the plugin root");
    };
    if relative.is_empty() {
        return Err("the path must not be `./`");
    }
    if relative
        .split(['/', '\\'])
        .any(|component| component == "..")
    {
        return Err("the path must not contain `..`");
    }
    let has_drive = matches!(relative.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic());
    if relative.starts_with(['/', '\\']) || has_drive {
        return Err("the path must stay within the plugin root");
    }
    Ok(Some(relative))
}
