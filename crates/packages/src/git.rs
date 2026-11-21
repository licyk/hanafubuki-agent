// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/marketplace_upgrade/git.rs` and
// `clone_git_plugin_source` of `codex-rs/core-plugins/src/loader.rs` (a
// marketplace's or a package's clone: sparse paths, a ref or a verified
// SHA, the remote's revision by `ls-remote`), with the isolated `git` of
// `codex-rs/worktree/src/git.rs`, at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `git` runs as a reserved execution of the
// Agent's `ExecutorManager` rather than a `std::process::Command`, bounded
// by a timeout, with the inherited repository selectors removed, hooks and
// filesystem monitoring off and bare repositories only when explicit; one
// clone serves marketplaces and package sources.

use std::collections::BTreeMap;
use std::path::Path;

use hanafubuki_agent_process::executor::ExecutorManager;

use crate::program::{self, ProgramRequest};

const GIT_TIMEOUT_MS: u64 = 10 * 60 * 1_000;
const GIT_OUTPUT_CAP: usize = 4 * 1024 * 1024;
const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// What a wrapper of `git` may have exported to select another repository.
const REPOSITORY_SELECTORS: [&str; 16] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// The variables `git` reads, as the Agent's processes inherit them.
fn git_environment() -> BTreeMap<String, Option<String>> {
    REPOSITORY_SELECTORS
        .iter()
        .map(|name| (name.to_string(), None))
        .chain([
            ("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string())),
            ("GIT_OPTIONAL_LOCKS".to_string(), Some("0".to_string())),
        ])
        .collect()
}

/// A clone to make.
pub(crate) struct CloneRequest<'a> {
    pub(crate) url: &'a str,
    /// Paths to check out alone; empty for the whole tree.
    pub(crate) sparse_paths: &'a [String],
    /// A package's subdirectory is one path pattern (`--no-cone`); a
    /// marketplace's sparse paths are directories (cone mode).
    pub(crate) cone: bool,
    pub(crate) ref_name: Option<&'a str>,
    /// A revision the checkout must end at.
    pub(crate) sha: Option<&'a str>,
}

/// Clones into `destination`, which must not exist, and returns the
/// revision checked out.
pub(crate) fn clone(
    executor: &ExecutorManager,
    request: &CloneRequest<'_>,
    destination: &Path,
) -> Result<String, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "the clone's destination has no parent".to_string())?;
    let target = destination.to_string_lossy();
    if request.sparse_paths.is_empty() {
        git(executor, parent, &["clone", request.url, &target])?;
    } else {
        let mut clone = vec!["clone", "--filter=blob:none", "--no-checkout"];
        if !request.cone {
            clone.push("--sparse");
        }
        clone.extend([request.url, &target]);
        git(executor, parent, &clone)?;
        let mut sparse = vec!["sparse-checkout", "set"];
        if !request.cone {
            sparse.extend(["--no-cone", "--"]);
        }
        sparse.extend(request.sparse_paths.iter().map(String::as_str));
        git(executor, destination, &sparse)?;
    }
    if let Some(sha) = request.sha {
        git(executor, destination, &["checkout", sha])?;
        let head = head(executor, destination)?;
        if !head.eq_ignore_ascii_case(sha) {
            return Err(format!(
                "checked out Git SHA {head} does not match requested SHA {sha}"
            ));
        }
        return Ok(head);
    }
    if let Some(ref_name) = request.ref_name {
        git(executor, destination, &["checkout", ref_name])?;
    } else if !request.sparse_paths.is_empty() {
        git(executor, destination, &["checkout"])?;
    }
    head(executor, destination)
}

/// The revision `ref_name` (else `HEAD`) names at `url`, without a clone.
pub(crate) fn remote_revision(
    executor: &ExecutorManager,
    cwd: &Path,
    url: &str,
    ref_name: Option<&str>,
) -> Result<String, String> {
    if let Some(sha) = ref_name.filter(|name| is_full_sha(name)) {
        return Ok(sha.to_string());
    }
    let output = git(
        executor,
        cwd,
        &["ls-remote", url, ref_name.unwrap_or("HEAD")],
    )?;
    let output = String::from_utf8_lossy(&output);
    output
        .lines()
        .next()
        .and_then(|line| line.split_once('\t'))
        .map(|(revision, _)| revision.trim().to_string())
        .filter(|revision| !revision.is_empty())
        .ok_or_else(|| {
            format!(
                "git ls-remote found no {} at {url}",
                ref_name.unwrap_or("HEAD")
            )
        })
}

fn head(executor: &ExecutorManager, repository: &Path) -> Result<String, String> {
    let output = git(executor, repository, &["rev-parse", "HEAD"])?;
    let revision = String::from_utf8_lossy(&output).trim().to_string();
    if revision.is_empty() {
        return Err("git rev-parse HEAD printed no revision".to_string());
    }
    Ok(revision)
}

fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Runs `git args` in `cwd` to its end and returns its stdout; a failure is
/// its stderr.
fn git(executor: &ExecutorManager, cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let command = command(args);
    let env = git_environment();
    let (exit_code, stdout, stderr) = program::run(
        executor,
        ProgramRequest {
            name: "git",
            command: &command,
            cwd,
            env: &env,
            timeout_ms: GIT_TIMEOUT_MS,
            output_cap: GIT_OUTPUT_CAP,
        },
    )?;
    if exit_code == Some(0) {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        format!("git {} exited with {exit_code:?}", args.join(" "))
    } else {
        format!("git {} failed: {stderr}", args.join(" "))
    })
}

fn command(args: &[&str]) -> Vec<String> {
    let mut command = vec![
        "git".to_string(),
        "-c".to_string(),
        "safe.bareRepository=explicit".to_string(),
        "-c".to_string(),
        format!("core.hooksPath={DISABLED_HOOKS_PATH}"),
        "-c".to_string(),
        "core.fsmonitor=".to_string(),
    ];
    command.extend(args.iter().map(|arg| arg.to_string()));
    command
}

#[cfg(test)]
mod tests;
