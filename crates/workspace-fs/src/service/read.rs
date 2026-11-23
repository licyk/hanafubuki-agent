use std::io::{BufRead, BufReader};

use super::*;

impl WorkspaceFsService {
    pub(super) fn read_file(
        &self,
        file: ExistingPathPlan,
        start: ReadFileStart,
        end_line: Option<u64>,
        max_bytes: usize,
        line_numbers: bool,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        revalidate_existing(&file)?;
        let handle = File::open(file.resolved.system()).map_err(|error| {
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
        let mut reader = BufReader::with_capacity(IO_CHUNK_BYTES, handle);
        let (offset, start_line) = locate_start(&mut reader, start, cancellation)?;
        let mut bytes = Vec::with_capacity(max_bytes.min(file.identity.size as usize));
        let mut line = start_line;
        while bytes.len() <= max_bytes && end_line.is_none_or(|end| line <= end) {
            check_cancel(cancellation)?;
            let chunk = reader.fill_buf().map_err(read_error)?;
            if chunk.is_empty() {
                break;
            }
            let chunk = &chunk[..chunk.len().min(max_bytes + 1 - bytes.len())];
            let length = if let Some(end) = end_line {
                advance_to_line(chunk, &mut line, end.saturating_add(1))
            } else {
                chunk.len()
            };
            bytes.extend_from_slice(&chunk[..length]);
            reader.consume(length);
        }
        let completed_identity = identity_from_metadata(
            &file.resolved,
            &reader
                .get_ref()
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
        let byte_page = matches!(start, ReadFileStart::Offset(_)) && end_line.is_none();
        let lines = if line_numbers && (byte_page || !content.is_empty()) {
            numbered_lines(&content, start_line)
        } else {
            Vec::new()
        };
        let next_offset = offset.saturating_add(content_bytes);
        Ok(WorkspaceFsResult::FileRead(FileReadResult {
            path: file.identity.path,
            content,
            sha256: content_sha256,
            offset,
            start_line: Some(start_line),
            lines,
            next_offset,
            bytes: content_bytes,
            total_bytes: file.identity.size,
            truncated,
        }))
    }
}

/// `content`'s lines with their numbers in the file, the first of them
/// `start_line`. A trailing blank line that only ends the last one is not a
/// line of its own, as a reader counts them.
fn numbered_lines(content: &str, start_line: u64) -> Vec<NumberedLine> {
    let body = content.strip_suffix('\n').unwrap_or(content);
    body.split('\n')
        .enumerate()
        .map(|(index, text)| NumberedLine {
            number: start_line.saturating_add(index as u64),
            text: text.strip_suffix('\r').unwrap_or(text).to_string(),
        })
        .collect()
}

/// Scan in bounded chunks and leave the reader at the selected byte or line.
fn locate_start(
    reader: &mut impl BufRead,
    start: ReadFileStart,
    cancellation: &CancellationToken,
) -> Result<(usize, u64), WorkspaceFsError> {
    let mut offset = 0;
    let mut line = 1;
    loop {
        match start {
            ReadFileStart::Offset(target) if offset == target => break,
            ReadFileStart::Line(target) if line == target => break,
            _ => {}
        }
        check_cancel(cancellation)?;
        let chunk = reader.fill_buf().map_err(read_error)?;
        if chunk.is_empty() {
            break;
        }
        let length = match start {
            ReadFileStart::Offset(target) => {
                let length = chunk.len().min(target - offset);
                line += chunk[..length]
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count() as u64;
                length
            }
            ReadFileStart::Line(target) => advance_to_line(chunk, &mut line, target),
        };
        reader.consume(length);
        offset += length;
    }
    Ok((offset, line))
}

/// Consume through the newline before `target`, or the whole chunk.
fn advance_to_line(chunk: &[u8], line: &mut u64, target: u64) -> usize {
    for (index, &byte) in chunk.iter().enumerate() {
        if byte == b'\n' {
            *line += 1;
            if *line == target {
                return index + 1;
            }
        }
    }
    chunk.len()
}

fn read_error(error: std::io::Error) -> WorkspaceFsError {
    WorkspaceFsError::Io(format!("read file: {error}"))
}
