// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/engine/discovery.rs` (a plugin's hook sources after the
// user's, their keys by the plugin and the file's path in it, the four
// environment variables), `codex-rs/hooks/src/engine/plugin_trust.rs` (a
// plugin's hook trusted by its definition and its package's content hash,
// the package hashed again before each run) and
// `codex-rs/core-plugins/src/loader.rs` (`load_plugin_hooks`: the hook files
// and the manifest's inline ones) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the application resolves a package's hook
// files and passes them in; a listing hashes each package once, a run hashes
// it before the first of its hooks of the event; a package that cannot be
// hashed has its hooks listed as untrusted rather than left out; there is
// no legacy trust baseline to migrate.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};

use crate::discovery::{hooks_table, read_hooks_json, trust, TableOrigin};
use crate::state::HookState;
use crate::{HookEntry, HookSource, HookTrust};

/// An enabled installed plugin's hooks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginHookSource {
    /// `<plugin>@<marketplace>`.
    pub id: String,
    /// The installed package (`PLUGIN_ROOT`).
    pub root: PathBuf,
    /// The package's own data directory (`PLUGIN_DATA`).
    pub data: PathBuf,
    /// Its hook files, in the manifest's order.
    pub declarations: Vec<PluginHookDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginHookDeclaration {
    /// A `hooks.json` file in the package.
    File(PathBuf),
    /// A hook file written in the manifest at `manifest`, the `index`th of
    /// its `hooks`.
    Inline {
        manifest: PathBuf,
        index: usize,
        file: Map<String, Value>,
    },
}

/// The hook tables of `plugin`, each with where it was read. For a
/// listing, the package's content hash goes with them; a package that
/// cannot be hashed is a diagnostic and its hooks read as untrusted.
pub(crate) fn plugin_tables(
    plugin: &PluginHookSource,
    listing: bool,
    diagnostics: &mut Vec<String>,
) -> Vec<(TableOrigin, Map<String, Value>)> {
    let content_hash = if listing {
        match hanafubuki_agent_packages::content_hash(&plugin.root) {
            Ok(hash) => Some(hash),
            Err(error) => {
                diagnostics.push(format!(
                    "{}: the plugin's contents cannot be hashed, so its hooks are untrusted: {error}",
                    plugin.id
                ));
                Some(String::new())
            }
        }
    } else {
        None
    };
    let source = HookSource::Plugin {
        id: plugin.id.clone(),
        root: plugin.root.clone(),
        data: plugin.data.clone(),
    };
    let mut tables = Vec::new();
    for declaration in &plugin.declarations {
        let (relative, source_path, table) = match declaration {
            PluginHookDeclaration::File(path) => {
                if !path.is_file() {
                    diagnostics.push(format!(
                        "{}: the hook file {} does not exist",
                        plugin.id,
                        path.display()
                    ));
                    continue;
                }
                let Some((resolved, table)) = read_hooks_json(path, diagnostics) else {
                    continue;
                };
                let relative = relative_text(&plugin.root, path);
                (relative, resolved.native().to_path_buf(), table)
            }
            PluginHookDeclaration::Inline {
                manifest,
                index,
                file,
            } => {
                let relative = format!("plugin.json#hooks[{index}]");
                let source = format!("{}:{relative}", plugin.id);
                let Some(table) = hooks_table(file.clone(), &source, diagnostics) else {
                    continue;
                };
                (relative, manifest.clone(), table)
            }
        };
        tables.push((
            TableOrigin {
                source: source.clone(),
                key_source: format!("{}:{relative}", plugin.id),
                source_path,
                content_hash: content_hash.clone(),
            },
            table,
        ));
    }
    tables
}

/// A plugin's hook's trust and the hash it is shown with. Trusting it
/// recorded its definition's hash and its package's content hash; read for
/// a listing, the package must still hash the same, and read for a run,
/// the run checks that.
pub(crate) fn plugin_trust(
    state: &HookState,
    declaration_hash: &str,
    content_hash: Option<&str>,
) -> (HookTrust, String) {
    let declared = trust(state, declaration_hash);
    let trust = match (
        declared,
        content_hash,
        state.trusted_content_hash.as_deref(),
    ) {
        (HookTrust::Trusted, _, None) => HookTrust::Modified,
        (HookTrust::Trusted, Some(current), Some(trusted)) if current != trusted => {
            HookTrust::Modified
        }
        (trust, _, _) => trust,
    };
    let current_hash = match content_hash {
        Some(content) => format!("{declaration_hash}/{content}"),
        None => declaration_hash.to_string(),
    };
    (trust, current_hash)
}

/// The definition hash and the content hash a hook is trusted with, from
/// its listed `current_hash`.
pub(crate) fn trusted_hashes(hook: &HookEntry) -> (String, Option<String>) {
    match (&hook.source, hook.current_hash.split_once('/')) {
        (HookSource::Plugin { .. }, Some((declaration, content))) => {
            (declaration.to_string(), Some(content.to_string()))
        }
        _ => (hook.current_hash.clone(), None),
    }
}

/// The packages hashed during one run of an event's hooks.
#[derive(Default)]
pub(crate) struct PackageCheck {
    hashes: HashMap<PathBuf, Result<String, String>>,
}

impl PackageCheck {
    /// Why `hook` may not run, if its package changed since it was trusted.
    pub(crate) fn refusal(&mut self, hook: &HookEntry) -> Option<String> {
        let HookSource::Plugin { root, .. } = &hook.source else {
            return None;
        };
        let current = self.hashes.entry(root.clone()).or_insert_with(|| {
            hanafubuki_agent_packages::content_hash(root).map_err(|error| error.to_string())
        });
        match (current, hook.trusted_content_hash.as_deref()) {
            (Ok(current), Some(trusted)) if current == trusted => None,
            (Ok(_), _) => Some("plugin contents changed; hook approval required".to_string()),
            (Err(error), _) => Some(format!("the plugin's contents cannot be hashed: {error}")),
        }
    }
}

/// The variables a plugin's hook runs with: its package and its data
/// directory, under Codex's names and Claude Code's.
pub(crate) fn plugin_environment(source: &HookSource) -> HashMap<String, Option<String>> {
    let HookSource::Plugin { root, data, .. } = source else {
        return HashMap::new();
    };
    let (root, data) = (root.display().to_string(), data.display().to_string());
    HashMap::from([
        ("PLUGIN_ROOT".to_string(), Some(root.clone())),
        ("CLAUDE_PLUGIN_ROOT".to_string(), Some(root)),
        ("PLUGIN_DATA".to_string(), Some(data.clone())),
        ("CLAUDE_PLUGIN_DATA".to_string(), Some(data)),
    ])
}

/// `path` under `root`, `/`-separated: a hook key that survives an update
/// of the package, whose root names its version.
fn relative_text(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
