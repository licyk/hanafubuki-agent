// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/agent_plugin_manifest.rs` (the Agent Plugins
// 1.0.0 root `plugin.json`: its portable fields, its fixed components, the
// synthesized interface and the Codex overlay) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `$schema` and `name`, the identity, stay
// strict; any other field of the wrong type (an explicit `null` included)
// is dropped with an issue, where Codex fails the manifest; from the
// overlay only `hooks` and `interface` are taken, apps and the onboarding
// Skill not being ported; an unreadable overlay is an issue.

use std::path::Path;

use serde_json::{Map, Value};

use super::fields::{kind, non_empty_trimmed, Fields};
use super::legacy;
use super::probe::AGENT_PLUGINS_SCHEMA;
use super::{ManifestError, ManifestInterface, McpServersDeclaration, PackageManifest};

const CODEX_EXTENSION: &str = "com.openai";
const FIELDS: &[&str] = &[
    "$schema",
    "name",
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
    "extensions",
];
const MAX_NAME_BYTES: usize = 64;

pub(super) fn parse(
    root: &Path,
    object: &Map<String, Value>,
    overlay: Option<Result<Map<String, Value>, ManifestError>>,
    issues: &mut Vec<String>,
) -> Result<PackageManifest, ManifestError> {
    let schema = object.get("$schema").and_then(Value::as_str);
    if schema != Some(AGENT_PLUGINS_SCHEMA) {
        return Err(ManifestError::Invalid(format!(
            "unsupported Agent Plugins schema {}; supported: `{AGENT_PLUGINS_SCHEMA}`",
            schema.map_or_else(|| "(none)".to_string(), |schema| format!("`{schema}`"))
        )));
    }
    let name = match object.get("name") {
        Some(Value::String(name)) if is_valid_name(name) => name.clone(),
        _ => {
            return Err(ManifestError::Invalid(
                "the Agent Plugins name must be 1 to 64 lowercase letters, digits, dots or hyphens, starting and ending with a letter or digit".to_string(),
            ))
        }
    };
    for field in object
        .keys()
        .filter(|field| !FIELDS.contains(&field.as_str()))
    {
        issues.push(format!("ignoring {field}: not an Agent Plugins field"));
    }
    let mut fields = Fields::new(object, "", issues);
    let version = fields.string("version").and_then(non_empty_trimmed);
    let description = fields.string("description").and_then(non_empty_trimmed);
    let developer_name = fields.object("author").and_then(|author| {
        Fields::new(author, "author.", fields.issues())
            .string("name")
            .and_then(non_empty_trimmed)
    });
    let homepage = fields.string("homepage").and_then(non_empty_trimmed);
    fields.string("repository");
    fields.string("license");
    let keywords = fields.strings("keywords");
    let codex_extension = codex_extension(&mut fields);

    let mut manifest = PackageManifest {
        name: name.clone(),
        version,
        description: description.clone(),
        keywords,
        paths: super::ManifestPaths {
            skills: vec![root.join("skills")],
            mcp_servers: Some(McpServersDeclaration::Path(root.join("mcp.json"))),
            hooks: None,
            commands: Vec::new(),
            agents: Vec::new(),
        },
        interface: Some(ManifestInterface {
            display_name: Some(name),
            short_description: description.clone(),
            long_description: description,
            developer_name,
            category: Some("Other".to_string()),
            website_url: homepage,
            ..ManifestInterface::default()
        }),
        code: None,
    };
    let overlay = match (codex_extension, overlay) {
        (Some(extension), _) => Some(extension.clone()),
        (None, Some(Ok(overlay))) => Some(overlay),
        (None, Some(Err(error))) => {
            issues.push(format!("ignoring the .codex-plugin overlay: {error}"));
            None
        }
        (None, None) => None,
    };
    if let Some(overlay) = overlay {
        let overlay = legacy::parse(root, &overlay, issues);
        manifest.paths.hooks = overlay.paths.hooks;
        if overlay.interface.is_some() {
            manifest.interface = overlay.interface;
        }
    }
    Ok(manifest)
}

/// `extensions["com.openai"]`, which overlays the manifest as Codex's
/// `.codex-plugin/plugin.json` would; other extensions are ignored.
fn codex_extension<'a>(fields: &mut Fields<'a>) -> Option<&'a Map<String, Value>> {
    let extension = fields.object("extensions")?.get(CODEX_EXTENSION)?;
    match extension {
        Value::Object(extension) => Some(extension),
        other => {
            let message = format!("expected an object, found {}", kind(other));
            fields.issue(&format!("extensions[\"{CODEX_EXTENSION}\"]"), &message);
            None
        }
    }
}

fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.contains("--")
        && !name.contains("..")
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(byte))
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}
