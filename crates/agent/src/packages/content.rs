// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/loader.rs` (`load_plugin`: a package's Skill
// roots, its migrated commands' root, its MCP servers with the user's
// per-plugin overlay, its hooks, Claude Code's default locations; an Agent
// Plugins package's Skills its direct children and its hooks never read) at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a package's components become the Skill,
// MCP and hook sources of this Agent's domains; what cannot be read is an
// issue of the package; no manifest cache, no apps, no remote plugins; the
// package's data directory is made before its content is read.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hanafubuki_agent_hooks::{PluginHookDeclaration, PluginHookSource};
use hanafubuki_agent_mcp::{agent_plugin_mcp_servers, plugin_mcp_servers, PluginMcpServers};
use hanafubuki_agent_packages::{
    migrated_command_skills_root, HooksDeclaration, InstalledPackage, LoadedManifest,
    ManifestFormat, McpServersDeclaration, PluginMcpServerSettings,
};
use hanafubuki_agent_skills::PluginSkillRoot;
use serde_json::Value;

use super::LoadedPackage;

const DEFAULT_SKILLS: &str = "skills";
const DEFAULT_MCP: &str = ".mcp.json";
const DEFAULT_HOOKS: &str = "hooks/hooks.json";

/// `package` as the Agent runs it: when `enabled`, its Skill roots, its MCP
/// servers with the user's `overlay` applied, and its hooks.
pub(super) fn load(
    package: &InstalledPackage,
    enabled: bool,
    overlay: &BTreeMap<String, PluginMcpServerSettings>,
) -> LoadedPackage {
    let manifest = package.manifest.as_ref();
    let mut loaded = LoadedPackage {
        id: package.id.clone(),
        version: package.version.clone(),
        format: manifest.map(|manifest| manifest.format),
        enabled,
        display_name: manifest.map_or_else(
            || package.id.plugin_name.clone(),
            |manifest| manifest.manifest.display_name().to_string(),
        ),
        namespace: manifest.map_or_else(
            || package.id.plugin_name.clone(),
            |manifest| manifest.manifest.name.clone(),
        ),
        description: manifest.and_then(|manifest| manifest.manifest.description.clone()),
        skill_roots: Vec::new(),
        mcp_servers: Vec::new(),
        hooks: None,
        issues: package.issues.clone(),
    };
    let Some(manifest) = manifest.filter(|_| enabled) else {
        return loaded;
    };
    loaded.issues.extend(manifest.issues.iter().cloned());
    if let Err(error) = fs::create_dir_all(&package.data_root) {
        loaded.issues.push(format!(
            "its data directory {} was not made: {error}",
            package.data_root.display()
        ));
    }
    loaded.skill_roots = skill_roots(package, manifest, &loaded.namespace);
    let servers = mcp_servers(package, manifest);
    loaded.issues.extend(
        servers
            .errors
            .iter()
            .map(|error| format!("MCP server `{}` is left out: {}", error.name, error.message)),
    );
    loaded.mcp_servers = servers
        .servers
        .into_iter()
        .filter_map(|server| apply_overlay(server, overlay, &mut loaded.issues))
        .collect();
    loaded.hooks = hooks(package, manifest, &mut loaded.issues);
    loaded
}

/// Its `skills` (the manifest's, else `skills/`) and, but for an Agent
/// Plugins package, the Skills its commands became.
fn skill_roots(
    package: &InstalledPackage,
    manifest: &LoadedManifest,
    namespace: &str,
) -> Vec<PluginSkillRoot> {
    let agent_plugins = manifest.format == ManifestFormat::AgentPlugins;
    let mut paths = if manifest.manifest.paths.skills.is_empty() {
        let default = package.root.join(DEFAULT_SKILLS);
        default.is_dir().then_some(default).into_iter().collect()
    } else {
        manifest.manifest.paths.skills.clone()
    };
    let migrated = migrated_command_skills_root(&package.root);
    if !agent_plugins && migrated.is_dir() {
        paths.push(migrated);
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| PluginSkillRoot {
            plugin: package.id.key(),
            namespace: namespace.to_string(),
            path,
            direct_children_only: agent_plugins,
        })
        .collect()
}

/// Its MCP servers: an Agent Plugins package's `mcp.json`, else the
/// manifest's `mcpServers` (a file or the servers themselves), else
/// `.mcp.json`.
fn mcp_servers(package: &InstalledPackage, manifest: &LoadedManifest) -> PluginMcpServers {
    let (root, data) = (package.root.as_path(), package.data_root.as_path());
    let path = match &manifest.manifest.paths.mcp_servers {
        Some(McpServersDeclaration::Inline(servers)) => {
            return plugin_mcp_servers(root, data, servers)
        }
        Some(McpServersDeclaration::Path(path)) => path.clone(),
        None => root.join(DEFAULT_MCP),
    };
    let declared = manifest.manifest.paths.mcp_servers.is_some()
        && manifest.format != ManifestFormat::AgentPlugins;
    let contents = match read_regular_file(&path) {
        Ok(Some(contents)) => contents,
        Ok(None) if !declared => return PluginMcpServers::default(),
        Ok(None) => return file_error(&path, "it does not exist"),
        Err(error) => return file_error(&path, &error),
    };
    let parsed = if manifest.format == ManifestFormat::AgentPlugins {
        agent_plugin_mcp_servers(root, data, &contents)
    } else {
        match serde_json::from_str::<Value>(&contents) {
            Ok(Value::Object(document)) => Ok(plugin_mcp_servers(root, data, &document)),
            Ok(_) => Err("it is not a JSON object".to_string()),
            Err(error) => Err(error.to_string()),
        }
    };
    parsed.unwrap_or_else(|error| file_error(&path, &error))
}

/// The contents of the regular file at `path`; `None` when there is none.
/// A link is refused: the package's files are its own.
fn read_regular_file(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() => Err("it is not a regular file".to_string()),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|error| error.to_string()),
    }
}

fn file_error(path: &Path, why: &str) -> PluginMcpServers {
    PluginMcpServers {
        servers: Vec::new(),
        errors: vec![hanafubuki_agent_mcp::PluginMcpServerError {
            name: "*".to_string(),
            message: format!("{} cannot be read: {why}", path.display()),
        }],
    }
}

/// `server` as the user's `[plugins."<id>".mcpServers.<name>]` restricts it.
fn apply_overlay(
    mut server: hanafubuki_agent_mcp::McpServerConfig,
    overlay: &BTreeMap<String, PluginMcpServerSettings>,
    issues: &mut Vec<String>,
) -> Option<hanafubuki_agent_mcp::McpServerConfig> {
    let Some(settings) = overlay.get(server.id.as_str()) else {
        return Some(server);
    };
    if let Some(enabled) = settings.enabled {
        server.enabled = enabled;
    }
    if let Some(tools) = &settings.enabled_tools {
        server.enabled_tools = tools.iter().cloned().collect();
    }
    if let Some(tools) = &settings.disabled_tools {
        server.disabled_tools = tools.iter().cloned().collect();
    }
    match server.validate() {
        Ok(()) => Some(server),
        Err(error) => {
            issues.push(format!(
                "MCP server `{}` is left out: its settings cannot be used: {error}",
                server.id.as_str()
            ));
            None
        }
    }
}

/// Its hooks: the manifest's files or inline files, else
/// `hooks/hooks.json`. An Agent Plugins package's hooks never run, as in
/// Codex.
fn hooks(
    package: &InstalledPackage,
    manifest: &LoadedManifest,
    issues: &mut Vec<String>,
) -> Option<PluginHookSource> {
    if manifest.format == ManifestFormat::AgentPlugins {
        if manifest.manifest.paths.hooks.is_some() {
            issues.push("an Agent Plugins package's hooks are not run".to_string());
        }
        return None;
    }
    let declarations = match &manifest.manifest.paths.hooks {
        Some(HooksDeclaration::Paths(paths)) => paths
            .iter()
            .cloned()
            .map(PluginHookDeclaration::File)
            .collect(),
        Some(HooksDeclaration::Inline(files)) => files
            .iter()
            .enumerate()
            .map(|(index, file)| PluginHookDeclaration::Inline {
                manifest: manifest.path.clone(),
                index,
                file: file.clone(),
            })
            .collect(),
        None => {
            let default = package.root.join(DEFAULT_HOOKS);
            if !default.is_file() {
                return None;
            }
            vec![PluginHookDeclaration::File(default)]
        }
    };
    Some(PluginHookSource {
        id: package.id.key(),
        root: package.root.clone(),
        data: package.data_root.clone(),
        declarations,
    })
}
