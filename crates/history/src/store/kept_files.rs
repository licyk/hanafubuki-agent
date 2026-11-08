//! Session files the history keeps but the index does not hold: rollouts
//! `open` moved into a `.recovery` directory (of a format this build does
//! not read) and rollouts refused at open that stay in place with a pending
//! repair. They are never decoded. What the store must know of them, the
//! attachments they name and the sessions they fork from, is read from
//! their bytes line by line, and a match counts even where it is not one:
//! the result only ever keeps more.

use crate::store::jsonl::session_id_from_rollout_path;
use crate::store::*;
use hanafubuki_agent_kernel::AgentAttachmentId;

/// An attachment id is a SHA-256 digest in hexadecimal.
const DIGEST_HEX_LENGTH: usize = 64;

/// The key of a runtime checkpoint's source session.
const FORKED_FROM_KEY: &[u8] = b"\"forkedFromSessionId\"";
/// The key of a runtime checkpoint's history base, the session whose rollout
/// holds the fork's prefix (`agent_session_lineage`).
const HISTORY_BASE_KEY: &[u8] = b"\"historyBase\"";
const SESSION_ID_KEY: &[u8] = b"\"sessionId\"";

/// What a kept file names.
#[derive(Default)]
pub(in crate::store) struct KeptFileReferences {
    /// Every 64-digit hexadecimal run: the attachment digests it may
    /// reference, whichever session owns the blob.
    pub(in crate::store) digests: HashSet<AgentAttachmentId>,
    /// The sessions it forks from: every `forkedFromSessionId` and every
    /// `historyBase` `sessionId` it holds.
    pub(in crate::store) parents: HashSet<AgentSessionId>,
}

impl FileAgentHistoryStore {
    /// Every file under a `.recovery` directory of the session trees.
    pub(in crate::store) fn recovery_files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        for tree in [
            &self.inner.paths.active_sessions,
            &self.inner.paths.archived_sessions,
        ] {
            collect_recovery_files(tree, false, &mut files)?;
        }
        Ok(files)
    }
}

/// Adds every file under a `.recovery` directory of `directory`.
fn collect_recovery_files(
    directory: &Path,
    in_recovery: bool,
    found: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    let read_error = |error: std::io::Error| format!("{}: {error}", directory.display());
    for entry in fs::read_dir(directory).map_err(read_error)? {
        let entry = entry.map_err(read_error)?;
        let file_type = entry.file_type().map_err(read_error)?;
        let path = entry.path();
        if file_type.is_dir() {
            let in_recovery = in_recovery || entry.file_name() == ".recovery";
            collect_recovery_files(&path, in_recovery, found)?;
        } else if in_recovery {
            found.push(path);
        }
    }
    Ok(())
}

/// The session a file `quarantine_file` moved aside belonged to, when its
/// name (`<rollout name>.<pid>.<n>.invalid`) says it.
pub(in crate::store) fn quarantined_session_id(path: &Path) -> Option<AgentSessionId> {
    let name = path.file_name()?.to_str()?.strip_suffix(".invalid")?;
    let original = name.rsplitn(3, '.').nth(2)?;
    session_id_from_rollout_path(Path::new(original))
}

/// Reads what `path` names, as bytes.
pub(in crate::store) fn read_kept_file(path: &Path) -> std::io::Result<KeptFileReferences> {
    let mut references = KeptFileReferences::default();
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(references);
        }
        collect_digests(&line, &mut references.digests);
        collect_parents(&line, &mut references.parents);
    }
}

fn collect_digests(line: &[u8], found: &mut HashSet<AgentAttachmentId>) {
    for run in line.split(|byte| !byte.is_ascii_hexdigit()) {
        if run.len() != DIGEST_HEX_LENGTH {
            continue;
        }
        if let Ok(id) = AgentAttachmentId::parse(String::from_utf8_lossy(run).into_owned()) {
            found.insert(id);
        }
    }
}

/// Every `"forkedFromSessionId": <n>` and every `"sessionId": <n>` inside a
/// `"historyBase": {..}` object (whose fields are all numbers) of `line`.
fn collect_parents(line: &[u8], found: &mut HashSet<AgentSessionId>) {
    for value in values_after(line, FORKED_FROM_KEY) {
        found.extend(leading_session_id(value));
    }
    for value in values_after(line, HISTORY_BASE_KEY) {
        let Some(object) = value.strip_prefix(b"{") else {
            continue;
        };
        let object = &object[..object
            .iter()
            .position(|byte| *byte == b'}')
            .unwrap_or(object.len())];
        for value in values_after(object, SESSION_ID_KEY) {
            found.extend(leading_session_id(value));
        }
    }
}

/// What follows each `key` of `bytes` and its colon, whitespace trimmed.
fn values_after<'a>(bytes: &'a [u8], key: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
    let mut rest = bytes;
    std::iter::from_fn(move || loop {
        let at = rest.windows(key.len()).position(|window| window == key)?;
        rest = &rest[at + key.len()..];
        if let Some(value) = rest.trim_ascii_start().strip_prefix(b":") {
            return Some(value.trim_ascii_start());
        }
    })
}

fn leading_session_id(value: &[u8]) -> Option<AgentSessionId> {
    let digits = value
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(value.len());
    std::str::from_utf8(&value[..digits])
        .ok()?
        .parse::<u64>()
        .ok()
        .map(AgentSessionId::new)
}

#[cfg(test)]
mod tests;
