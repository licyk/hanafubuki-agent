//! What a new session records of where it was created: the workspace's
//! primary root, and the Git branch checked out there, read from the work
//! tree's `HEAD` file without running Git.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::native_form;
use hanafubuki_agent_kernel::AgentSessionWorkspace;

/// The workspace a session created in `primary` records. Each part is
/// `None` when it cannot be read; a session is never refused for it.
pub(crate) fn session_workspace(primary: &Path) -> AgentSessionWorkspace {
    AgentSessionWorkspace {
        cwd: native_form(primary)
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
        branch: checked_out_branch(primary),
    }
}

/// The branch `HEAD` names in the work tree holding `directory`; `None` for
/// a detached `HEAD`, outside a work tree, or on anything unexpected.
fn checked_out_branch(directory: &Path) -> Option<String> {
    let dot_git = directory
        .ancestors()
        .map(|ancestor| ancestor.join(".git"))
        .find(|candidate| candidate.exists())?;
    let head = fs::read_to_string(repository_dir(&dot_git)?.join("HEAD")).ok()?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?;
    (!branch.is_empty()).then(|| branch.to_string())
}

/// The repository directory `.git` stands for: itself, or, in a linked
/// work tree or a submodule, the directory its `gitdir:` line names.
fn repository_dir(dot_git: &Path) -> Option<PathBuf> {
    if dot_git.is_dir() {
        return Some(dot_git.to_path_buf());
    }
    let pointer = fs::read_to_string(dot_git).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    Some(dot_git.parent()?.join(target))
}

#[cfg(test)]
mod tests;
