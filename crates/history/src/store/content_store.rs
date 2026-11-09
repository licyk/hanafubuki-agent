//! A session's content store: the originals `hierarchical`'s content store
//! replaced, which the kernel keeps apart from the records and writes whole
//! when it grew. One file, `attachments/<session>/compression-store.json`,
//! so deleting the session removes it with the rest of its attachments; the
//! blob sweeps only read `blobs/`. A file that cannot be read is set aside
//! beside it (`compression-store.unreadable-<millis>.json`) and reads as
//! none: the kernel stores the originals the window still holds again.

use crate::store::*;

const CONTENT_STORE_FILE: &str = "compression-store.json";

impl FileAgentHistoryStore {
    pub(in crate::store) fn write_compression_store_inner(
        &self,
        session_id: AgentSessionId,
        store: &hanafubuki_agent_kernel::AgentContentStore,
    ) -> Result<(), AgentHistoryStoreError> {
        self.ensure_session_exists(session_id)?;
        let session_root = self.session_attachments_root(session_id)?;
        let staging = session_root.join("staging");
        fs::create_dir_all(&staging).map_err(io_error)?;
        let temporary = staging.join(format!("{CONTENT_STORE_FILE}-{}.tmp", now_millis()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(&serde_json::to_vec(store).map_err(json_error)?)
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        publish_file(&temporary, &session_root.join(CONTENT_STORE_FILE)).map_err(io_error)?;
        sync_directory(&session_root)
    }

    pub(in crate::store) fn load_compression_store_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<hanafubuki_agent_kernel::AgentContentStore>, AgentHistoryStoreError> {
        let session_root = self.session_attachments_root(session_id)?;
        let path = session_root.join(CONTENT_STORE_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        match serde_json::from_slice(&bytes) {
            Ok(store) => Ok(Some(store)),
            Err(error) => {
                let aside = session_root.join(format!(
                    "compression-store.unreadable-{}.json",
                    now_millis()
                ));
                fs::rename(&path, &aside).map_err(io_error)?;
                log::warn!(
                    "Agent session {session_id} content store set aside: it could not be read: {error}"
                );
                Ok(None)
            }
        }
    }

    fn session_attachments_root(
        &self,
        session_id: AgentSessionId,
    ) -> Result<PathBuf, AgentHistoryStoreError> {
        let session_root = self
            .inner
            .paths
            .attachments
            .join(session_id.value().to_string());
        ensure_scoped_path(&session_root, &self.inner.paths.attachments)?;
        Ok(session_root)
    }
}
