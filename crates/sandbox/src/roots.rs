// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/protocol/src/permissions.rs`
// (`default_read_only_subpaths_for_writable_root`, what stays read-only
// under a writable root) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: writable roots are resolved in full (a
// symlinked root grants its target, as Landlock does) and one that does not
// resolve is left out; the read-only names are `.git`, `.agents` and the
// project's `.hanafubuki-agent`, and only when they exist.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

/// Kept read-only under a writable root, when they exist there: the
/// repository (its hooks run outside the sandbox), the repository's Skills,
/// and the project's configuration, which could choose a weaker sandbox.
const READ_ONLY_NAMES: [&str; 3] = [".git", ".agents", ".hanafubuki-agent"];

/// A writable root, resolved, with the paths under it that stay read-only.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedRoot {
    pub(crate) root: ResolvedPath,
    pub(crate) read_only: Vec<ResolvedPath>,
}

/// `roots` resolved (`/tmp` is `/private/tmp` on macOS, which matches the
/// paths a command reaches), each once, with what stays read-only under
/// it. A root that does not resolve, one that does not exist, is left out
/// and so stays unwritable.
pub(crate) fn resolve_roots(roots: &[PathBuf]) -> Vec<ResolvedRoot> {
    let mut resolved: Vec<ResolvedPath> = Vec::new();
    for root in roots {
        let Ok(root) = ResolvedPath::resolve(root) else {
            continue;
        };
        if !resolved.iter().any(|seen| seen.key() == root.key()) {
            resolved.push(root);
        }
    }
    resolved
        .into_iter()
        .map(|root| ResolvedRoot {
            read_only: read_only_under(&root),
            root,
        })
        .collect()
}

/// The [`READ_ONLY_NAMES`] that exist directly under `root`; for a `.git`
/// file (a linked worktree's or a submodule's), the repository directory it
/// points to as well.
fn read_only_under(root: &ResolvedPath) -> Vec<ResolvedPath> {
    let mut paths = Vec::new();
    for name in READ_ONLY_NAMES {
        let Ok(path) = ResolvedPath::resolve_child(root, OsStr::new(name)) else {
            continue;
        };
        let Ok(metadata) = std::fs::symlink_metadata(path.system()) else {
            continue;
        };
        if name == ".git" && metadata.is_file() {
            paths.extend(git_directory(root, &path));
        }
        paths.push(path);
    }
    paths
}

/// The directory a `.git` file's `gitdir: <path>` names, relative to the
/// root it is in.
fn git_directory(root: &ResolvedPath, dot_git: &ResolvedPath) -> Option<ResolvedPath> {
    let contents = std::fs::read_to_string(dot_git.system()).ok()?;
    let (prefix, target) = contents.trim().split_once(':')?;
    let target = target.trim();
    if prefix.trim() != "gitdir" || target.is_empty() {
        return None;
    }
    let requested = root.join_request(Path::new(target)).ok()?;
    ResolvedPath::resolve(requested).ok()
}
