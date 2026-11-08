use crate::store::*;
use image::{ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use std::io::Cursor;

const MAXIMUM_NORMALIZED_IMAGE_BYTES: usize = 8 * 1024 * 1024;

impl FileAgentHistoryStore {
    pub(in crate::store) fn load_visible_image_attachment_inner(
        &self,
        session_id: AgentSessionId,
        attachment: &hanafubuki_agent_kernel::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        let encoded_items = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let mut statement = connection
                .prepare(
                    r#"
WITH RECURSIVE lineage(session_id) AS (
    SELECT ?1
    UNION ALL
    SELECT parent_session_id
    FROM agent_session_lineage
    JOIN lineage ON child_session_id = lineage.session_id
)
SELECT item_json
FROM agent_history_items
JOIN lineage USING (session_id)
ORDER BY session_id, first_item_order
                    "#,
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map(params![to_i64(session_id.value(), "session id")?], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        let mut visible = false;
        for encoded in encoded_items {
            let item: AgentHistoryItem = serde_json::from_str(&encoded).map_err(json_error)?;
            visible |= match item.content {
                ConversationItem::UserMessage { images, .. }
                | ConversationItem::SteeringMessage { images, .. }
                | ConversationItem::ToolOutput { images, .. } => images.contains(attachment),
                _ => false,
            };
            if visible {
                break;
            }
        }
        if !visible {
            return Err(AgentHistoryStoreError::NotFound { session_id });
        }
        self.load_image_attachment_inner(attachment)
    }

    /// Checks that every committed item decodes and that each image it
    /// references is intact. A session that fails is refused
    /// (`projection_lag`, listed by recovery) and the others are checked
    /// on; only reading the index fails the call.
    pub(in crate::store) fn validate_committed_attachments(
        &self,
    ) -> Result<(), AgentHistoryStoreError> {
        let mut failed = HashSet::new();
        for (session_id, encoded) in self.committed_items(None)? {
            if failed.contains(&session_id) {
                continue;
            }
            if let Err(error) = self.validate_committed_item(&encoded) {
                log::error!(
                    "Agent session {session_id} quarantined: a committed item or its attachment cannot be read, its files are kept: {error}"
                );
                failed.insert(session_id);
                self.inner
                    .projection_lag
                    .lock()
                    .entry(session_id)
                    .or_insert_with(|| ProjectionLag::Refused(error.to_string()));
            }
        }
        Ok(())
    }

    /// Checks one session's committed items as start-up does, for a repair.
    pub(in crate::store) fn validate_session_attachments(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        self.committed_items(Some(session_id))?
            .iter()
            .try_for_each(|(_, encoded)| self.validate_committed_item(encoded))
    }

    /// The encoded items of one session, or of all, in order.
    fn committed_items(
        &self,
        session_id: Option<AgentSessionId>,
    ) -> Result<Vec<(AgentSessionId, String)>, AgentHistoryStoreError> {
        let session_id = session_id
            .map(|session_id| to_i64(session_id.value(), "session id"))
            .transpose()?;
        let rows = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let mut statement = connection
                .prepare(
                    "SELECT session_id, item_json FROM agent_history_items WHERE ?1 IS NULL OR session_id = ?1 ORDER BY session_id, first_item_order",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map(params![session_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        rows.into_iter()
            .map(|(session_id, encoded)| {
                Ok((
                    AgentSessionId::new(from_i64(session_id, "session id")?),
                    encoded,
                ))
            })
            .collect()
    }

    fn validate_committed_item(&self, encoded: &str) -> Result<(), AgentHistoryStoreError> {
        let item: AgentHistoryItem = serde_json::from_str(encoded).map_err(json_error)?;
        let images = match &item.content {
            ConversationItem::UserMessage { images, .. }
            | ConversationItem::SteeringMessage { images, .. }
            | ConversationItem::ToolOutput { images, .. } => images,
            _ => return Ok(()),
        };
        for image in images {
            self.load_image_attachment_inner(image)?;
        }
        Ok(())
    }

    pub(in crate::store) fn cleanup_attachment_staging(
        &self,
    ) -> Result<(), AgentHistoryStoreError> {
        if !self.inner.paths.attachments.exists() {
            return Ok(());
        }
        for session in fs::read_dir(&self.inner.paths.attachments).map_err(io_error)? {
            let session = session.map_err(io_error)?;
            if !session.file_type().map_err(io_error)?.is_dir() {
                continue;
            }
            let staging = session.path().join("staging");
            if staging.exists() {
                fs::remove_dir_all(&staging).map_err(io_error)?;
            }
        }
        Ok(())
    }

    pub(in crate::store) fn stage_image_attachment_inner(
        &self,
        session_id: AgentSessionId,
        image: hanafubuki_agent_kernel::PreparedAgentImage,
    ) -> Result<hanafubuki_agent_kernel::AgentImageAttachmentRef, AgentHistoryStoreError> {
        if image.bytes.is_empty() || image.bytes.len() > MAXIMUM_NORMALIZED_IMAGE_BYTES {
            return Err(AgentHistoryStoreError::Io(format!(
                "normalized image byte length must be between 1 and {MAXIMUM_NORMALIZED_IMAGE_BYTES}"
            )));
        }
        if image.width == 0 || image.height == 0 {
            return Err(AgentHistoryStoreError::Io(
                "normalized image dimensions must be non-zero".to_string(),
            ));
        }
        validate_image_metadata(
            &image.bytes,
            &image.mime_type,
            image.width,
            image.height,
            "prepared image",
        )?;
        {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM agent_sessions WHERE session_id = ?",
                    params![to_i64(session_id.value(), "session id")?],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            if exists == 0 {
                return Err(AgentHistoryStoreError::NotFound { session_id });
            }
        }

        let digest = Sha256::digest(&image.bytes);
        let digest_hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let attachment_id = hanafubuki_agent_kernel::AgentAttachmentId::from_sha256(digest_hex);
        let session_root = self
            .inner
            .paths
            .attachments
            .join(session_id.value().to_string());
        ensure_scoped_path(&session_root, &self.inner.paths.attachments)?;
        let staging = session_root.join("staging");
        let blobs = session_root.join("blobs");
        fs::create_dir_all(&staging).map_err(io_error)?;
        fs::create_dir_all(&blobs).map_err(io_error)?;
        let final_path = blobs.join(format!("{}.blob", attachment_id.as_str()));
        if !final_path.exists() {
            let temporary =
                staging.join(format!("{}-{}.tmp", attachment_id.as_str(), now_millis()));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(io_error)?;
            file.write_all(&image.bytes).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            publish_file(&temporary, &final_path).map_err(io_error)?;
            sync_directory(&blobs)?;
            sync_directory(&session_root)?;
        }
        let byte_length = u64::try_from(image.bytes.len()).map_err(|_| {
            AgentHistoryStoreError::Io("attachment byte length exceeds u64".to_string())
        })?;
        Ok(hanafubuki_agent_kernel::AgentImageAttachmentRef {
            attachment_id,
            source_session_id: session_id,
            mime_type: image.mime_type,
            byte_length,
            width: image.width,
            height: image.height,
            display_name: image.display_name,
            detail: image.detail,
        })
    }

    pub(in crate::store) fn load_image_attachment_inner(
        &self,
        attachment: &hanafubuki_agent_kernel::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        let path = self
            .inner
            .paths
            .attachments
            .join(attachment.source_session_id.value().to_string())
            .join("blobs")
            .join(format!("{}.blob", attachment.attachment_id.as_str()));
        ensure_scoped_path(&path, &self.inner.paths.attachments)?;
        let bytes = fs::read(&path).map_err(|error| {
            AgentHistoryStoreError::CorruptHistory(format!(
                "image attachment {} could not be read: {error}",
                attachment.attachment_id.as_str()
            ))
        })?;
        if u64::try_from(bytes.len()).ok() != Some(attachment.byte_length) {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "image attachment {} has an unexpected byte length",
                attachment.attachment_id.as_str()
            )));
        }
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if digest != attachment.attachment_id.as_str() {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "image attachment {} failed digest validation",
                attachment.attachment_id.as_str()
            )));
        }
        validate_image_metadata(
            &bytes,
            &attachment.mime_type,
            attachment.width,
            attachment.height,
            "stored image attachment",
        )?;
        Ok(bytes)
    }
}

fn validate_image_metadata(
    bytes: &[u8],
    expected_mime: &str,
    expected_width: u32,
    expected_height: u32,
    label: &str,
) -> Result<(), AgentHistoryStoreError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| {
            AgentHistoryStoreError::CorruptHistory(format!("decode {label}: {error}"))
        })?;
    let format = reader.format().ok_or_else(|| {
        AgentHistoryStoreError::CorruptHistory(format!("{label} has no recognized image format"))
    })?;
    let actual_mime = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        _ => {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "{label} is not a normalized PNG or JPEG"
            )))
        }
    };
    if actual_mime != expected_mime {
        return Err(AgentHistoryStoreError::CorruptHistory(format!(
            "{label} MIME does not match its bytes"
        )));
    }
    let (width, height) = reader.into_dimensions().map_err(|error| {
        AgentHistoryStoreError::CorruptHistory(format!("decode {label} dimensions: {error}"))
    })?;
    if (width, height) != (expected_width, expected_height) {
        return Err(AgentHistoryStoreError::CorruptHistory(format!(
            "{label} dimensions do not match durable metadata"
        )));
    }
    Ok(())
}
