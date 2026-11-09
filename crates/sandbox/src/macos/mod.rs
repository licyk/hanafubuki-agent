// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/sandboxing/src/seatbelt.rs` (the Seatbelt policy of a command
// and `sandbox-exec`'s command line) and `codex-rs/protocol/src/
// permissions.rs` (`default_read_only_subpaths_for_writable_root`, what
// stays read-only under a writable root) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the two policies a command can have
// here, reading everything and writing nothing or under writable roots, so
// no restricted reads, unreadable globs, platform defaults, network proxy,
// Unix-socket allowlist or app-server socket protection; writable roots
// are resolved in full (a symlinked root grants its target, as Landlock
// does) and one that does not resolve is left out; the read-only names are
// `.git`, `.agents` and the project's `.hanafubuki-agent`, and only when
// they exist; `/dev/tty` is writable, as on Linux.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

/// Only this `sandbox-exec`, never one found on the PATH, which a command
/// could have changed; replacing this one already takes root.
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

const BASE_POLICY: &str = include_str!("seatbelt_base_policy.sbpl");
const NETWORK_POLICY: &str = include_str!("seatbelt_network_policy.sbpl");
const PREFERENCES_POLICY: &str = include_str!("seatbelt_preferences_policy.sbpl");
// System libcurl needs this service for TLS.
const TLS_TRUST_POLICY: &str =
    "(allow mach-lookup (global-name \"com.apple.TrustEvaluationAgent\"))\n";
const TTY_POLICY: &str = "; the command's own terminal, which prompts and pagers open\n\
     (allow file-read* file-write* file-ioctl (literal \"/dev/tty\"))";

/// Kept read-only under a writable root, when they exist there: the
/// repository (its hooks run outside the sandbox), the repository's Skills,
/// and the project's configuration, which could choose a weaker sandbox.
const READ_ONLY_NAMES: [&str; 3] = [".git", ".agents", ".hanafubuki-agent"];

/// A writable root as the policy names it: resolved, with the paths under
/// it that stay read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WritableRoot {
    pub(crate) root: String,
    pub(crate) read_only: Vec<String>,
}

/// `command` run by `sandbox-exec`, reading everything, writing only under
/// `writable_roots` (nothing when there are none), and using the network
/// only with `network_access`.
pub(crate) fn command_line(
    writable_roots: &[PathBuf],
    network_access: bool,
    command: Vec<String>,
) -> Vec<String> {
    let (policy, params) = policy(&resolve_roots(writable_roots), network_access);
    let mut line = vec![SANDBOX_EXEC.to_string(), "-p".to_string(), policy];
    line.extend(
        params
            .into_iter()
            .map(|(key, value)| format!("-D{key}={value}")),
    );
    line.push("--".to_string());
    line.extend(command);
    line
}

/// The Seatbelt policy and the parameters (`-D`) it names its paths by,
/// so no path is ever spliced into the policy's text.
pub(crate) fn policy(
    roots: &[WritableRoot],
    network_access: bool,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let mut writable = Vec::new();
    let mut anchors = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let key = format!("WRITABLE_ROOT_{index}");
        let mut filter = format!("(subpath (param \"{key}\"))");
        if !root.read_only.is_empty() {
            let mut parts = vec![filter];
            for (excluded_index, path) in root.read_only.iter().enumerate() {
                let excluded = format!("{key}_EXCLUDED_{excluded_index}");
                // The path itself as well as beneath it: `subpath` alone
                // would let the command create it.
                parts.push(format!("(require-not (literal (param \"{excluded}\")))"));
                parts.push(format!("(require-not (subpath (param \"{excluded}\")))"));
                params.push((excluded, path.clone()));
            }
            filter = format!("(require-all {} )", parts.join(" "));
        }
        // A command must not replace a root, which the next command's
        // policy names again, by renaming or removing it.
        anchors.push(format!(
            "(deny file-write-unlink (require-all (literal (param \"{key}\")) (vnode-type DIRECTORY)))"
        ));
        params.push((key, root.root.clone()));
        writable.push(filter);
    }

    let mut sections = vec![
        BASE_POLICY.to_string(),
        TTY_POLICY.to_string(),
        "; allow read-only file operations\n(allow file-read*)".to_string(),
    ];
    if !writable.is_empty() {
        sections.push(format!("(allow file-write*\n{}\n)", writable.join(" ")));
        sections.extend(anchors);
    }
    if network_access {
        sections.push(format!(
            "(allow network-outbound)\n(allow network-inbound)\n{NETWORK_POLICY}{TLS_TRUST_POLICY}"
        ));
    }
    sections.push(PREFERENCES_POLICY.to_string());
    // The denies come last: the last rule that matches decides.
    sections.push("(deny mach-lookup (xpc-service-name-prefix \"\"))".to_string());
    // These fcntls change a file through a read-only descriptor, past
    // file-write*: F_MAKECOMPRESSED (80) and F_TRANSFEREXTENTS (110).
    sections.push("(deny system-fcntl (fcntl-command 80 110))".to_string());
    (sections.join("\n"), params)
}

/// `roots` resolved (`/tmp` is `/private/tmp` to Seatbelt, which matches
/// the paths a command reaches), each once, with what stays read-only under
/// it. A root that does not resolve, one that does not exist, is left out
/// and so stays unwritable.
pub(crate) fn resolve_roots(roots: &[PathBuf]) -> Vec<WritableRoot> {
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
        .iter()
        .map(|root| WritableRoot {
            root: root.native_text().to_string(),
            read_only: read_only_under(root),
        })
        .collect()
}

/// The [`READ_ONLY_NAMES`] that exist directly under `root`; for a `.git`
/// file (a linked worktree's or a submodule's), the repository directory it
/// points to as well.
fn read_only_under(root: &ResolvedPath) -> Vec<String> {
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
        paths.push(path.native_text().to_string());
    }
    paths
}

/// The directory a `.git` file's `gitdir: <path>` names, relative to the
/// root it is in.
fn git_directory(root: &ResolvedPath, dot_git: &ResolvedPath) -> Option<String> {
    let contents = std::fs::read_to_string(dot_git.system()).ok()?;
    let (prefix, target) = contents.trim().split_once(':')?;
    let target = target.trim();
    if prefix.trim() != "gitdir" || target.is_empty() {
        return None;
    }
    let requested = root.join_request(Path::new(target)).ok()?;
    let directory = ResolvedPath::resolve(requested).ok()?;
    Some(directory.native_text().to_string())
}

#[cfg(test)]
mod tests;
