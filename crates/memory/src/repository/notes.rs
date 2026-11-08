use super::*;

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};

use crate::{
    redact_memory_text, AdHocMemoryNoteOperation, AppendAdHocMemoryNote,
    MAX_AD_HOC_MEMORY_NOTE_BYTES,
};

const MAX_AD_HOC_MEMORY_LOG_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AD_HOC_MEMORY_NOTES: usize = 4096;
const AD_HOC_MEMORY_NOTE_VERSION: u8 = 1;

impl AgentMemoryRepository {
    pub fn append_ad_hoc_note(
        &self,
        request: AppendAdHocMemoryNote,
    ) -> Result<AdHocMemoryNote, AgentMemoryError> {
        validate_id(&request.session_id, "note session id")?;
        validate_text(
            &request.content,
            MAX_AD_HOC_MEMORY_NOTE_BYTES,
            "ad-hoc memory note",
        )?;
        if request.content.trim().is_empty() {
            return Err(AgentMemoryError::InvalidInput(
                "ad-hoc memory note must not be empty".into(),
            ));
        }
        let content = redact_memory_text(request.content.trim(), &[]);
        let _guard = self.inner.gate.lock();
        reject_note_path_symlink(&self.inner.paths.ad_hoc_notes)?;
        let connection = self.connection()?;
        let (reset_epoch, enabled): (i64, bool) = connection
            .query_row(
                "SELECT meta.reset_epoch,
                        EXISTS(SELECT 1 FROM memory_sources source
                               WHERE source.session_id=?1
                                 AND source.contribution_mode='enabled'
                                 AND source.revoked=0)
                 FROM memory_meta meta WHERE meta.singleton=1",
                [&request.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error)?;
        if !enabled {
            return Err(AgentMemoryError::InvalidInput(
                "the originating session is not enabled for memory contribution".into(),
            ));
        }
        let reset_epoch = from_i64(reset_epoch)?;
        let note_id = note_id(
            reset_epoch,
            &request.session_id,
            request.turn_id,
            request.source_revision,
            request.operation,
            &content,
        )?;
        let existing = read_all_notes(&self.inner.paths.ad_hoc_notes)?;
        if existing.iter().any(|note| note.note_id == note_id) {
            return Err(AgentMemoryError::InvalidInput(
                "duplicate ad-hoc memory note".into(),
            ));
        }
        let note = AdHocMemoryNote {
            version: AD_HOC_MEMORY_NOTE_VERSION,
            note_id,
            created_at_millis: request.created_at_millis,
            session_id: request.session_id,
            turn_id: request.turn_id,
            source_revision: request.source_revision,
            operation: request.operation,
            content,
            reset_epoch,
        };
        let mut encoded = serde_json::to_vec(&note).map_err(json_error)?;
        encoded.push(b'\n');
        let current_bytes = fs::metadata(&self.inner.paths.ad_hoc_notes)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if current_bytes.saturating_add(encoded.len() as u64) > MAX_AD_HOC_MEMORY_LOG_BYTES {
            return Err(AgentMemoryError::InvalidInput(
                "ad-hoc memory note log exceeds its byte bound".into(),
            ));
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.paths.ad_hoc_notes)
            .map_err(io_error)?;
        file.write_all(&encoded).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        connection
            .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
            .map_err(db_error)?;
        Ok(note)
    }

    pub(super) fn active_ad_hoc_notes(&self) -> Result<Vec<AdHocMemoryNote>, AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        reject_note_path_symlink(&self.inner.paths.ad_hoc_notes)?;
        let connection = self.connection()?;
        active_ad_hoc_notes_from_connection(&connection, &self.inner.paths.ad_hoc_notes)
    }

    pub(super) fn recover_ad_hoc_notes(&self) -> Result<(), AgentMemoryError> {
        let _guard = self.inner.gate.lock();
        reject_note_path_symlink(&self.inner.paths.ad_hoc_notes)?;
        let connection = self.connection()?;
        let (generation, dirty): (i64, bool) = connection
            .query_row(
                "SELECT generation, dirty FROM memory_meta WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error)?;
        let generation = from_i64(generation)?;
        let active_ids =
            active_ad_hoc_notes_from_connection(&connection, &self.inner.paths.ad_hoc_notes)?
                .into_iter()
                .map(|note| note.note_id)
                .collect::<BTreeSet<_>>();
        let (published_ids, manifest_valid) = if generation == 0 {
            (BTreeSet::new(), true)
        } else {
            match super::artifacts::validated_manifest_note_ids(
                &self.inner.paths.artifacts,
                generation,
            ) {
                Ok(ids) => (ids, true),
                Err(_) => (BTreeSet::new(), false),
            }
        };
        if !dirty && (!manifest_valid || active_ids != published_ids) {
            connection
                .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
                .map_err(db_error)?;
        }
        Ok(())
    }
}

pub(super) fn active_ad_hoc_notes_from_connection(
    connection: &rusqlite::Connection,
    path: &Path,
) -> Result<Vec<AdHocMemoryNote>, AgentMemoryError> {
    reject_note_path_symlink(path)?;
    let reset_epoch: i64 = connection
        .query_row(
            "SELECT reset_epoch FROM memory_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let reset_epoch = from_i64(reset_epoch)?;
    let mut statement = connection
        .prepare(
            "SELECT session_id FROM memory_sources
             WHERE contribution_mode='enabled' AND revoked=0",
        )
        .map_err(db_error)?;
    let enabled = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_error)?
        .map(|row| row.map_err(db_error))
        .collect::<Result<BTreeSet<_>, AgentMemoryError>>()?;
    let mut notes = read_all_notes(path)?
        .into_iter()
        .filter(|note| note.reset_epoch == reset_epoch && enabled.contains(&note.session_id))
        .collect::<Vec<_>>();
    if notes.len() > MAX_AD_HOC_MEMORY_NOTES {
        notes = notes.split_off(notes.len() - MAX_AD_HOC_MEMORY_NOTES);
    }
    Ok(notes)
}

/// Every note in the log. The log as a whole past its byte bound is corrupt;
/// one record that does not read (a torn append, a hand edit, a note whose
/// provenance does not match its content, a repeated note) is skipped and
/// logged by line number, never by content, and the others are kept.
fn read_all_notes(path: &Path) -> Result<Vec<AdHocMemoryNote>, AgentMemoryError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    if file.metadata().map_err(io_error)?.len() > MAX_AD_HOC_MEMORY_LOG_BYTES {
        return Err(AgentMemoryError::Corrupt(
            "ad-hoc memory note log exceeds its byte bound".into(),
        ));
    }
    let mut notes = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in BufReader::new(file).split(b'\n').enumerate() {
        let line = line.map_err(io_error)?;
        if line.is_empty() {
            continue;
        }
        match read_note(&line) {
            Ok(note) if ids.insert(note.note_id.clone()) => notes.push(note),
            Ok(_) => skip_note_line(index, "its note ID repeats an earlier note"),
            Err(reason) => skip_note_line(index, reason),
        }
    }
    Ok(notes)
}

fn read_note(line: &[u8]) -> Result<AdHocMemoryNote, &'static str> {
    if line.len() > MAX_AD_HOC_MEMORY_NOTE_BYTES.saturating_add(4096) {
        return Err("it exceeds the record byte bound");
    }
    let note: AdHocMemoryNote =
        serde_json::from_slice(line).map_err(|_| "it is not a note record")?;
    validate_note(&note).map_err(|_| "its provenance or content is invalid")?;
    Ok(note)
}

fn skip_note_line(index: usize, reason: &str) {
    log::warn!(
        "skipping ad-hoc memory note line={}: {reason}",
        index.saturating_add(1)
    );
}

fn validate_note(note: &AdHocMemoryNote) -> Result<(), AgentMemoryError> {
    validate_id(&note.session_id, "note session id")?;
    validate_text(
        &note.content,
        MAX_AD_HOC_MEMORY_NOTE_BYTES,
        "ad-hoc memory note",
    )?;
    if note.version != AD_HOC_MEMORY_NOTE_VERSION
        || note.content.trim().is_empty()
        || !valid_note_id(&note.note_id)
        || note.note_id
            != note_id(
                note.reset_epoch,
                &note.session_id,
                note.turn_id,
                note.source_revision,
                note.operation,
                &note.content,
            )?
    {
        return Err(AgentMemoryError::Corrupt(
            "ad-hoc memory note provenance is invalid".into(),
        ));
    }
    Ok(())
}

fn note_id(
    reset_epoch: u64,
    session_id: &str,
    turn_id: u64,
    source_revision: u64,
    operation: AdHocMemoryNoteOperation,
    content: &str,
) -> Result<String, AgentMemoryError> {
    serde_json::to_vec(&(
        AD_HOC_MEMORY_NOTE_VERSION,
        reset_epoch,
        session_id,
        turn_id,
        source_revision,
        operation,
        content,
    ))
    .map(|bytes| hex_digest(&bytes))
    .map_err(json_error)
}

fn valid_note_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn reject_note_path_symlink(path: &Path) -> Result<(), AgentMemoryError> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(AgentMemoryError::InvalidInput(
            "ad-hoc memory note log must not be a symbolic link".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
