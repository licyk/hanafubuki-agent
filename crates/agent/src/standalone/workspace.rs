//! The standalone Agent's workspace: the working directory, its Git root
//! and the directories added with it.

use std::path::{Path, PathBuf};

use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};

use crate::{agent_workspace_roots_from, AgentWorkspaceRoots, WorkspaceRootsPort};

/// The roots are fixed for the run. The working directory is the first, so
/// relative requests resolve against it, even inside its Git root, which
/// follows it.
pub(crate) struct StandaloneWorkspace {
    roots: AgentWorkspaceRoots,
    /// The project the configuration's project layer belongs to.
    project: PathBuf,
}

impl StandaloneWorkspace {
    pub(crate) fn new(cwd: &Path, add_dirs: &[PathBuf]) -> Result<Self, String> {
        let cwd = resolve_directory(cwd)?;
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
        for directory in add_dirs {
            roots.push(resolve_directory(directory)?.native().to_path_buf());
        }
        let roots = agent_workspace_roots_from(roots)
            .ok_or_else(|| "the standalone workspace has no root".to_string())?;
        Ok(Self { roots, project })
    }

    pub(crate) fn project(&self) -> &Path {
        &self.project
    }
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
        Ok(self.roots.clone())
    }

    fn extra_read_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn selected(&self, _host: &TurnExtensions) -> Option<PathBuf> {
        None
    }

    /// Each root once: a root inside another (the working directory inside
    /// its Git root) is searched as part of it.
    fn search_roots(&self, _host: &TurnExtensions) -> Result<Vec<(String, PathBuf)>, String> {
        let roots = self.roots.roots();
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
