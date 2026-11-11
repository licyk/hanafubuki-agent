//! An installed plugin's roles (site/content/docs/roles.en.mdx, "Plugins'
//! roles"): the role files of a package in this project's format, read as
//! the user's are, and the agent files of a Claude Code package, each named
//! `<plugin>:<role>` after the package id's plugin name. What their reading
//! leaves out is the package's issue; resolving them is the catalog's.

use std::path::{Path, PathBuf};

use hanafubuki_agent_config::{is_role_name, read_role_files, RoleDefinition, RoleOrigin};
use hanafubuki_agent_packages::{read_claude_agents, ClaudeAgent, LoadedManifest, ManifestFormat};

const DEFAULT_AGENTS: &str = "agents";

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
