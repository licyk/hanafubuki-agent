use super::*;

impl WorkspaceFsService {
    pub fn read_selected_binary(
        &self,
        path: &str,
        max_bytes: usize,
        cancellation: &CancellationToken,
    ) -> Result<SelectedBinaryFile, WorkspaceFsError> {
        self.read_bounded_binary(path, max_bytes, cancellation)
    }

    /// Validate one bounded binary read through the same workspace and
    /// protected path policy used by the model-visible filesystem tools.
    pub fn validate_bounded_binary_read(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<(), WorkspaceFsError> {
        self.resolve_bounded_binary(path, max_bytes).map(|_| ())
    }

    /// Read one bounded binary file through the same workspace and protected
    /// path policy used by the model-visible filesystem tools.
    pub fn read_bounded_binary(
        &self,
        path: &str,
        max_bytes: usize,
        cancellation: &CancellationToken,
    ) -> Result<SelectedBinaryFile, WorkspaceFsError> {
        let file = self.resolve_bounded_binary(path, max_bytes)?;
        let mut handle = File::open(file.resolved.system())
            .map_err(|error| WorkspaceFsError::Io(format!("open binary file: {error}")))?;
        let handle_identity = identity_from_metadata(
            &file.resolved,
            &handle
                .metadata()
                .map_err(|error| WorkspaceFsError::Io(format!("inspect binary file: {error}")))?,
        )?;
        if handle_identity != file.identity {
            return Err(WorkspaceFsError::PathChanged);
        }
        let mut bytes = Vec::with_capacity(file.identity.size as usize);
        let mut chunk = vec![0u8; IO_CHUNK_BYTES.min(max_bytes)];
        loop {
            check_cancel(cancellation)?;
            let read = handle
                .read(&mut chunk)
                .map_err(|error| WorkspaceFsError::Io(format!("read selected file: {error}")))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.len() > max_bytes {
                return Err(WorkspaceFsError::InvalidRequest(format!(
                    "binary file exceeds {max_bytes} bytes"
                )));
            }
        }
        Ok(SelectedBinaryFile {
            canonical_path: file.identity.path,
            bytes,
        })
    }

    fn resolve_bounded_binary(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<ExistingPathPlan, WorkspaceFsError> {
        if max_bytes == 0 {
            return Err(WorkspaceFsError::InvalidRequest(
                "binary read maximum bytes must be non-zero".to_string(),
            ));
        }
        let file =
            crate::resolver::resolve_existing(&self.policy, path, Some(WorkspacePathKind::File))?;
        revalidate_existing(&file)?;
        if file.identity.size > max_bytes as u64 {
            return Err(WorkspaceFsError::InvalidRequest(format!(
                "binary file exceeds {max_bytes} bytes"
            )));
        }
        Ok(file)
    }
}

pub(super) fn bounded_utf8(
    mut bytes: Vec<u8>,
    truncated: bool,
) -> Result<String, WorkspaceFsError> {
    match String::from_utf8(bytes) {
        Ok(content) => Ok(content),
        Err(error) if truncated && error.utf8_error().error_len().is_none() => {
            let boundary = error.utf8_error().valid_up_to();
            bytes = error.into_bytes();
            bytes.truncate(boundary);
            String::from_utf8(bytes).map_err(|_| WorkspaceFsError::BinaryFile)
        }
        Err(_) => Err(WorkspaceFsError::BinaryFile),
    }
}

pub(super) fn bounded_preview(line: &str) -> String {
    if line.len() <= MAX_PREVIEW_BYTES {
        return line.to_string();
    }
    let boundary = (0..=MAX_PREVIEW_BYTES)
        .rev()
        .find(|index| line.is_char_boundary(*index))
        .unwrap_or(0);
    format!("{}…", &line[..boundary])
}
