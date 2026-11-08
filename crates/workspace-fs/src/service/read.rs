use super::*;

impl WorkspaceFsService {
    pub(super) fn read_file(
        &self,
        file: ExistingPathPlan,
        offset: usize,
        max_bytes: usize,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        revalidate_existing(&file)?;
        let mut handle = File::open(file.resolved.system()).map_err(|error| {
            WorkspaceFsError::Io(format!("open {}: {error}", file.identity.path))
        })?;
        let handle_identity = identity_from_metadata(
            &file.resolved,
            &handle
                .metadata()
                .map_err(|error| WorkspaceFsError::Io(format!("inspect opened file: {error}")))?,
        )?;
        if handle_identity != file.identity {
            return Err(WorkspaceFsError::PathChanged);
        }
        handle
            .seek(std::io::SeekFrom::Start(offset as u64))
            .map_err(|error| WorkspaceFsError::Io(format!("seek file: {error}")))?;
        let mut bytes = Vec::with_capacity(max_bytes.min(file.identity.size as usize));
        let mut chunk = vec![0u8; IO_CHUNK_BYTES.min(max_bytes.saturating_add(1))];
        while bytes.len() <= max_bytes {
            check_cancel(cancellation)?;
            let remaining = max_bytes.saturating_add(1).saturating_sub(bytes.len());
            if remaining == 0 {
                break;
            }
            let read_len = chunk.len().min(remaining);
            let read = handle
                .read(&mut chunk[..read_len])
                .map_err(|error| WorkspaceFsError::Io(format!("read file: {error}")))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        let completed_identity = identity_from_metadata(
            &file.resolved,
            &handle
                .metadata()
                .map_err(|error| WorkspaceFsError::Io(format!("inspect read file: {error}")))?,
        )?;
        if completed_identity != file.identity {
            return Err(WorkspaceFsError::PathChanged);
        }
        let truncated = bytes.len() > max_bytes;
        bytes.truncate(max_bytes);
        if bytes.contains(&0) {
            return Err(WorkspaceFsError::BinaryFile);
        }
        let content = bounded_utf8(bytes, truncated)?;
        let content_sha256 = sha256_hex(content.as_bytes());
        let content_bytes = content.len();
        let next_offset = offset.saturating_add(content_bytes);
        Ok(WorkspaceFsResult::FileRead(FileReadResult {
            path: file.identity.path,
            content,
            sha256: content_sha256,
            offset,
            next_offset,
            bytes: content_bytes,
            total_bytes: file.identity.size,
            truncated,
        }))
    }
}
