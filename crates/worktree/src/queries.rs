// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/git-utils/src/info.rs` (`local_git_branches`,
// `current_branch_name`, `recent_commits`) and `src/branch.rs`
// (`merge_base_with_head`) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `git` runs through the Agent's
// `ExecutorManager`; a failure is an error rather than an empty answer,
// except a repository without commits, which has none; a commit carries no
// timestamp; the merge base is taken with the local branch, never its
// upstream when the upstream is ahead.

//! What to review: a repository's local branches, its current branch, its
//! latest commits and the merge base of `HEAD` with a branch.

use std::path::Path;

use hanafubuki_agent_process::executor::ExecutorManager;

use crate::git::{git, git_text, git_text_unless_one};
use crate::WorktreeError;

/// A commit of the current branch's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    /// The first line of its message.
    pub subject: String,
}

/// Read-only questions about the repository a directory is in, every
/// `git` run through `executor`.
#[derive(Clone)]
pub struct GitQueries {
    executor: ExecutorManager,
}

impl GitQueries {
    pub fn new(executor: ExecutorManager) -> Self {
        Self { executor }
    }

    /// Staged and unstaged patches, followed by untracked file names.
    /// Every query uses the same owned executor as the worktree operations.
    pub fn working_tree_diff(&self, cwd: &Path) -> Result<String, WorktreeError> {
        let staged = git_text(
            &self.executor,
            cwd,
            &["diff", "--no-ext-diff", "--no-textconv", "--cached"],
        )?;
        let unstaged = git_text(
            &self.executor,
            cwd,
            &["diff", "--no-ext-diff", "--no-textconv"],
        )?;
        let untracked = git_text(
            &self.executor,
            cwd,
            &["ls-files", "--others", "--exclude-standard"],
        )?;
        let mut sections = Vec::new();
        if !staged.is_empty() {
            sections.push(format!("Staged changes\n{staged}"));
        }
        if !unstaged.is_empty() {
            sections.push(format!("Unstaged changes\n{unstaged}"));
        }
        if !untracked.is_empty() {
            sections.push(format!("Untracked files\n{untracked}"));
        }
        Ok(sections.join("\n\n"))
    }

    /// The local branches by name, the default one (`main`, else `master`)
    /// first.
    pub fn local_branches(&self, cwd: &Path) -> Result<Vec<String>, WorktreeError> {
        let listing = git_text(
            &self.executor,
            cwd,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )?;
        let mut branches = listing
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        branches.sort_unstable();
        if let Some(position) = ["main", "master"]
            .iter()
            .find_map(|default| branches.iter().position(|name| name == default))
        {
            let default = branches.remove(position);
            branches.insert(0, default);
        }
        Ok(branches)
    }

    /// The branch checked out; `None` while `HEAD` is detached.
    pub fn current_branch(&self, cwd: &Path) -> Result<Option<String>, WorktreeError> {
        let branch = git_text(&self.executor, cwd, &["branch", "--show-current"])?;
        Ok(Some(branch.trim().to_string()).filter(|branch| !branch.is_empty()))
    }

    /// The latest `limit` commits of `HEAD`'s history, newest first.
    pub fn recent_commits(
        &self,
        cwd: &Path,
        limit: usize,
    ) -> Result<Vec<GitCommit>, WorktreeError> {
        if git_text_unless_one(
            &self.executor,
            cwd,
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
        )?
        .is_none()
        {
            return Ok(Vec::new());
        }
        let limit = limit.to_string();
        let log = git(
            &self.executor,
            cwd,
            &[
                "log",
                "-n",
                &limit,
                "--pretty=format:%H%x1f%s",
                "--end-of-options",
                "HEAD",
            ],
        )?;
        Ok(String::from_utf8_lossy(&log)
            .lines()
            .filter_map(|line| {
                let (sha, subject) = line.split_once('\u{1f}')?;
                Some(GitCommit {
                    sha: sha.trim().to_string(),
                    subject: subject.trim().to_string(),
                })
            })
            .filter(|commit| !commit.sha.is_empty())
            .collect())
    }

    /// The merge base of `HEAD` and the local `branch`; `None` when either
    /// does not name a commit or they share no history.
    pub fn merge_base_with_head(
        &self,
        cwd: &Path,
        branch: &str,
    ) -> Result<Option<String>, WorktreeError> {
        for revision in ["HEAD^{commit}".to_string(), format!("{branch}^{{commit}}")] {
            if git_text_unless_one(
                &self.executor,
                cwd,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &revision,
                ],
            )?
            .is_none()
            {
                return Ok(None);
            }
        }
        git_text_unless_one(
            &self.executor,
            cwd,
            &["merge-base", "--end-of-options", "HEAD", branch],
        )
    }
}
