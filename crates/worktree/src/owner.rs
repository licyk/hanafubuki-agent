// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/worktree/src/metadata.rs` (the owner record in the worktree's
// own Git metadata, bound once) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the record names an Agent session; it is
// read tolerantly (unknown fields ignored, a record that does not read is
// quarantined and the worktree has no owner) and written as a temporary
// file renamed over it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::persistence::quarantine_file;
use hanafubuki_agent_process::executor::ExecutorManager;
use serde::{Deserialize, Serialize};

use crate::git::git_path;
use crate::WorktreeError;

/// In the worktree's own administrative directory of the repository, so it
/// goes with the worktree.
const OWNER_FILE: &str = "hanafubuki-agent-owner.json";

#[derive(Deserialize, Serialize)]
struct OwnerRecord {
    #[serde(default)]
    session: Option<String>,
}

/// The session the worktree at `checkout` belongs to.
pub(crate) fn owner(
    executor: &ExecutorManager,
    checkout: &Path,
) -> Result<Option<String>, WorktreeError> {
    read(&record_path(executor, checkout)?)
}

/// Binds the worktree at `checkout` to `session`; binding it to the session
/// it belongs to again changes nothing, and a worktree another session
/// owns is refused.
pub(crate) fn bind(
    executor: &ExecutorManager,
    checkout: &Path,
    session: &str,
) -> Result<(), WorktreeError> {
    let path = record_path(executor, checkout)?;
    match read(&path)? {
        Some(owner) if owner == session => return Ok(()),
        Some(owner) => return Err(WorktreeError::Owned(owner)),
        None => {}
    }
    let record = OwnerRecord {
        session: Some(session.to_string()),
    };
    let bytes =
        serde_json::to_vec(&record).map_err(|error| WorktreeError::Io(error.to_string()))?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })()
    .map_err(|error| WorktreeError::Io(format!("write {}: {error}", path.display())))
}

fn record_path(executor: &ExecutorManager, checkout: &Path) -> Result<PathBuf, WorktreeError> {
    git_path(
        executor,
        checkout,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            OWNER_FILE,
        ],
    )
}

fn read(path: &Path) -> Result<Option<String>, WorktreeError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(WorktreeError::Io(format!(
                "read {}: {error}",
                path.display()
            )))
        }
    };
    match serde_json::from_slice::<OwnerRecord>(&bytes) {
        Ok(record) => Ok(record.session.filter(|session| !session.is_empty())),
        Err(error) => {
            log::warn!(
                "the worktree owner record {} does not read and is quarantined: {error}",
                path.display()
            );
            quarantine_file(path).map_err(WorktreeError::Io)?;
            Ok(None)
        }
    }
}
