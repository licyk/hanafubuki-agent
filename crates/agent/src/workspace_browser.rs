//! Read-only browsing in the workspace selected for a session or for the host.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_workspace_fs::WorkspaceFsPolicy;

use crate::{AgentApplicationService, AgentSessionId};

/// A workspace directory entry available to a human reader.
pub struct AgentWorkspaceEntry {
    pub path: PathBuf,
    pub directory: bool,
}

/// Bytes of a file opened by a human in its conversation's workspace.
pub struct AgentWorkspaceDocument {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

impl AgentApplicationService {
    fn browser_policy(&self, session: Option<AgentSessionId>) -> Result<WorkspaceFsPolicy, String> {
        let roots = match session {
            Some(session) => self.session_workspace_roots(session)?,
            None => self.workspace_roots()?,
        };
        WorkspaceFsPolicy::new(roots.into_vec(), self.workspace.protected_roots())
            .map_err(|error| error.to_string())
    }

    /// A single directory, with its accessible children. A disappearing child
    /// or a symlink leading outside the workspace does not hide its siblings.
    pub fn browse_workspace_directory(
        &self,
        session: Option<AgentSessionId>,
        path: &Path,
    ) -> Result<Vec<AgentWorkspaceEntry>, String> {
        let policy = self.browser_policy(session)?;
        let directory = policy
            .resolve_browser_path(path)
            .map_err(|e| e.to_string())?;
        let children = fs::read_dir(directory.system()).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for entry in children.flatten() {
            let child = directory.native().join(entry.file_name());
            let Ok(path) = policy.resolve_browser_path(&child) else {
                continue;
            };
            let Ok(meta) = fs::metadata(path.system()) else {
                continue;
            };
            if meta.is_dir() || meta.is_file() {
                entries.push(AgentWorkspaceEntry {
                    path: child,
                    directory: meta.is_dir(),
                });
            }
        }
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
        });
        Ok(entries)
    }

    /// Reads the bytes for one preview; the bound applies to bytes retained
    /// by the reader, including when the file grows during the read.
    pub fn read_workspace_document(
        &self,
        session: Option<AgentSessionId>,
        path: &Path,
        max_bytes: usize,
    ) -> Result<AgentWorkspaceDocument, String> {
        let policy = self.browser_policy(session)?;
        let path: ResolvedPath = policy
            .resolve_browser_path(path)
            .map_err(|e| e.to_string())?;
        let meta = fs::metadata(path.system()).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("Choose a regular file to preview".into());
        }
        let limit = max_bytes as u64;
        let too_large = || format!("Preview exceeds {max_bytes} bytes");
        if meta.len() > limit {
            return Err(too_large());
        }
        let mut file = fs::File::open(path.system())
            .map_err(|e| e.to_string())?
            .take(limit.saturating_add(1));
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > max_bytes {
            return Err(too_large());
        }
        Ok(AgentWorkspaceDocument {
            path: path.native().to_owned(),
            bytes,
        })
    }
}

/// Keeps the parent-directory subscription alive across atomic file replacement.
pub struct AgentWorkspaceWatch {
    _watcher: notify::RecommendedWatcher,
}

impl AgentApplicationService {
    pub fn watch_workspace_document(
        &self,
        session: Option<AgentSessionId>,
        path: &Path,
        changed: impl Fn() + Send + 'static,
    ) -> Result<AgentWorkspaceWatch, String> {
        use hanafubuki_agent_foundation::paths::PathKey;
        use notify::{Event, EventKind, RecursiveMode, Watcher};

        let policy = self.browser_policy(session)?;
        let document = policy
            .resolve_browser_path(path)
            .map_err(|e| e.to_string())?;
        let key = document.key().clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if let Ok(event) = event {
                if !matches!(event.kind, EventKind::Access(_))
                    && event.paths.iter().any(|p| PathKey::of(p) == key)
                {
                    changed();
                }
            }
        })
        .map_err(|e| e.to_string())?;
        let parent = document
            .system()
            .parent()
            .ok_or("preview file has no parent directory")?;
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|e| e.to_string())?;
        Ok(AgentWorkspaceWatch { _watcher: watcher })
    }
}
