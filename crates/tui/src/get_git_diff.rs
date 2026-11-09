// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Utility to compute the current Git diff for a directory (`/diff`).
//!
//! The implementation mirrors Codex's: it returns the diff for tracked
//! changes as well as any untracked files. When the directory is not inside
//! a Git repository, the function returns `Ok((false, String::new()))`.
//! Codex runs `git` in the process's working directory with its colours
//! (`--color`, read back through an ANSI parser) and the untracked files'
//! diffs in parallel; here it runs in the workspace's first root, without
//! colours (the pager colours the lines itself), one after the other, on
//! the caller's thread. It is the interface's own short-lived process, not
//! a tool call of the Agent's.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Return value of [`get_git_diff`].
///
/// * `bool` – Whether `directory` is inside a Git repo.
/// * `String` – The concatenated diff (may be empty).
pub(crate) fn get_git_diff(directory: &Path) -> io::Result<(bool, String)> {
    // First check if we are inside a Git repository.
    if !inside_git_repo(directory)? {
        return Ok((false, String::new()));
    }

    let tracked_diff = run_git_capture_diff(directory, &["diff"])?;
    let untracked_output =
        run_git_capture_stdout(directory, &["ls-files", "--others", "--exclude-standard"])?;

    let null_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut untracked_diff = String::new();
    for file in untracked_output
        .split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let args = ["diff", "--no-index", "--", null_path, file];
        match run_git_capture_diff(directory, &args) {
            Ok(diff) => untracked_diff.push_str(&diff),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }

    Ok((true, format!("{tracked_diff}{untracked_diff}")))
}

/// Helper that executes `git` with the given `args` and returns `stdout` as a
/// UTF-8 string. Any non-zero exit status is considered an *error*.
fn run_git_capture_stdout(directory: &Path, args: &[&str]) -> io::Result<String> {
    let output = git(directory, args).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(io::Error::other(format!(
            "git {:?} failed with status {}",
            args, output.status
        )))
    }
}

/// Like [`run_git_capture_stdout`] but treats exit status 1 as success and
/// returns stdout. Git returns 1 for diffs when differences are present.
fn run_git_capture_diff(directory: &Path, args: &[&str]) -> io::Result<String> {
    let output = git(directory, args).output()?;

    if output.status.success() || output.status.code() == Some(1) {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(io::Error::other(format!(
            "git {:?} failed with status {}",
            args, output.status
        )))
    }
}

/// Determine if `directory` is inside a Git repository.
fn inside_git_repo(directory: &Path) -> io::Result<bool> {
    let status = git(directory, &["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false), // git not installed
        Err(e) => Err(e),
    }
}

fn git(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests;
