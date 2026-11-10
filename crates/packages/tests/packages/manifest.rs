// Derived from OpenAI Codex (https://github.com/openai/codex), the tests
// of `codex-rs/core-plugins/src/manifest.rs` and
// `codex-rs/core-plugins/src/agent_plugin_manifest_tests.rs` at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a field of the wrong type is dropped with
// an issue where Codex fails the manifest; apps and the URI conventions are
// not ported; this project's code blocks and the path rules are added.

use std::path::Path;

use hanafubuki_agent_packages::{
    load_manifest, HooksDeclaration, ManifestFormat, ManifestInterface, McpServersDeclaration,
    PackageManifest, RuntimeKind, WorkspaceAccess, AGENT_PLUGINS_SCHEMA,
};
use serde_json::json;

use super::{fixtures, write};

const MAX_DEFAULT_PROMPT_CHARS: usize = 512;

fn write_manifest(root: &Path, version: Option<&str>, interface: &str) {
    let version = version
        .map(|version| format!("\"version\": \"{version}\","))
        .unwrap_or_default();
    write(
        &root.join(".codex-plugin/plugin.json"),
        &format!(r#"{{"name": "demo-plugin", {version} "interface": {interface}}}"#),
    );
}

fn load(root: &Path) -> PackageManifest {
    load_manifest(root).unwrap().manifest
}

fn interface(root: &Path) -> ManifestInterface {
    load(root).interface.unwrap()
}

#[test]
fn plugin_interface_accepts_legacy_default_prompt_string() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_manifest(
        &root,
        None,
        r#"{"displayName": "Demo Plugin", "defaultPrompt": "  Summarize   my inbox  "}"#,
    );

    assert_eq!(
        interface(&root).default_prompt,
        Some(vec!["Summarize my inbox".to_string()])
    );
}

#[test]
fn plugin_interface_normalizes_default_prompt_array() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    let at_limit = "é".repeat(MAX_DEFAULT_PROMPT_CHARS);
    let too_long = "x".repeat(MAX_DEFAULT_PROMPT_CHARS + 1);
    write_manifest(
        &root,
        None,
        &json!({
            "displayName": "Demo Plugin",
            "defaultPrompt": [
                format!(" {at_limit} "), 123, too_long, "   ",
                "Draft the reply  ", "Find   my next action", "Archive old mail"
            ]
        })
        .to_string(),
    );

    let loaded = load_manifest(&root).unwrap();

    assert_eq!(
        loaded.manifest.interface.unwrap().default_prompt,
        Some(vec![
            at_limit,
            "Draft the reply".to_string(),
            "Find my next action".to_string(),
        ])
    );
    assert_eq!(loaded.issues.len(), 4, "{:?}", loaded.issues);
}

#[test]
fn plugin_interface_ignores_invalid_default_prompt_shape() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_manifest(
        &root,
        None,
        r#"{"displayName": "Demo Plugin", "defaultPrompt": {"text": "Summarize my inbox"}}"#,
    );

    assert_eq!(interface(&root).default_prompt, None);
}

#[test]
fn plugin_interface_reads_dark_logo_path() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_manifest(&root, None, r#"{"logoDark": "./assets/logo-dark.svg"}"#);

    assert_eq!(
        interface(&root).logo_dark,
        Some(root.join("assets/logo-dark.svg"))
    );
}

#[test]
fn plugin_manifest_reads_trimmed_version() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_manifest(
        &root,
        Some(" 1.2.3-beta+7 "),
        r#"{"displayName": "Demo Plugin"}"#,
    );

    assert_eq!(load(&root).version, Some("1.2.3-beta+7".to_string()));
}

#[test]
fn plugin_manifest_reads_keywords() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"name": "demo-plugin", "keywords": ["api-key", "developer tools"]}"#,
    );

    assert_eq!(load(&root).keywords, vec!["api-key", "developer tools"]);
}

#[test]
fn plugin_manifest_uses_alternate_discoverable_path() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join(".claude-plugin/plugin.json"),
        r#"{"name": "demo-plugin", "version": " 2.0.0 ", "interface": {"displayName": "Fallback Plugin"}}"#,
    );

    let loaded = load_manifest(&root).unwrap();

    assert_eq!(loaded.format, ManifestFormat::Claude);
    assert_eq!(loaded.manifest.version, Some("2.0.0".to_string()));
    assert_eq!(loaded.manifest.display_name(), "Fallback Plugin");
}

#[test]
fn a_manifest_without_a_name_takes_its_directory_name() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("unnamed");
    write(&root.join(".claude-plugin/plugin.json"), r#"{"name": " "}"#);

    assert_eq!(load(&root).name, "unnamed");
}

#[test]
fn component_paths_resolve_below_the_package_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "demo-plugin",
  "skills": ["./skills", "./more-skills"],
  "mcpServers": "./.mcp.json",
  "hooks": "./hooks.json",
  "commands": "./cmds",
  "interface": {"displayName": "Demo Plugin", "composerIcon": "./assets/icon.svg"}
}"#,
    );

    let manifest = load(&root);

    assert_eq!(
        manifest.paths.skills,
        vec![root.join("skills"), root.join("more-skills")]
    );
    assert_eq!(
        manifest.paths.mcp_servers,
        Some(McpServersDeclaration::Path(root.join(".mcp.json")))
    );
    assert_eq!(
        manifest.paths.hooks,
        Some(HooksDeclaration::Paths(vec![root.join("hooks.json")]))
    );
    assert_eq!(manifest.paths.commands, vec![root.join("cmds")]);
    assert_eq!(
        manifest.interface,
        Some(ManifestInterface {
            display_name: Some("Demo Plugin".to_string()),
            composer_icon: Some(root.join("assets/icon.svg")),
            ..ManifestInterface::default()
        })
    );
}

#[test]
fn inline_servers_and_hook_files_are_kept_as_written() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "demo-plugin",
  "mcpServers": {"counter": {"type": "http", "url": "https://example.com/mcp"}},
  "hooks": [{"hooks": {"Stop": []}}]
}"#,
    );

    let manifest = load(&root);

    let Some(McpServersDeclaration::Inline(servers)) = manifest.paths.mcp_servers else {
        panic!("inline servers");
    };
    assert_eq!(servers["counter"]["url"], json!("https://example.com/mcp"));
    let Some(HooksDeclaration::Inline(files)) = manifest.paths.hooks else {
        panic!("inline hooks");
    };
    assert_eq!(files.len(), 1);
}

#[test]
fn a_path_that_could_leave_the_package_is_ignored_with_an_issue() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "demo-plugin",
  "skills": ["skills", "./", "./a/../../b", "./a\\..\\b", ".//etc", "./C:/x", "./kept"],
  "interface": {"logo": "assets/logo.svg"}
}"#,
    );

    let loaded = load_manifest(&root).unwrap();

    assert_eq!(loaded.manifest.paths.skills, vec![root.join("kept")]);
    assert_eq!(loaded.manifest.interface, None);
    assert_eq!(loaded.issues.len(), 7, "{:?}", loaded.issues);
}

#[test]
fn a_field_of_the_wrong_type_costs_only_that_field() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join(".claude-plugin/plugin.json"),
        r#"{"name": "demo-plugin", "version": 2, "description": ["x"], "keywords": ["a", 1],
            "skills": 7, "mcpServers": true, "hooks": [1, "./h.json"], "interface": "wide"}"#,
    );

    let loaded = load_manifest(&root).unwrap();

    assert_eq!(loaded.manifest.name, "demo-plugin");
    assert_eq!(loaded.manifest.version, None);
    assert_eq!(loaded.manifest.description, None);
    assert_eq!(loaded.manifest.keywords, vec!["a"]);
    assert_eq!(loaded.manifest.paths, Default::default());
    assert_eq!(loaded.manifest.interface, None);
    assert_eq!(loaded.issues.len(), 7, "{:?}", loaded.issues);
}

#[test]
fn a_manifest_that_is_not_an_object_fails() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(&root.join(".codex-plugin/plugin.json"), "[]");

    assert!(load_manifest(&root).is_err());
}

fn write_agent_plugin_manifest(root: &Path, extra_fields: &str) {
    write(
        &root.join("plugin.json"),
        &format!(r#"{{"$schema": "{AGENT_PLUGINS_SCHEMA}", "name": "demo-plugin"{extra_fields}}}"#),
    );
}

#[test]
fn agent_plugins_use_portable_metadata_and_fixed_components() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_agent_plugin_manifest(
        &root,
        r#", "version": "release-2026-07", "description": "Portable demo",
            "author": {"name": "Portable Author"}, "homepage": "https://example.com/plugin",
            "keywords": ["portable"]"#,
    );

    let loaded = load_manifest(&root).unwrap();
    let manifest = loaded.manifest;

    assert_eq!(loaded.format, ManifestFormat::AgentPlugins);
    assert_eq!(manifest.name, "demo-plugin");
    assert_eq!(manifest.version.as_deref(), Some("release-2026-07"));
    assert_eq!(manifest.description.as_deref(), Some("Portable demo"));
    assert_eq!(manifest.keywords, vec!["portable"]);
    assert_eq!(manifest.paths.skills, vec![root.join("skills")]);
    assert_eq!(
        manifest.paths.mcp_servers,
        Some(McpServersDeclaration::Path(root.join("mcp.json")))
    );
    assert_eq!(
        manifest.interface,
        Some(ManifestInterface {
            display_name: Some("demo-plugin".to_string()),
            short_description: Some("Portable demo".to_string()),
            long_description: Some("Portable demo".to_string()),
            developer_name: Some("Portable Author".to_string()),
            category: Some("Other".to_string()),
            website_url: Some("https://example.com/plugin".to_string()),
            ..ManifestInterface::default()
        })
    );
}

#[test]
fn agent_plugins_do_not_invent_optional_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_agent_plugin_manifest(&root, "");

    let manifest = load(&root);

    assert_eq!(manifest.version, None);
    assert_eq!(manifest.description, None);
    let interface = manifest.interface.unwrap();
    assert_eq!(interface.display_name.as_deref(), Some("demo-plugin"));
    assert_eq!(interface.short_description, None);
    assert_eq!(interface.developer_name, None);
    assert_eq!(interface.website_url, None);
}

#[test]
fn agent_plugins_normalize_empty_optional_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_agent_plugin_manifest(
        &root,
        r#", "version": "", "description": " ", "author": {"name": " ", "email": ""},
            "homepage": "", "keywords": [""]"#,
    );

    let manifest = load(&root);

    assert_eq!(manifest.version, None);
    assert_eq!(manifest.description, None);
    assert_eq!(manifest.keywords, vec![""]);
    let interface = manifest.interface.unwrap();
    assert_eq!(interface.short_description, None);
    assert_eq!(interface.developer_name, None);
    assert_eq!(interface.website_url, None);
}

#[test]
fn agent_plugins_accept_dotted_names_and_ignore_extensions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("acme-tools");
    write(
        &root.join("plugin.json"),
        &format!(
            r#"{{"$schema":"{AGENT_PLUGINS_SCHEMA}","name":"acme.tools",
                "extensions":{{"com.example.client":{{"future":true}},"com.example.unimplemented":"ignored"}}}}"#
        ),
    );
    assert_eq!(load(&root).name, "acme.tools");

    write(
        &root.join("plugin.json"),
        &format!(
            r#"{{"$schema":"{AGENT_PLUGINS_SCHEMA}","name":"acme.tools","extensions":false}}"#
        ),
    );
    assert_eq!(load(&root).name, "acme.tools");
}

#[test]
fn agent_plugins_reject_a_bad_name_and_drop_wrongly_typed_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write(
        &root.join("plugin.json"),
        &format!(
            r#"{{"$schema":"{AGENT_PLUGINS_SCHEMA}","name":"{}"}}"#,
            "a".repeat(65)
        ),
    );
    assert!(load_manifest(&root).is_err());
    write(
        &root.join("plugin.json"),
        &format!(r#"{{"$schema":"{AGENT_PLUGINS_SCHEMA}","name":"Demo"}}"#),
    );
    assert!(load_manifest(&root).is_err());

    // Codex fails these manifests; here only the field is dropped.
    for extra in [
        r#", "homepage": 42"#,
        r#", "version": null"#,
        r#", "author": {"name": null}"#,
        r#", "unknown": true"#,
    ] {
        write_agent_plugin_manifest(&root, extra);
        let loaded = load_manifest(&root).unwrap();
        assert_eq!(loaded.manifest.name, "demo-plugin", "{extra}");
        assert_eq!(loaded.manifest.version, None, "{extra}");
    }
}

#[test]
fn agent_plugins_legacy_codex_overlay_keeps_portable_components_fixed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_agent_plugin_manifest(
        &root,
        r#", "version": "portable-version", "description": "Portable description""#,
    );
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"name": "different-name", "version": "9.9.9", "description": "Codex description",
            "skills": [], "mcpServers": null, "hooks": "./hooks.json",
            "interface": {"displayName": "Codex Demo"}}"#,
    );

    let manifest = load(&root);

    assert_eq!(manifest.name, "demo-plugin");
    assert_eq!(manifest.version.as_deref(), Some("portable-version"));
    assert_eq!(
        manifest.description.as_deref(),
        Some("Portable description")
    );
    assert_eq!(manifest.paths.skills, vec![root.join("skills")]);
    assert_eq!(
        manifest.paths.mcp_servers,
        Some(McpServersDeclaration::Path(root.join("mcp.json")))
    );
    assert_eq!(
        manifest.paths.hooks,
        Some(HooksDeclaration::Paths(vec![root.join("hooks.json")]))
    );
    assert_eq!(manifest.display_name(), "Codex Demo");
}

#[test]
fn agent_plugins_inline_openai_extension_precedes_legacy_overlay() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo-plugin");
    write_agent_plugin_manifest(
        &root,
        r#", "extensions": {"com.openai": {"interface": {"displayName": "Inline Codex"}}}"#,
    );
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"interface":{"displayName":"Legacy Codex"}}"#,
    );

    assert_eq!(load(&root).display_name(), "Inline Codex");
}

#[test]
fn hanafubuki_manifests_carry_their_code_blocks() {
    let root = fixtures().join("marketplace/plugins/hanafubuki-demo");

    let loaded = load_manifest(&root).unwrap();
    let code = loaded.manifest.code.unwrap();

    assert_eq!(loaded.format, ManifestFormat::Hanafubuki);
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.manifest.paths.skills, vec![root.join("skills")]);
    let runtime = code.runtime.unwrap();
    assert_eq!(runtime.kind, RuntimeKind::Process);
    assert_eq!(runtime.command, vec!["./bin/jira-helper"]);
    assert_eq!(runtime.protocol.as_deref(), Some("kardia-guest/2"));
    assert_eq!(code.contributes.tools[0].name, "jira_search");
    assert_eq!(code.contributes.context_sections[0].kind, "jira_board");
    assert_eq!(code.contributes.observers, vec!["turn_completed"]);
    assert!(code.permissions.network);
    assert_eq!(code.permissions.workspace, WorkspaceAccess::Read);
    assert_eq!(code.permissions.env, vec!["JIRA_TOKEN"]);
    assert!(code.config_schema.is_some());
}

#[test]
fn a_bad_code_entry_costs_only_itself() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("guest");
    write(
        &root.join(".hanafubuki-plugin/plugin.json"),
        r#"{"name": "guest",
            "runtime": {"type": "native", "command": ["./bin/guest"]},
            "contributes": {
              "tools": [{"name": "ok", "inputSchema": {"type": "object"}}, {"name": "no-schema"}, 3],
              "approvalRules": [],
              "contextSections": [{"renderer": "markdown"}]
            },
            "permissions": {"workspace": "everything", "network": "yes"}}"#,
    );

    let loaded = load_manifest(&root).unwrap();
    let code = loaded.manifest.code.unwrap();

    assert_eq!(code.runtime, None);
    let tools = code
        .contributes
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tools, vec!["ok"]);
    assert!(code.contributes.context_sections.is_empty());
    assert_eq!(code.permissions.workspace, WorkspaceAccess::Read);
    assert!(!code.permissions.network);
    assert_eq!(loaded.issues.len(), 7, "{:?}", loaded.issues);
}

#[test]
fn only_this_projects_format_carries_code() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("demo");
    write(
        &root.join(".codex-plugin/plugin.json"),
        r#"{"name": "demo", "runtime": {"type": "process", "command": ["./x"]}}"#,
    );

    assert_eq!(load(&root).code, None);
}
