use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::{PathError, ResolvedPath};

use crate::{WorkspaceFsError, MAX_PATH_BYTES};

#[derive(Clone, Debug)]
pub struct WorkspaceFsPolicy {
    /// Every directory the Agent may operate in without elevation. Never empty;
    /// the first entry is the base for relative requests.
    workspace_roots: Vec<ResolvedPath>,
    protected_roots: Vec<ResolvedPath>,
}

impl WorkspaceFsPolicy {
    /// Build a policy over one or more workspace roots.
    ///
    /// The first root is the **base for relative requests**; all of them are
    /// checked for containment. Multiple roots exist because the Agent works on
    /// the WebUI instances this application manages, and an attached instance
    /// lives wherever the user put it — often on another volume entirely.
    ///
    /// Every supplied root must resolve. Deciding which roots are worth
    /// offering, and tolerating one that has gone missing, belongs to the
    /// caller that knows why each root is on the list.
    pub fn new(
        workspace_roots: impl IntoIterator<Item = PathBuf>,
        protected_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, WorkspaceFsError> {
        let workspace_roots = workspace_roots
            .into_iter()
            .map(|path| resolved_directory(&path, "workspace root"))
            .collect::<Result<Vec<_>, _>>()?;
        if workspace_roots.is_empty() {
            return Err(WorkspaceFsError::Resolution(
                "a workspace policy needs at least one root".to_string(),
            ));
        }
        // Keep the first root in place — it is the relative-request base — and
        // drop any later root that is the same place as, or nested inside, one
        // already kept. Nested roots would only widen nothing.
        let mut roots: Vec<ResolvedPath> = Vec::with_capacity(workspace_roots.len());
        for root in workspace_roots {
            if !roots.iter().any(|kept| root.starts_with(kept)) {
                roots.push(root);
            }
        }
        let mut protected = protected_roots
            .into_iter()
            .map(|path| resolved_directory(&path, "protected root"))
            .collect::<Result<Vec<_>, _>>()?;
        protected.sort_by(|left, right| left.key().cmp(right.key()));
        protected.dedup_by(|left, right| left.key() == right.key());
        Ok(Self {
            workspace_roots: roots,
            protected_roots: protected,
        })
    }

    /// Convenience for the single-root case: one directory, no elevation-free
    /// siblings.
    pub fn rooted_at(
        root: impl AsRef<Path>,
        protected_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, WorkspaceFsError> {
        Self::new([root.as_ref().to_path_buf()], protected_roots)
    }

    /// The workspace roots, in the order the policy checks them. The first is
    /// the base for relative requests.
    pub fn roots(&self) -> impl Iterator<Item = &Path> {
        self.workspace_roots.iter().map(ResolvedPath::native)
    }

    /// Turn the text a model supplied into an absolute native path.
    ///
    /// A relative request is resolved against the **first** root. With several
    /// roots there is no other defensible base, and a model that means one of
    /// the others can say so absolutely — the paths it is given back are always
    /// absolute.
    ///
    /// `.` and `..` are collapsed here, before the filesystem sees anything.
    /// Doing it later would be too late on Windows: a resolved root is a
    /// verbatim path, Win32 does not normalize verbatim paths, and
    /// `<root>\..\sibling` would simply fail to open.
    pub(crate) fn requested_path(&self, value: &str) -> Result<PathBuf, WorkspaceFsError> {
        if value.is_empty() || value.len() > MAX_PATH_BYTES || value.as_bytes().contains(&0) {
            return Err(WorkspaceFsError::InvalidRequest(format!(
                "path must contain 1 to {MAX_PATH_BYTES} bytes without NUL"
            )));
        }
        self.workspace_roots[0]
            .join_request(Path::new(value))
            .map_err(|error| WorkspaceFsError::Resolution(error.to_string()))
    }

    /// Whether a resolved path lies inside any workspace root.
    ///
    /// Compared through path identity rather than text: on Windows `\\?\E:\ws`
    /// and `E:\WS` are the same directory, and a byte-wise prefix test sees
    /// neither the prefix kind nor the case difference.
    pub(crate) fn is_within_workspace(&self, path: &ResolvedPath) -> bool {
        self.workspace_roots
            .iter()
            .any(|root| path.starts_with(root))
    }

    pub(crate) fn ensure_unprotected(&self, path: &ResolvedPath) -> Result<(), WorkspaceFsError> {
        if self
            .protected_roots
            .iter()
            .any(|root| path.starts_with(root))
        {
            return Err(WorkspaceFsError::ProtectedPath(
                path.native_text().to_string(),
            ));
        }
        Ok(())
    }
}

fn resolved_directory(path: &Path, label: &str) -> Result<ResolvedPath, WorkspaceFsError> {
    let resolved = ResolvedPath::resolve(path)
        .map_err(|error| WorkspaceFsError::Resolution(format!("{label}: {error}")))?;
    if !resolved.system().is_dir() {
        return Err(WorkspaceFsError::Resolution(format!(
            "{label} is not a directory"
        )));
    }
    Ok(resolved)
}

pub(crate) fn resolution_error(error: PathError) -> WorkspaceFsError {
    WorkspaceFsError::Resolution(error.to_string())
}
