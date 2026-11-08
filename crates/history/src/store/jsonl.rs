use crate::store::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentHistoryFormatProbe {
    format_version: u32,
}

fn decode_record(bytes: &[u8]) -> Result<AgentHistoryRecord, AgentHistoryStoreError> {
    let probe: AgentHistoryFormatProbe = serde_json::from_slice(bytes).map_err(json_error)?;
    if probe.format_version != AGENT_HISTORY_FORMAT_VERSION {
        return Err(AgentHistoryStoreError::IncompatibleFormat {
            found: probe.format_version,
            expected: AGENT_HISTORY_FORMAT_VERSION,
        });
    }
    serde_json::from_slice(bytes).map_err(json_error)
}

pub(in crate::store) fn append_commit_to_file(
    path: &Path,
    file: &mut Option<File>,
    next_ordinal: &mut u64,
    last_revision: &mut Option<u64>,
    commit: PendingHistoryCommit,
) -> Result<(), AgentHistoryStoreError> {
    if last_revision.is_some_and(|revision| commit.session_revision.value() < revision) {
        return Err(AgentHistoryStoreError::StaleRevision);
    }
    if file.is_none() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        repair_partial_tail(path)?;
        *file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(path)
                .map_err(io_error)?,
        );
    }
    let record = AgentHistoryRecord {
        format_version: AGENT_HISTORY_FORMAT_VERSION,
        session_id: commit.session_id,
        ordinal: *next_ordinal,
        recorded_at: now_millis(),
        commit_id: format!(
            "{}-{}-{}",
            commit.session_id,
            *next_ordinal,
            commit.session_revision.value()
        ),
        session_revision: commit.session_revision,
        session_updated_at: commit.session_updated_at,
        payload: commit.payload,
    };
    let mut encoded = serde_json::to_vec(&record).map_err(json_error)?;
    encoded.push(b'\n');
    // Opened above when it was not yet.
    let file = file.as_mut().expect("writer file was opened");
    file.write_all(&encoded).map_err(io_error)?;
    file.sync_data().map_err(io_error)?;
    *next_ordinal = next_ordinal.checked_add(1).ok_or_else(|| {
        AgentHistoryStoreError::CorruptHistory("record ordinal exhausted".to_string())
    })?;
    *last_revision = Some(record.session_revision.value());
    Ok(())
}

pub(in crate::store) fn read_writer_state(
    path: &Path,
) -> Result<(u64, Option<u64>), AgentHistoryStoreError> {
    if !path.exists() {
        return Ok((0, None));
    }
    repair_partial_tail(path)?;
    let file = File::open(path).map_err(io_error)?;
    let mut next = 0;
    let mut last_revision = None;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(io_error)?;
        if line.trim().is_empty() {
            continue;
        }
        let record = decode_record(line.as_bytes())?;
        if record.ordinal != next {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "expected record ordinal {next}, got {}",
                record.ordinal
            )));
        }
        if last_revision.is_some_and(|revision| record.session_revision.value() < revision) {
            return Err(AgentHistoryStoreError::CorruptHistory(
                "session revisions are not monotonic".to_string(),
            ));
        }
        last_revision = Some(record.session_revision.value());
        next = next.checked_add(1).ok_or_else(|| {
            AgentHistoryStoreError::CorruptHistory("record ordinal exhausted".to_string())
        })?;
    }
    Ok((next, last_revision))
}

pub(in crate::store) fn repair_partial_tail(path: &Path) -> Result<(), AgentHistoryStoreError> {
    if !path.exists() {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    let len = file.metadata().map_err(io_error)?.len();
    if len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1)).map_err(io_error)?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last).map_err(io_error)?;
    if last[0] == b'\n' {
        return Ok(());
    }
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    file.set_len(complete_len as u64).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

pub(in crate::store) fn collect_jsonl_files(
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), AgentHistoryStoreError> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let file_type = entry.file_type().map_err(io_error)?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_jsonl_files(&path, output)?;
        } else if file_type.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("jsonl")
        {
            output.push(path);
        }
    }
    Ok(())
}

pub(in crate::store) fn session_id_from_rollout_path(path: &Path) -> Option<AgentSessionId> {
    let stem = path.file_stem()?.to_str()?;
    let raw = stem.rsplit_once('-')?.1.parse::<u64>().ok()?;
    Some(AgentSessionId::new(raw))
}

pub(in crate::store) struct CompleteRecord {
    pub(in crate::store) record: AgentHistoryRecord,
    pub(in crate::store) start_offset: u64,
    pub(in crate::store) end_offset: u64,
}

pub(in crate::store) struct CompleteRecords {
    pub(in crate::store) records: Vec<CompleteRecord>,
    pub(in crate::store) next_offset: u64,
}

pub(in crate::store) fn read_complete_records(
    path: &Path,
    start_offset: u64,
) -> Result<CompleteRecords, AgentHistoryStoreError> {
    let mut file = File::open(path).map_err(io_error)?;
    let file_len = file.metadata().map_err(io_error)?.len();
    if start_offset > file_len {
        return Err(AgentHistoryStoreError::CorruptHistory(
            "projection byte offset is past the end of JSONL".to_string(),
        ));
    }
    file.seek(SeekFrom::Start(start_offset)).map_err(io_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let mut records = Vec::new();
    let mut line_start = start_offset;
    for line in bytes[..complete_len].split_inclusive(|byte| *byte == b'\n') {
        let line_end = line_start.checked_add(line.len() as u64).ok_or_else(|| {
            AgentHistoryStoreError::CorruptHistory("byte offset exhausted".to_string())
        })?;
        if line.iter().any(|byte| !byte.is_ascii_whitespace()) {
            let record = decode_record(line)?;
            records.push(CompleteRecord {
                record,
                start_offset: line_start,
                end_offset: line_end,
            });
        }
        line_start = line_end;
    }
    Ok(CompleteRecords {
        records,
        next_offset: start_offset + complete_len as u64,
    })
}

pub(in crate::store) fn validate_record(
    expected_session_id: AgentSessionId,
    expected_ordinal: u64,
    record: &AgentHistoryRecord,
) -> Result<(), AgentHistoryStoreError> {
    if record.format_version != AGENT_HISTORY_FORMAT_VERSION {
        return Err(AgentHistoryStoreError::IncompatibleFormat {
            found: record.format_version,
            expected: AGENT_HISTORY_FORMAT_VERSION,
        });
    }
    if record.session_id != expected_session_id {
        return Err(AgentHistoryStoreError::CorruptHistory(
            "history record belongs to another session".to_string(),
        ));
    }
    if record.ordinal != expected_ordinal {
        return Err(AgentHistoryStoreError::CorruptHistory(format!(
            "expected record ordinal {expected_ordinal}, got {}",
            record.ordinal
        )));
    }
    if let Some(runtime) = &record.payload.runtime_checkpoint {
        if runtime.id != expected_session_id
            || record.session_revision != runtime.revision
            || record.session_updated_at != runtime.updated_at
        {
            return Err(AgentHistoryStoreError::CorruptHistory(
                "record identity does not match its runtime checkpoint".to_string(),
            ));
        }
    } else if record.ordinal == 0 {
        return Err(AgentHistoryStoreError::CorruptHistory(
            "the initial history record is not a runtime checkpoint".to_string(),
        ));
    }
    Ok(())
}
