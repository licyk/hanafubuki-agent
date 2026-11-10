// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/sandboxing/src/seatbelt.rs` (the Seatbelt policy of a command
// and `sandbox-exec`'s command line) and `codex-rs/protocol/src/
// permissions.rs` (`default_read_only_subpaths_for_writable_root`, what
// stays read-only under a writable root) and its restricted reads (each
// readable root a `subpath`, with `seatbelt_read_only_platform_defaults.sbpl`)
// at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the policies a command or a plugin
// can have here, reading everything (or, for a plugin, the platform's
// defaults and its readable roots) and writing nothing or under writable
// roots, so no unreadable globs, network proxy,
// Unix-socket allowlist or app-server socket protection; the writable roots
// and what stays read-only under them are `crate::roots`'; `/dev/tty` is
// writable, as on Linux.

use std::path::PathBuf;

/// Only this `sandbox-exec`, never one found on the PATH, which a command
/// could have changed; replacing this one already takes root.
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

const BASE_POLICY: &str = include_str!("seatbelt_base_policy.sbpl");
const NETWORK_POLICY: &str = include_str!("seatbelt_network_policy.sbpl");
const PREFERENCES_POLICY: &str = include_str!("seatbelt_preferences_policy.sbpl");
const PLATFORM_DEFAULTS_POLICY: &str = include_str!("seatbelt_read_only_platform_defaults.sbpl");
// System libcurl needs this service for TLS.
const TLS_TRUST_POLICY: &str =
    "(allow mach-lookup (global-name \"com.apple.TrustEvaluationAgent\"))\n";
const TTY_POLICY: &str = "; the command's own terminal, which prompts and pagers open\n\
     (allow file-read* file-write* file-ioctl (literal \"/dev/tty\"))";

/// A writable root as the policy names it: resolved, with the paths under
/// it that stay read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WritableRoot {
    pub(crate) root: String,
    pub(crate) read_only: Vec<String>,
}

/// `command` run by `sandbox-exec`, reading everything (with
/// `readable_roots`, only the platform's defaults, those roots and the
/// writable ones), writing only under `writable_roots` (nothing when there
/// are none), and using the network only with `network_access`.
pub(crate) fn command_line(
    writable_roots: &[PathBuf],
    readable_roots: Option<&[PathBuf]>,
    network_access: bool,
    command: Vec<String>,
) -> Vec<String> {
    let readable = readable_roots.map(resolve_readable);
    let (policy, params) = policy(
        &resolve_roots(writable_roots),
        readable.as_deref(),
        network_access,
    );
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
    readable_roots: Option<&[String]>,
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

    let read_policy = match readable_roots {
        None => "; allow read-only file operations\n(allow file-read*)".to_string(),
        Some(readable) => {
            let mut subpaths = Vec::new();
            for (index, path) in readable.iter().enumerate() {
                let key = format!("READABLE_ROOT_{index}");
                subpaths.push(format!("(subpath (param \"{key}\"))"));
                params.push((key, path.clone()));
            }
            for index in 0..roots.len() {
                subpaths.push(format!("(subpath (param \"WRITABLE_ROOT_{index}\"))"));
            }
            format!(
                "; allow read-only file operations under the readable roots\n(allow file-read*\n{}\n)",
                subpaths.join(" ")
            )
        }
    };
    let mut sections = vec![BASE_POLICY.to_string(), TTY_POLICY.to_string(), read_policy];
    if !writable.is_empty() {
        sections.push(format!("(allow file-write*\n{}\n)", writable.join(" ")));
        sections.extend(anchors);
    }
    if network_access {
        sections.push(format!(
            "(allow network-outbound)\n(allow network-inbound)\n{NETWORK_POLICY}{TLS_TRUST_POLICY}"
        ));
    }
    sections.push(match readable_roots {
        None => PREFERENCES_POLICY.to_string(),
        Some(_) => PLATFORM_DEFAULTS_POLICY.to_string(),
    });
    // The denies come last: the last rule that matches decides.
    sections.push("(deny mach-lookup (xpc-service-name-prefix \"\"))".to_string());
    // These fcntls change a file through a read-only descriptor, past
    // file-write*: F_MAKECOMPRESSED (80) and F_TRANSFEREXTENTS (110).
    sections.push("(deny system-fcntl (fcntl-command 80 110))".to_string());
    (sections.join("\n"), params)
}

/// Readable roots resolved, as the policy names them; one that does not
/// resolve is left out, and so stays unreadable.
fn resolve_readable(roots: &[PathBuf]) -> Vec<String> {
    crate::roots::resolve_roots(roots)
        .into_iter()
        .map(|root| root.root.native_text().to_string())
        .collect()
}

/// `roots` resolved, each once, with what stays read-only under it
/// ([`crate::roots::resolve_roots`]), as the policy names them.
pub(crate) fn resolve_roots(roots: &[PathBuf]) -> Vec<WritableRoot> {
    crate::roots::resolve_roots(roots)
        .into_iter()
        .map(|root| WritableRoot {
            root: root.root.native_text().to_string(),
            read_only: root
                .read_only
                .iter()
                .map(|path| path.native_text().to_string())
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
