//! `cargo xtask check-dependencies`: the direction of the workspace's
//! dependencies, read from `cargo metadata`.
//!
//! Hanafubuki depends on this repository, never the reverse, so no workspace
//! package may depend on a `hanafubuki*` package that is not one of its own
//! members, from any source. Inside the workspace, each member may depend
//! only on the members its row of [`LAYERS`] allows, and the plugin
//! architecture's rules ([`rule_violations`]) hold whatever the table says:
//! the kernel depends on no plugin; a plugin depends on the plugin API and on
//! no other plugin, the kernel or the facade; the facade mounts every plugin
//! and alone reads the configuration; model-api cannot reach the context
//! section kinds; the plugin host depends on no member but the foundation;
//! and kardia stays inside the host, the plugin API, the kernel, the plugins
//! and the facade.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// Each member and the members it may depend on (normal, dev or build), by
/// layer from the bottom (site/content/docs/architecture.en.mdx, "Crates and
/// layers"). The table is
/// enforced once it has rows: every member then needs one.
const LAYERS: &[(&str, &[&str])] = &[
    // Neutral mechanisms.
    ("hanafubuki-agent-foundation", &[]),
    ("hanafubuki-agent-tool-contracts", &[]),
    ("hanafubuki-agent-process", &[FOUNDATION]),
    ("hanafubuki-agent-runtime", &[FOUNDATION]),
    (HOST, &[FOUNDATION]),
    // Domains.
    ("hanafubuki-agent-approval", &[FOUNDATION]),
    ("hanafubuki-agent-compression", &[]),
    ("hanafubuki-agent-skills", &[FOUNDATION]),
    ("hanafubuki-agent-web-search", &[FOUNDATION]),
    (MODEL_API, &[FOUNDATION, TOOL_CONTRACTS]),
    ("hanafubuki-agent-workspace-fs", &[APPROVAL, FOUNDATION]),
    (WORKTREE, &[FOUNDATION, PROCESS]),
    (PACKAGES, &[FOUNDATION, PROCESS]),
    (HOOKS, &[FOUNDATION, PACKAGES, PROCESS]),
    (SANDBOX, &[FOUNDATION]),
    (
        "hanafubuki-agent-execution",
        &[APPROVAL, FOUNDATION, PROCESS, SANDBOX],
    ),
    (
        "hanafubuki-agent-mcp",
        &[FOUNDATION, HOST, PROCESS, TOOL_CONTRACTS],
    ),
    (
        "hanafubuki-agent-memory",
        &[FOUNDATION, MODEL_API, TOOL_CONTRACTS],
    ),
    // The plugin API.
    (CONTRACTS, &[APPROVAL, FOUNDATION, HOST, TOOL_CONTRACTS]),
    // The kernel, and the history of its sessions.
    (
        KERNEL,
        &[
            APPROVAL,
            "hanafubuki-agent-compression",
            CONTRACTS,
            FOUNDATION,
            MODEL_API,
            SKILLS,
            TOOL_CONTRACTS,
        ],
    ),
    ("hanafubuki-agent-history", &[FOUNDATION, KERNEL]),
    // Plugins.
    (
        "hanafubuki-agent-plugin-execution",
        &[
            APPROVAL,
            CONTRACTS,
            "hanafubuki-agent-execution",
            FOUNDATION,
            SANDBOX,
            TOOL_CONTRACTS,
        ],
    ),
    (
        "hanafubuki-agent-plugin-mcp",
        &[
            APPROVAL,
            CONTRACTS,
            FOUNDATION,
            MCP,
            PROCESS,
            TOOL_CONTRACTS,
        ],
    ),
    (
        "hanafubuki-agent-plugin-web",
        &[
            APPROVAL,
            CONTRACTS,
            FOUNDATION,
            TOOL_CONTRACTS,
            "hanafubuki-agent-web-search",
        ],
    ),
    (
        "hanafubuki-agent-plugin-workspace-fs",
        &[
            APPROVAL,
            CONTRACTS,
            FOUNDATION,
            TOOL_CONTRACTS,
            "hanafubuki-agent-workspace-fs",
        ],
    ),
    (
        "hanafubuki-agent-plugin-worktree",
        &[CONTRACTS, FOUNDATION, PROCESS, WORKTREE],
    ),
    (
        "hanafubuki-agent-plugin-hooks",
        &[CONTRACTS, FOUNDATION, HOOKS, PROCESS, TOOL_CONTRACTS],
    ),
    // The configuration.
    (CONFIG, &[FOUNDATION, MCP, PACKAGES]),
    // The facade: the composition over everything below.
    (
        FACADE,
        &[
            APPROVAL,
            CONFIG,
            CONTRACTS,
            "hanafubuki-agent-execution",
            FOUNDATION,
            "hanafubuki-agent-history",
            HOOKS,
            HOST,
            KERNEL,
            MCP,
            "hanafubuki-agent-memory",
            PACKAGES,
            "hanafubuki-agent-plugin-execution",
            "hanafubuki-agent-plugin-hooks",
            "hanafubuki-agent-plugin-mcp",
            "hanafubuki-agent-plugin-web",
            "hanafubuki-agent-plugin-workspace-fs",
            "hanafubuki-agent-plugin-worktree",
            PROCESS,
            "hanafubuki-agent-runtime",
            SANDBOX,
            SKILLS,
            TOOL_CONTRACTS,
            "hanafubuki-agent-workspace-fs",
            WORKTREE,
        ],
    ),
    // The terminal interface over the facade, and the command line over both.
    ("hanafubuki-agent-tui", &[FACADE]),
    ("hanafubuki-agent-cli", &[FACADE, "hanafubuki-agent-tui"]),
    ("xtask", &[]),
];

const FACADE: &str = "hanafubuki-agent";
const KERNEL: &str = "hanafubuki-agent-kernel";
const CONTRACTS: &str = "hanafubuki-agent-contracts";
const CONFIG: &str = "hanafubuki-agent-config";
const HOST: &str = "hanafubuki-agent-host";
const FOUNDATION: &str = "hanafubuki-agent-foundation";
const MODEL_API: &str = "hanafubuki-agent-model-api";
const TOOL_CONTRACTS: &str = "hanafubuki-agent-tool-contracts";
const APPROVAL: &str = "hanafubuki-agent-approval";
const PROCESS: &str = "hanafubuki-agent-process";
const MCP: &str = "hanafubuki-agent-mcp";
const SKILLS: &str = "hanafubuki-agent-skills";
const WORKTREE: &str = "hanafubuki-agent-worktree";
const HOOKS: &str = "hanafubuki-agent-hooks";
const PACKAGES: &str = "hanafubuki-agent-packages";
const SANDBOX: &str = "hanafubuki-agent-sandbox";
const PLUGIN_PREFIX: &str = "hanafubuki-agent-plugin-";
const KARDIA: [&str; 4] = ["kardia", "kardia-bridge", "kardia-core", "kardia-macros"];

/// The part of `cargo metadata --no-deps` the check reads. Without the
/// dependency graph, `packages` holds exactly the workspace members.
#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub dependencies: Vec<Dependency>,
}

/// A declared dependency. `name` is the package name, even when the
/// dependency is renamed in the manifest.
#[derive(Debug, Deserialize)]
pub struct Dependency {
    pub name: String,
}

pub fn run(root: &Path) -> Result<(), String> {
    let metadata = metadata(root)?;
    let mut violations = violations(&metadata, LAYERS);
    violations.extend(rule_violations(&metadata));

    if violations.is_empty() {
        println!("Dependency check passed.");
        return Ok(());
    }
    Err(format!(
        "dependency violations:\n  {}",
        violations.join("\n  ")
    ))
}

/// Every dependency on a foreign `hanafubuki*` package, and, once `layers`
/// has rows, every internal dependency its member's row does not allow.
pub fn violations(metadata: &Metadata, layers: &[(&str, &[&str])]) -> Vec<String> {
    let members: BTreeSet<&str> = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let mut violations = Vec::new();
    for package in &metadata.packages {
        for dependency in &package.dependencies {
            let name = dependency.name.as_str();
            if name.starts_with("hanafubuki") && !members.contains(name) {
                violations.push(format!(
                    "{} depends on {name}, a Hanafubuki crate: this repository depends on no Hanafubuki crate",
                    package.name
                ));
            }
        }
    }
    if !layers.is_empty() {
        layer_violations(metadata, &members, layers, &mut violations);
    }
    violations
}

fn layer_violations(
    metadata: &Metadata,
    members: &BTreeSet<&str>,
    layers: &[(&str, &[&str])],
    violations: &mut Vec<String>,
) {
    for (name, _) in layers {
        if !members.contains(name) {
            violations.push(format!(
                "the layer table names {name}, which is not a workspace member"
            ));
        }
    }
    for package in &metadata.packages {
        let Some((_, allowed)) = layers.iter().find(|(name, _)| *name == package.name) else {
            violations.push(format!(
                "{} has no row in the layer table: add it with the members it may depend on",
                package.name
            ));
            continue;
        };
        for dependency in &package.dependencies {
            let name = dependency.name.as_str();
            if members.contains(name) && !allowed.contains(&name) {
                violations.push(format!(
                    "{} depends on {name}, which its layer does not allow",
                    package.name
                ));
            }
        }
    }
}

/// Every break of the plugin architecture's rules, whatever [`LAYERS`]
/// allows.
pub fn rule_violations(metadata: &Metadata) -> Vec<String> {
    let is_plugin = |name: &str| name.starts_with(PLUGIN_PREFIX);
    let plugins: Vec<&str> = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .filter(|name| is_plugin(name))
        .collect();
    let mut violations = Vec::new();
    for package in &metadata.packages {
        let name = package.name.as_str();
        let depends_on = |other: &str| {
            package
                .dependencies
                .iter()
                .any(|dependency| dependency.name == other)
        };
        let members = package
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .filter(|dependency| dependency.starts_with("hanafubuki"));
        if name == KERNEL {
            for plugin in plugins.iter().filter(|plugin| depends_on(plugin)) {
                violations.push(format!(
                    "{name} depends on the plugin {plugin}; plugins register through {CONTRACTS}"
                ));
            }
        }
        if is_plugin(name) {
            if !depends_on(CONTRACTS) {
                violations.push(format!("{name} does not depend on {CONTRACTS}"));
            }
            for forbidden in [KERNEL, FACADE] {
                if depends_on(forbidden) {
                    violations.push(format!(
                        "{name} depends on {forbidden}; a plugin reaches the Agent through {CONTRACTS}"
                    ));
                }
            }
            for plugin in plugins
                .iter()
                .filter(|plugin| **plugin != name && depends_on(plugin))
            {
                violations.push(format!(
                    "{name} depends on the plugin {plugin}; plugins share services through {CONTRACTS}"
                ));
            }
        }
        if name == FACADE {
            for plugin in plugins.iter().filter(|plugin| !depends_on(plugin)) {
                violations.push(format!(
                    "{name} does not depend on the plugin {plugin}; a plugin crate is mounted from the facade's manifest"
                ));
            }
        } else if depends_on(CONFIG) {
            violations.push(format!(
                "{name} depends on {CONFIG}; only the facade reads the configuration"
            ));
        }
        if name == MODEL_API && depends_on(CONTRACTS) {
            violations.push(format!(
                "{name} depends on {CONTRACTS}; context sections and their renderers are the plugins' (check-model-api-context)"
            ));
        }
        if name == HOST {
            for member in members.filter(|member| *member != FOUNDATION) {
                violations.push(format!(
                    "{name} depends on {member}; the plugin host depends only on kardia and {FOUNDATION}"
                ));
            }
        }
        if depends_on("kardia") {
            violations.push(format!(
                "{name} depends on the kardia facade; depend on kardia-core and kardia-macros"
            ));
        }
        let may_use_kardia = matches!(name, HOST | CONTRACTS | KERNEL | FACADE) || is_plugin(name);
        if !may_use_kardia {
            for kardia in KARDIA.iter().filter(|kardia| depends_on(kardia)) {
                violations.push(format!(
                    "{name} depends on {kardia}; only the plugin host, the plugin API, the kernel, the plugins and the facade may"
                ));
            }
        }
    }
    violations
}

fn metadata(root: &Path) -> Result<Metadata, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("`cargo metadata` output: {error}"))
}

#[cfg(test)]
mod tests;
