//! Where the Agent's direct filesystem tools may work as ordinary edits.
//!
//! The roots come from the host ([`WorkspaceRootsPort`]), never from the
//! process working directory: a packaged application's working directory is
//! whatever launched it, so the boundary between "an ordinary edit" and "a
//! write outside the workspace" would move with how the user started it.
//!
//! Writing a path outside these roots is not gated by an envelope directive
//! in the patch; it is an elevated request decided by the same approval flow
//! as every other mutation. The exact external paths are frozen into the
//! approval action, the request is classified at the highest risk, and the
//! user is asked to approve it.

use std::path::PathBuf;

use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_foundation::paths::PathKey;

/// Where the Agent works, as the host knows it. Every method may block; the
/// Agent calls them on its blocking pool.
pub trait WorkspaceRootsPort: Send + Sync {
    /// The roots as they are now, read again for every turn and every Skill
    /// catalog read. The first is the base a relative request resolves
    /// against.
    fn roots(&self) -> Result<AgentWorkspaceRoots, String>;

    /// Directories the filesystem tools refuse, reads included, even under a
    /// root: the host's own data, its credentials among it.
    fn protected_roots(&self) -> Vec<PathBuf>;

    /// The directory what the host submitted with the turn (`host`) selects,
    /// which narrows where workspace instructions are looked up.
    fn selected(&self, host: &TurnExtensions) -> Option<PathBuf>;

    /// The directories file search walks for what the host submitted with
    /// the search, each with the label its candidates carry.
    fn search_roots(&self, host: &TurnExtensions) -> Result<Vec<(String, PathBuf)>, String>;
}

/// The Agent's elevation-free filesystem surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentWorkspaceRoots {
    /// Every root, the host's primary root first. Never empty, deduplicated
    /// on path identity, and free of roots nested inside an earlier root;
    /// an earlier root may lie inside a later one (the standalone working
    /// directory inside its Git root).
    ///
    /// The first entry is the base for relative requests.
    roots: Vec<PathBuf>,
}

impl AgentWorkspaceRoots {
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// The base a relative request resolves against.
    pub fn primary(&self) -> &PathBuf {
        &self.roots[0]
    }

    pub fn into_vec(self) -> Vec<PathBuf> {
        self.roots
    }
}

/// Workspace roots from a list, the first one first: the first occurrence of
/// each location is kept and a root nested in one already kept is dropped.
/// `None` for an empty list.
pub fn agent_workspace_roots_from(
    roots: impl IntoIterator<Item = PathBuf>,
) -> Option<AgentWorkspaceRoots> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for root in roots {
        let key = PathKey::of(&root);
        if !kept
            .iter()
            .any(|existing| key.starts_with(&PathKey::of(existing)))
        {
            kept.push(root);
        }
    }
    (!kept.is_empty()).then_some(AgentWorkspaceRoots { roots: kept })
}

#[cfg(test)]
mod tests;
