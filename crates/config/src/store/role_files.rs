//! Role sources are readable; only files under the user's agents directory are writable.
use super::AgentConfig;
use crate::file::resolve_destination;
use crate::file::{ConfigFile, FileAccess, FileRead};
use crate::keys;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

impl AgentConfig {
    pub fn read_role_document(&self, path: &Path) -> Result<Map<String, Value>, String> {
        let path = resolve_destination(path)?;
        match ConfigFile::new(path.system().to_path_buf(), FileAccess::Shared).read()? {
            FileRead::Document(document) => Ok(document),
            FileRead::Missing => Ok(Map::new()),
            FileRead::Unusable(reason) => Err(reason),
        }
    }

    pub fn user_role_file(&self, name: &str) -> Result<PathBuf, String> {
        if !crate::is_role_name(name) || name.contains(['/', '\\']) || matches!(name, "." | "..") {
            return Err("Use a role name without path separators".into());
        }
        let directory = self
            .inner
            .settings_file
            .path()
            .parent()
            .ok_or("Configuration has no directory")?;
        Ok(directory.join("agents").join(format!("{name}.toml")))
    }

    pub fn role_file_is_writable(&self, path: &Path) -> Result<bool, String> {
        let root = self
            .inner
            .settings_file
            .path()
            .parent()
            .ok_or("Configuration has no directory")?
            .join("agents");
        let root = resolve_destination(&root)?;
        let path = resolve_destination(path)?;
        Ok(path.key().starts_with(root.key()) && path.key() != root.key())
    }

    /// The caller has checked fields with the role schema; keep unknown keys and comments.
    pub fn write_user_role_file(
        &self,
        path: &Path,
        values: &Map<String, Value>,
    ) -> Result<(), String> {
        if !self.role_file_is_writable(path)? {
            return Err("Only files in the user's agents directory are writable here".into());
        }
        let mut cached = self.inner.settings.lock();
        self.read_role_document(path)?;
        let mut patch = Map::new();
        for (key, value) in values {
            keys::insert(&mut patch, &keys::parse_key(key)?, value.clone());
        }
        let path = resolve_destination(path)?;
        ConfigFile::new(path.system().to_path_buf(), FileAccess::Shared).write(&patch)?;
        *cached = None;
        Ok(())
    }
}
