// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/linux-sandbox/src/seccomp.rs`
// (`install_filesystem_landlock_rules_on_current_thread`, its legacy
// Landlock filesystem rules) and `codex-rs/linux-sandbox/src/bwrap.rs`
// (`LINUX_PLATFORM_DEFAULT_READ_ROOTS`, the system directories a
// restricted read keeps) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: errors are text for the helper's report;
// read-only is the same rules without writable roots; `/dev/tty` is
// writable too; a restricted read is Landlock rules on the platform's
// directories and the readable roots rather than a bubblewrap mount.

use std::path::{Path, PathBuf};

use landlock::{
    Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr,
    RulesetStatus, ABI,
};

/// What a restricted read keeps readable: the system's programs, libraries
/// and configuration (Nix's too), never a user's data.
const PLATFORM_READ_ROOTS: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/lib",
    "/lib64",
    "/nix/store",
    "/run/current-system/sw",
];
/// The devices a restricted read keeps readable.
const READABLE_DEVICES: &[&str] = &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"];

/// Lets this thread write only to `/dev/null`, `/dev/tty` (the process's
/// own controlling terminal: the PTY the Agent gave a terminal command,
/// which prompts and pagers open) and under `writable_roots`, and read and
/// execute everything, or with `readable_roots` only the platform's
/// directories, those roots and the writable ones. Landlock rules only add
/// rights, so nothing under a writable root can be kept read-only; a root
/// that cannot be opened (one that does not exist) gets no rule, so stays
/// out of reach.
///
/// Best effort, as in Codex: a kernel older than the ABI handles the rights
/// it knows (before Linux 6.2, for one, truncating a file is not
/// restricted), but one that enforces nothing is an error.
pub(super) fn restrict(
    writable_roots: &[PathBuf],
    readable_roots: Option<&[PathBuf]>,
) -> Result<(), String> {
    let abi = ABI::V5;
    let read_write = AccessFs::from_all(abi);
    let read_only = AccessFs::from_read(abi);
    let landlock = |error: landlock::RulesetError| format!("Landlock: {error}");
    let readable: Vec<&Path> = match readable_roots {
        None => vec![Path::new("/")],
        Some(roots) => PLATFORM_READ_ROOTS
            .iter()
            .chain(READABLE_DEVICES)
            .map(Path::new)
            .chain(roots.iter().map(PathBuf::as_path))
            .collect(),
    };
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(read_write)
        .map_err(landlock)?
        .create()
        .map_err(landlock)?
        .add_rules(landlock::path_beneath_rules(readable, read_only))
        .map_err(landlock)?
        .add_rules(landlock::path_beneath_rules(
            ["/dev/null", "/dev/tty"],
            read_write,
        ))
        .map_err(landlock)?
        .no_new_privs(true);
    if !writable_roots.is_empty() {
        ruleset = ruleset
            .add_rules(landlock::path_beneath_rules(writable_roots, read_write))
            .map_err(landlock)?;
    }
    let status = ruleset.restrict_self().map_err(landlock)?;
    if status.ruleset == RulesetStatus::NotEnforced {
        return Err("Landlock enforced none of the filesystem rules".to_string());
    }
    Ok(())
}
