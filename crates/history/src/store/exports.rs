//! A session's exports: text files the Agent writes for the model to read
//! (`decompress` with `toFile`). They live beside the session's image blobs,
//! in `attachments/<session>/exports`, so deleting the session removes them
//! with the rest of its attachments; the blob sweeps never look there.

use crate::store::*;
use hanafubuki_agent_foundation::paths::{PathError, ResolvedPath};

impl FileAgentHistoryStore {
    pub(in crate::store) fn write_session_export_inner(
        &self,
        session_id: AgentSessionId,
        file_name: &str,
        contents: &str,
    ) -> Result<String, AgentHistoryStoreError> {
        self.ensure_session_exists(session_id)?;
        let session_root = self
            .inner
            .paths
            .attachments
            .join(session_id.value().to_string());
        ensure_scoped_path(&session_root, &self.inner.paths.attachments)?;
        let staging = session_root.join("staging");
        let exports = session_root.join("exports");
        fs::create_dir_all(&staging).map_err(io_error)?;
        fs::create_dir_all(&exports).map_err(io_error)?;
        let path_error = |error: PathError| AgentHistoryStoreError::Io(error.to_string());
        let exports = ResolvedPath::resolve(&exports).map_err(path_error)?;
        let target = ResolvedPath::resolve_child(&exports, std::ffi::OsStr::new(file_name))
            .map_err(path_error)?;
        let temporary = staging.join(format!("{file_name}-{}.tmp", now_millis()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(contents.as_bytes()).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        publish_file(&temporary, target.system()).map_err(io_error)?;
        sync_directory(exports.system())?;
        Ok(target.native_text().to_owned())
    }

    fn ensure_session_exists(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let exists: i64 = self
            .connection()?
            .query_row(
                "SELECT COUNT(*) FROM agent_sessions WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if exists == 0 {
            return Err(AgentHistoryStoreError::NotFound { session_id });
        }
        Ok(())
    }
}
