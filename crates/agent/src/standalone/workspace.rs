//! The standalone Agent's workspace: the working directory, its Git root
//! and the directories added with it.

use std::path::{Path, PathBuf};

use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};
use parking_lot::RwLock;

use crate::{agent_workspace_roots_from, AgentWorkspaceRoots, WorkspaceRootsPort};

/// The working directory is the first root, so relative requests resolve
/// against it, even inside its Git root, which follows it. It can change
/// while the Agent runs ([`crate::AgentRuntime::set_working_directory`]);
/// the added directories stay.
pub(crate) struct StandaloneWorkspace {
    current: RwLock<WorkspaceState>,
    /// The `--add-dir` directories, resolved.
    add_dirs: Vec<PathBuf>,
}

/// The roots from one working directory.
#[derive(Clone)]
pub(crate) struct WorkspaceState {
    roots: AgentWorkspaceRoots,
    /// The project the configuration's project layer belongs to.
    project: PathBuf,
}

impl StandaloneWorkspace {
    pub(crate) fn new(cwd: &Path, add_dirs: &[PathBuf]) -> Result<Self, String> {
        let add_dirs = add_dirs
            .iter()
            .map(|directory| Ok(resolve_directory(directory)?.native().to_path_buf()))
            .collect::<Result<Vec<_>, String>>()?;
        let current = workspace_state(&resolve_directory(cwd)?, &add_dirs)?;
        Ok(Self {
            current: RwLock::new(current),
            add_dirs,
        })
    }

    pub(crate) fn project(&self) -> PathBuf {
        self.current.read().project.clone()
    }

    /// The workspace with `path`, relative to the current working directory
    /// or absolute, as its working directory; nothing changes until it is
    /// [applied](Self::apply).
    pub(crate) fn moved_to(&self, path: &Path) -> Result<WorkspaceState, String> {
        let current = self.current.read().roots.primary().clone();
        let requested = resolve_directory(&current)?
            .join_request(path)
            .map_err(|error| format!("working directory {}: {error}", path.display()))?;
        workspace_state(&resolve_directory(&requested)?, &self.add_dirs)
    }

    pub(crate) fn apply(&self, state: WorkspaceState) {
        *self.current.write() = state;
    }
}

impl WorkspaceState {
    pub(crate) fn roots(&self) -> &AgentWorkspaceRoots {
        &self.roots
    }
}

fn workspace_state(cwd: &ResolvedPath, add_dirs: &[PathBuf]) -> Result<WorkspaceState, String> {
    let git_root = match cwd
        .system()
        .ancestors()
        .find(|directory| directory.join(".git").exists())
    {
        Some(root) => Some(resolve_directory(root)?.native().to_path_buf()),
        None => None,
    };
    let project = git_root
        .clone()
        .unwrap_or_else(|| cwd.native().to_path_buf());
    let mut roots = vec![cwd.native().to_path_buf()];
    roots.extend(git_root);
    roots.extend(add_dirs.iter().cloned());
    let roots = agent_workspace_roots_from(roots)
        .ok_or_else(|| "the standalone workspace has no root".to_string())?;
    Ok(WorkspaceState { roots, project })
}

fn resolve_directory(directory: &Path) -> Result<ResolvedPath, String> {
    let resolved = ResolvedPath::resolve(directory)
        .map_err(|error| format!("workspace directory {}: {error}", directory.display()))?;
    if !resolved.system().is_dir() {
        return Err(format!(
            "workspace directory {} is not a directory",
            resolved.native_text()
        ));
    }
    Ok(resolved)
}

impl WorkspaceRootsPort for StandaloneWorkspace {
    fn roots(&self) -> Result<AgentWorkspaceRoots, String> {
        Ok(self.current.read().roots.clone())
    }

    fn protected_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn selected(&self, _host: &TurnExtensions) -> Option<PathBuf> {
        None
    }

    /// Each root once: a root inside another (the working directory inside
    /// its Git root) is searched as part of it.
    fn search_roots(&self, _host: &TurnExtensions) -> Result<Vec<(String, PathBuf)>, String> {
        let current = self.current.read();
        let roots = current.roots.roots();
        Ok(roots
            .iter()
            .filter(|root| {
                let key = PathKey::of(root);
                !roots
                    .iter()
                    .any(|other| PathKey::of(other) != key && key.starts_with(&PathKey::of(other)))
            })
            .map(|root| {
                let label = root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                (label, root.clone())
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
