// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/windows-sandbox-rs/src/cap.rs` (a capability SID per writable
// root), `src/spawn_prep.rs` (which roots' SIDs a read-only path under
// them denies), `src/env.rs` (`apply_no_network_to_env`) and
// `src/winutil.rs` (the command line of an argv) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a root's capability SID is derived from
// its path key rather than drawn at random and stored, since a restricting
// SID grants nothing to a token that does not already list it; read-only
// has no capability SID of its own; the offline environment does not
// install `ssh`/`scp` stubs on the PATH; the command line is quoted as
// Rust's standard library quotes it, as the command would be spawned
// without a sandbox.

use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};
use sha2::{Digest, Sha256};

use crate::roots::ResolvedRoot;

const CAPABILITY_DOMAIN: &[u8] = b"hanafubuki-agent sandbox writable root\0";

/// The proxy and offline switches a command without the network gets
/// unless it already has them (Codex's `apply_no_network_to_env`). They
/// steer well-behaved tools; nothing in the unelevated sandbox blocks a
/// connection.
const OFFLINE_ENVIRONMENT: [(&str, &str); 13] = [
    ("HTTP_PROXY", "http://127.0.0.1:9"),
    ("HTTPS_PROXY", "http://127.0.0.1:9"),
    ("ALL_PROXY", "http://127.0.0.1:9"),
    ("NO_PROXY", "localhost,127.0.0.1,::1"),
    ("PIP_NO_INDEX", "1"),
    ("PIP_DISABLE_PIP_VERSION_CHECK", "1"),
    ("NPM_CONFIG_OFFLINE", "true"),
    ("CARGO_NET_OFFLINE", "true"),
    ("GIT_HTTP_PROXY", "http://127.0.0.1:9"),
    ("GIT_HTTPS_PROXY", "http://127.0.0.1:9"),
    ("GIT_SSH_COMMAND", "cmd /c exit 1"),
    ("GIT_ALLOW_PROTOCOLS", ""),
    ("SBX_NONET_ACTIVE", "1"),
];

/// The ACEs a command's writable roots need before it starts.
#[derive(Debug)]
pub(crate) struct AclPlan {
    /// Each root, writable to its own capability SID.
    pub(crate) grants: Vec<Grant>,
    /// Each path that stays read-only, unwritable to the SIDs of the
    /// grants (by index) whose roots hold it or lie under it.
    pub(crate) denies: Vec<Deny>,
}

#[derive(Debug)]
pub(crate) struct Grant {
    pub(crate) root: ResolvedPath,
    pub(crate) sid: String,
}

#[derive(Debug)]
pub(crate) struct Deny {
    pub(crate) path: ResolvedPath,
    pub(crate) grants: Vec<usize>,
}

/// The plan of `roots`. A path outside every root (the repository a
/// worktree's `.git` file names) is denied to every root's SID, as in
/// Codex.
pub(crate) fn acl_plan(roots: &[ResolvedRoot]) -> AclPlan {
    let grants: Vec<Grant> = roots
        .iter()
        .map(|root| Grant {
            sid: capability_sid(root.root.key()),
            root: root.root.clone(),
        })
        .collect();
    let mut denies: Vec<Deny> = Vec::new();
    for path in roots.iter().flat_map(|root| &root.read_only) {
        if denies.iter().any(|deny| deny.path.key() == path.key()) {
            continue;
        }
        let mut holders: Vec<usize> = (0..grants.len())
            .filter(|&index| {
                let root = &grants[index].root;
                path.starts_with(root) || root.starts_with(path)
            })
            .collect();
        if holders.is_empty() {
            holders = (0..grants.len()).collect();
        }
        denies.push(Deny {
            path: path.clone(),
            grants: holders,
        });
    }
    AclPlan { grants, denies }
}

/// The capability SID of the writable root `key`: the same for the same
/// place, whatever its spelling, so its ACE is added once and a token that
/// lists only the current roots' SIDs never writes a root an earlier
/// command had.
pub(crate) fn capability_sid(key: &PathKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CAPABILITY_DOMAIN);
    hasher.update(key.digest_bytes());
    let digest = hasher.finalize();
    let part = |index: usize| {
        let bytes = &digest[index * 4..index * 4 + 4];
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    };
    format!("S-1-5-21-{}-{}-{}-{}", part(0), part(1), part(2), part(3))
}

/// The variables of [`OFFLINE_ENVIRONMENT`] the environment lacks
/// (`is_set` says which it has).
pub(crate) fn offline_environment(
    is_set: impl Fn(&str) -> bool,
) -> Vec<(&'static str, &'static str)> {
    OFFLINE_ENVIRONMENT
        .into_iter()
        .filter(|(key, _)| !is_set(key))
        .collect()
}

/// The Windows command line of `argv`, quoted as Rust's standard library
/// quotes a command it spawns: the program always, an argument when it is
/// empty or holds a space or a tab, with the backslashes before a quote
/// doubled so `CommandLineToArgvW` reads every argument back unchanged.
pub(crate) fn command_line(argv: &[String]) -> Result<String, String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("no command to run".to_string());
    };
    if program.contains('"') || argv.iter().any(|arg| arg.contains('\0')) {
        return Err("the command holds a character a command line cannot".to_string());
    }
    let mut line = format!("\"{program}\"");
    for arg in args {
        line.push(' ');
        append_argument(&mut line, arg);
    }
    Ok(line)
}

fn append_argument(line: &mut String, arg: &str) {
    let quote = arg.is_empty() || arg.contains([' ', '\t']);
    if quote {
        line.push('"');
    }
    let mut backslashes = 0;
    for character in arg.chars() {
        if character == '\\' {
            backslashes += 1;
        } else {
            if character == '"' {
                line.extend(std::iter::repeat_n('\\', backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(character);
    }
    if quote {
        line.extend(std::iter::repeat_n('\\', backslashes));
        line.push('"');
    }
}

#[cfg(test)]
mod tests;
