// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/linux-sandbox/src/seccomp.rs`
// (`install_filesystem_landlock_rules_on_current_thread`, its legacy
// Landlock filesystem rules) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: errors are text for the helper's report;
// read-only is the same rules without writable roots; `/dev/tty` is
// writable too.

use std::path::PathBuf;

use landlock::{
    Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr,
    RulesetStatus, ABI,
};

/// Lets this thread read and execute everything but write only to
/// `/dev/null`, `/dev/tty` (the process's own controlling terminal: the
/// PTY the Agent gave a terminal command, which prompts and pagers open)
/// and under `writable_roots`. Landlock rules only add rights,
/// so nothing under a writable root can be kept read-only; a root that
/// cannot be opened (one that does not exist) gets no rule, so stays
/// unwritable.
///
/// Best effort, as in Codex: a kernel older than the ABI handles the rights
/// it knows (before Linux 6.2, for one, truncating a file is not
/// restricted), but one that enforces nothing is an error.
pub(super) fn restrict_writes(writable_roots: &[PathBuf]) -> Result<(), String> {
    let abi = ABI::V5;
    let read_write = AccessFs::from_all(abi);
    let read_only = AccessFs::from_read(abi);
    let landlock = |error: landlock::RulesetError| format!("Landlock: {error}");
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(read_write)
        .map_err(landlock)?
        .create()
        .map_err(landlock)?
        .add_rules(landlock::path_beneath_rules(["/"], read_only))
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
