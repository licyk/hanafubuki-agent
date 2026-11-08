use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::AgentMemoryError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMemoryPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub ad_hoc_notes: PathBuf,
    pub artifacts: PathBuf,
    pub staging: PathBuf,
    pub reset_trash: PathBuf,
}

impl AgentMemoryPaths {
    pub fn under(root: PathBuf) -> Self {
        Self {
            database: root.join("memory.sqlite3"),
            ad_hoc_notes: root.join("ad-hoc-notes.jsonl"),
            artifacts: root.join("artifacts"),
            staging: root.join("staging"),
            reset_trash: root.join("reset-trash"),
            root,
        }
    }

    pub(crate) fn prepare(&self) -> Result<(), AgentMemoryError> {
        if self
            .root
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(AgentMemoryError::InvalidInput(
                "memory root must not be a symbolic link".into(),
            ));
        }
        fs::create_dir_all(&self.root).map_err(io_error)?;
        for directory in [&self.artifacts, &self.staging, &self.reset_trash] {
            if directory
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(AgentMemoryError::InvalidInput(
                    "memory directory must not be a symbolic link".into(),
                ));
            }
            fs::create_dir_all(directory).map_err(io_error)?;
        }
        Ok(())
    }

    pub(crate) fn resource_path(&self, id: &str) -> Result<PathBuf, AgentMemoryError> {
        let path = Path::new(id);
        if id.is_empty()
            || id.len() > 256
            || path.is_absolute()
            || path.components().any(|component| {
                !matches!(component, Component::Normal(_))
                    || component.as_os_str().to_string_lossy().starts_with('.')
            })
            || !id.ends_with(".md")
        {
            return Err(AgentMemoryError::InvalidInput(
                "memory resource id must be a visible relative Markdown path".into(),
            ));
        }
        let candidate = self.artifacts.join(id);
        if candidate
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(AgentMemoryError::InvalidInput(
                "memory resources must not be symbolic links".into(),
            ));
        }
        Ok(candidate)
    }
}

pub(crate) fn io_error(error: std::io::Error) -> AgentMemoryError {
    AgentMemoryError::Io(error.to_string())
}
