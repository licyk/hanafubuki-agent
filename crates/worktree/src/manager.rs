// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/worktree/src/lib.rs` and `src/paths.rs` (`WorktreeManager`:
// a detached checkout of `HEAD` added without a checkout and then reset
// into place, rolled back when that fails; the managed worktrees of a
// repository from `git worktree list --porcelain -z`; removal) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a worktree is `<root>/<repository>-<id>`
// rather than in a bucket, and is always the repository's root (no base
// revision, no working directory below it); paths are resolved and compared
// through the foundation's `ResolvedPath` and `PathKey`; listing does not
// re-check the administrative backlinks; removal forces, then prunes.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};
use hanafubuki_agent_process::executor::ExecutorManager;

use crate::git::{git, git_path, git_text};
use crate::{owner, WorktreeError};

/// Tries at a fresh name before giving up.
const NAME_ATTEMPTS: usize = 16;

/// A worktree under the managed directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    /// Its root, in native form.
    pub path: PathBuf,
    /// The commit checked out.
    pub head: String,
    /// The branch checked out; `None` while `HEAD` is detached, as it is
    /// when the worktree is made.
    pub branch: Option<String>,
    /// The session it belongs to.
    pub owner: Option<String>,
}

/// The worktrees under one directory, every `git` run through `executor`.
#[derive(Clone)]
pub struct WorktreeManager {
    root: PathBuf,
    executor: ExecutorManager,
}

impl WorktreeManager {
    pub fn new(root: PathBuf, executor: ExecutorManager) -> Self {
        Self { root, executor }
    }

    /// A new worktree of the repository `source` is in, detached at its
    /// `HEAD`, with no owner yet.
    pub fn create(&self, source: &Path) -> Result<Worktree, WorktreeError> {
        let repository = self.repository_root(source)?;
        let head = git_text(
            &self.executor,
            repository.native(),
            &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"],
        )?;
        let path = self.unused_path(&repository)?;
        let target = path.native_text();
        git(
            &self.executor,
            repository.native(),
            &[
                "worktree",
                "add",
                "--detach",
                "--no-checkout",
                target,
                &head,
            ],
        )?;
        let reset = git(
            &self.executor,
            path.native(),
            &["reset", "--hard", "--no-recurse-submodules", &head],
        );
        if let Err(error) = reset {
            let rollback = git(
                &self.executor,
                repository.native(),
                &["worktree", "remove", "--force", target],
            );
            return Err(match rollback {
                Ok(_) => error,
                Err(rollback) => WorktreeError::Git(format!(
                    "{error}; the incomplete worktree {target} was not removed: {rollback}"
                )),
            });
        }
        Ok(Worktree {
            path: path.native().to_path_buf(),
            head,
            branch: None,
            owner: None,
        })
    }

    /// The managed worktrees of the repository `source` is in, by path. One
    /// whose owner does not read is listed without one.
    pub fn list(&self, source: &Path) -> Result<Vec<Worktree>, WorktreeError> {
        let source = resolve(source)?;
        let listing = git(
            &self.executor,
            source.native(),
            &["worktree", "list", "--porcelain", "-z"],
        )?;
        let mut worktrees = Vec::new();
        for entry in listing
            .split(|byte| *byte == 0)
            .collect::<Vec<_>>()
            .split(|field| field.is_empty())
        {
            let Some(worktree) = self.listed(entry) else {
                continue;
            };
            let owner = owner::owner(&self.executor, &worktree.path).unwrap_or_else(|error| {
                log::warn!(
                    "the owner of worktree {} does not read: {error}",
                    worktree.path.display()
                );
                None
            });
            worktrees.push(Worktree { owner, ..worktree });
        }
        worktrees.sort_by(|left, right| PathKey::of(&left.path).cmp(&PathKey::of(&right.path)));
        Ok(worktrees)
    }

    /// Binds the worktree at `path` to `session` (see [`Worktree::owner`]);
    /// a worktree another session owns is refused.
    pub fn bind(&self, path: &Path, session: &str) -> Result<(), WorktreeError> {
        let path = self.managed(path)?;
        owner::bind(&self.executor, path.native(), session)
    }

    /// Removes the worktree at `path`, local changes and all, and prunes
    /// the repository's record of it; run from `source`, which must not be
    /// inside it.
    pub fn remove(&self, source: &Path, path: &Path) -> Result<(), WorktreeError> {
        let source = resolve(source)?;
        let path = self.managed(path)?;
        if source.starts_with(&path) {
            return Err(WorktreeError::InUse(path.native().to_path_buf()));
        }
        git(
            &self.executor,
            source.native(),
            &["worktree", "remove", "--force", path.native_text()],
        )?;
        git(&self.executor, source.native(), &["worktree", "prune"])?;
        Ok(())
    }

    fn repository_root(&self, source: &Path) -> Result<ResolvedPath, WorktreeError> {
        let source = resolve(source)?;
        resolve(&git_path(
            &self.executor,
            source.native(),
            &["rev-parse", "--show-toplevel"],
        )?)
    }

    /// `<root>/<repository>-<id>` for an `id` no directory has yet.
    fn unused_path(&self, repository: &ResolvedPath) -> Result<ResolvedPath, WorktreeError> {
        fs::create_dir_all(&self.root).map_err(|error| {
            WorktreeError::Io(format!("create {}: {error}", self.root.display()))
        })?;
        let root = resolve(&self.root)?;
        let name = repository
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_string());
        for _ in 0..NAME_ATTEMPTS {
            let id = uuid::Uuid::new_v4().simple().to_string();
            let candidate = format!("{name}-{}", &id[..8]);
            let path = ResolvedPath::resolve_child(&root, candidate.as_ref())
                .map_err(|error| WorktreeError::Io(error.to_string()))?;
            if !path.system().exists() {
                return Ok(path);
            }
        }
        Err(WorktreeError::Io(format!(
            "no unused worktree name under {}",
            root.native_text()
        )))
    }

    /// `path` if it is a worktree directly under the managed directory.
    fn managed(&self, path: &Path) -> Result<ResolvedPath, WorktreeError> {
        let not_managed = || WorktreeError::NotManaged(path.to_path_buf());
        let resolved = ResolvedPath::resolve(path).map_err(|_| not_managed())?;
        let root = ResolvedPath::resolve(&self.root).map_err(|_| not_managed())?;
        let directly_under = resolved
            .native()
            .parent()
            .is_some_and(|parent| PathKey::of(parent) == *root.key());
        if !directly_under || !resolved.system().join(".git").is_file() {
            return Err(not_managed());
        }
        Ok(resolved)
    }

    /// One entry of `git worktree list --porcelain -z`, if it is a managed
    /// worktree that is still there.
    fn listed(&self, fields: &[&[u8]]) -> Option<Worktree> {
        let mut path = None;
        let mut head = None;
        let mut branch = None;
        for field in fields {
            let field = std::str::from_utf8(field).ok()?;
            if let Some(value) = field.strip_prefix("worktree ") {
                path = Some(PathBuf::from(value));
            } else if let Some(value) = field.strip_prefix("HEAD ") {
                head = Some(value.to_string());
            } else if let Some(value) = field.strip_prefix("branch ") {
                branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_string(),
                );
            }
        }
        let path = self.managed(&path?).ok()?;
        Some(Worktree {
            path: path.native().to_path_buf(),
            head: head?,
            branch,
            owner: None,
        })
    }
}

fn resolve(path: &Path) -> Result<ResolvedPath, WorktreeError> {
    ResolvedPath::resolve(path)
        .map_err(|error| WorktreeError::Io(format!("{}: {error}", path.display())))
}
