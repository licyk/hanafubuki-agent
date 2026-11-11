//! An installed plugin's roles (site/content/docs/roles.en.mdx, "Plugins'
//! roles"): the role files of a package in this project's format, read as
//! the user's are, and the agent files of a Claude Code package, each named
//! `<plugin>:<role>` after the package id's plugin name. What their reading
//! leaves out is the package's issue; resolving them is the catalog's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hanafubuki_agent_config::{
    is_role_name, read_role_files, AgentSettings, RoleDefinition, RoleOrigin,
};
use hanafubuki_agent_packages::{
    read_claude_agents, ClaudeAgent, LoadedManifest, ManifestFormat, PackageStore, PluginId,
};

use crate::roles::AgentPackageRoles;

const DEFAULT_AGENTS: &str = "agents";

/// The roles of the packages installed under `plugins_root` that
/// `settings` leave on (none in `safe_mode`), as a started Agent's catalog
/// takes them, and what reading them left out, each issue naming its
/// package. Read without opening the store: nothing is swept, quarantined,
/// made or written, so a command may read them beside a running Agent.
pub(crate) fn read_installed_roles(
    plugins_root: &Path,
    settings: &AgentSettings,
    safe_mode: bool,
) -> (Vec<AgentPackageRoles>, Vec<String>) {
    let mut installed = PackageStore::read_installed(plugins_root);
    installed.sort_by_key(|package| package.id.key());
    let mut packages: Vec<(PluginId, Vec<RoleDefinition>, Vec<String>)> = installed
        .into_iter()
        .filter(|package| !safe_mode && settings.package_enabled(&package.id))
        .filter_map(|package| {
            let manifest = package.manifest.as_ref()?;
            let mut issues = Vec::new();
            let roles = roles(
                &package.root,
                &package.id.plugin_name,
                manifest,
                &mut issues,
            );
            Some((package.id, roles, issues))
        })
        .collect();
    resolve_names(
        packages
            .iter_mut()
            .map(|(id, roles, issues)| (&*id, roles, issues)),
    );
    let issues = packages
        .iter()
        .flat_map(|(id, _, issues)| {
            issues
                .iter()
                .map(move |issue| format!("plugin `{id}`: {issue}"))
        })
        .collect();
    let roles = packages
        .into_iter()
        .filter(|(_, roles, _)| !roles.is_empty())
        .map(|(id, definitions, _)| AgentPackageRoles {
            id: id.key(),
            definitions,
        })
        .collect();
    (roles, issues)
}

/// Keeps the roles of a plugin name (`<plugin>:…`) to one of the packages
/// that are on, given by id with their roles and issues: the first with
/// roles; another of that plugin name, from another marketplace,
/// contributes none, and that is its issue.
pub(super) fn resolve_names<'a>(
    packages: impl Iterator<
        Item = (
            &'a PluginId,
            &'a mut Vec<RoleDefinition>,
            &'a mut Vec<String>,
        ),
    >,
) {
    let mut owners = BTreeMap::<String, String>::new();
    for (id, roles, issues) in packages.filter(|(_, roles, _)| !roles.is_empty()) {
        let plugin = &id.plugin_name;
        match owners.get(plugin) {
            Some(owner) => {
                issues.push(format!(
                    "its roles are left out: `{owner}` comes first, and its roles are named \
                     `{plugin}:…` already"
                ));
                roles.clear();
            }
            None => {
                owners.insert(plugin.clone(), id.key());
            }
        }
    }
}

/// The roles of the package at `root`, of the plugin name `plugin`: those
/// of the manifest's `agents`, else of `agents/`. A Codex, Cursor or Agent
/// Plugins package has none.
pub(super) fn roles(
    root: &Path,
    plugin: &str,
    manifest: &LoadedManifest,
    issues: &mut Vec<String>,
) -> Vec<RoleDefinition> {
    let declared = &manifest.manifest.paths.agents;
    if !matches!(
        manifest.format,
        ManifestFormat::Hanafubuki | ManifestFormat::Claude
    ) {
        if !declared.is_empty() {
            issues.push(format!(
                "its `agents` are not read: a {} package has no roles",
                manifest.format.as_str()
            ));
        }
        return Vec::new();
    }
    let paths = if declared.is_empty() {
        let default = root.join(DEFAULT_AGENTS);
        if !default.is_dir() {
            return Vec::new();
        }
        vec![default]
    } else {
        declared.clone()
    };
    let (definitions, read_issues) = match manifest.format {
        ManifestFormat::Claude => claude_roles(&paths),
        _ => {
            let files = read_role_files(&paths);
            (files.definitions, files.issues)
        }
    };
    issues.extend(
        read_issues
            .into_iter()
            .map(|issue| format!("roles: {issue}")),
    );
    definitions
        .into_iter()
        .map(|mut definition| {
            definition.name = format!("{plugin}:{}", definition.name);
            let name = &definition.name;
            issues.extend(
                definition
                    .issues
                    .drain(..)
                    .map(|issue| format!("role `{name}`: {issue}")),
            );
            definition
        })
        .collect()
}

fn claude_roles(paths: &[PathBuf]) -> (Vec<RoleDefinition>, Vec<String>) {
    let read = read_claude_agents(paths);
    let mut issues = read.issues;
    let definitions = read
        .agents
        .into_iter()
        .filter_map(|agent| claude_role(agent).map_err(|issue| issues.push(issue)).ok())
        .collect();
    (definitions, issues)
}

/// The role `agent` is, named by its `name` when that can be a role's name,
/// else by its file's stem; the error says why it has no name.
fn claude_role(agent: ClaudeAgent) -> Result<RoleDefinition, String> {
    let ClaudeAgent {
        file,
        name,
        description,
        instructions,
        tools,
        denied_tools,
        permission_mode_cap,
        unusable,
        mut issues,
    } = agent;
    let named = name.filter(|name| {
        let usable = is_role_name(name);
        if !usable {
            issues.push(format!(
                "`name` `{name}` cannot be a role's name (it holds a space, a control \
                 character or `:`); the file's name is used"
            ));
        }
        usable
    });
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| is_role_name(stem))
        .map(str::to_string);
    let Some(name) = named.or(stem) else {
        return Err(format!(
            "{} has no role name: neither its `name` nor its file's name can be one; it is \
             ignored",
            file.display()
        ));
    };
    let mut definition = RoleDefinition::empty(name, RoleOrigin::File { path: file });
    definition.description = description;
    definition.instructions = instructions;
    definition.tools_allow = tools;
    definition.tools_deny = denied_tools;
    definition.permission_mode_cap = permission_mode_cap;
    definition.unusable = unusable;
    definition.issues = issues;
    Ok(definition)
}
