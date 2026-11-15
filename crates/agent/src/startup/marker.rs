use super::CRASH_LOOP_STARTS;
use crate::{data_dir_lock::DataDirLock, AgentDisabledReason};
use hanafubuki_agent_config::{write_atomic, FileAccess};
use hanafubuki_agent_foundation::{
    file_lock::FileLock,
    panic::{read_crash_record, CrashRecord},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_MARKER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartMarkerDocument {
    unfinished_starts: u32,
    /// The run that recorded the last attempt, to pair it with a crash
    /// record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_attempt: Option<StartAttempt>,
    #[serde(default)]
    pending: Vec<PendingStart>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StartAttempt {
    pid: u32,
    at_unix_millis: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingStart {
    id: u64,
    attempt: StartAttempt,
}

// Fatal panics are outside the Agent's contained threads; the host's death
// must not disable its Agent at the next start.
fn counted_starts(document: &StartMarkerDocument, crash: Option<&CrashRecord>) -> u32 {
    match (&document.last_attempt, crash) {
        (Some(attempt), Some(crash))
            if crash.pid == attempt.pid && crash.at_unix_millis >= attempt.at_unix_millis =>
        {
            log::info!(
                "the last Agent start is not counted: its run ended in a fatal panic on thread \
                 '{}' at {}",
                crash.thread,
                crash.location
            );
            document.unfinished_starts.saturating_sub(1)
        }
        _ => document.unfinished_starts,
    }
}

/// The crash-loop start-up marker under the Agent data directory, which
/// also holds the data directory's lock: the directory the marker's file is
/// in is the data directory (`AgentStoragePaths::start_marker_file`).
#[derive(Clone)]
pub struct AgentStartMarker {
    file: PathBuf,
    id: u64,
    lease: Arc<OnceLock<Result<FileLock, String>>>,
    /// The crash record the panic hook writes (`configure_crash_record`).
    crash_record: Option<PathBuf>,
    /// Whether this run recorded an attempt that is not cleared yet.
    recorded: Arc<AtomicBool>,
    /// Taken by the first start and held across restarts until shutdown.
    lock: Arc<Mutex<Option<DataDirLock>>>,
}

impl AgentStartMarker {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            id: NEXT_MARKER.fetch_add(1, Ordering::Relaxed),
            lease: Arc::new(OnceLock::new()),
            crash_record: None,
            recorded: Arc::new(AtomicBool::new(false)),
            lock: Arc::new(Mutex::new(None)),
        }
    }

    /// The marker, discounting a start whose run ended in the fatal panic
    /// recorded at `crash_record`.
    pub fn with_crash_record(self, crash_record: PathBuf) -> Self {
        Self {
            crash_record: Some(crash_record),
            ..self
        }
    }

    /// Starts recorded and never cleared, less one whose run ended in a
    /// fatal panic. An unreadable marker counts none: it must not keep the
    /// Agent off.
    pub fn unfinished_starts(&self) -> u32 {
        let document = self.read_document();
        let crash = self.crash_record.as_deref().and_then(read_crash_record);
        let pending = document
            .pending
            .iter()
            .filter(|pending| {
                !self.is_running(pending) && !ended_in_crash(&pending.attempt, crash.as_ref())
            })
            .count() as u32;
        counted_starts(&document, crash.as_ref()).saturating_add(pending)
    }

    /// Whether the Agent starts this run, its module being enabled or not,
    /// and if not, why.
    pub fn disabled_reason(&self, module_enabled: bool) -> Option<AgentDisabledReason> {
        if !module_enabled {
            log::info!("the agent module is turned off in Settings; the Agent is not built");
            return Some(AgentDisabledReason::Setting);
        }
        let unfinished = self.unfinished_starts();
        if unfinished >= CRASH_LOOP_STARTS {
            log::warn!(
                "the Agent starts off: its last {unfinished} starts did not finish; \
                 restarting it or turning its module off and on clears this"
            );
            return Some(AgentDisabledReason::CrashLoop);
        }
        None
    }

    /// Shares the data directory with other instances, excluding an older
    /// Agent or maintenance operation that holds its exclusive lock.
    pub(crate) fn lock_data_dir(&self) -> Result<(), String> {
        let mut lock = self.lock.lock();
        if lock.is_none() {
            let directory = self.file.parent().unwrap_or(&self.file);
            *lock = Some(DataDirLock::acquire(directory)?);
        }
        Ok(())
    }

    /// Releases the data directory's lock, once the Agent has stopped.
    pub(crate) fn release_data_dir(&self) {
        self.lock.lock().take();
    }

    /// Records a start attempt before the Agent is constructed.
    pub(crate) fn record_attempt(&self) -> Result<(), String> {
        let pending = PendingStart {
            id: self.id,
            attempt: StartAttempt {
                pid: std::process::id(),
                at_unix_millis: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis() as u64)
                    .unwrap_or(0),
            },
        };
        self.lease
            .get_or_init(|| {
                FileLock::exclusive(&self.lease_path(&pending)).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let _write = FileLock::exclusive(&self.file.with_extension("lock"))
            .map_err(|error| error.to_string())?;
        let mut document = self.read_document();
        let crash = self.crash_record.as_deref().and_then(read_crash_record);
        document.unfinished_starts = counted_starts(&document, crash.as_ref());
        document.last_attempt = None;
        let previous = std::mem::take(&mut document.pending);
        for item in previous {
            if self.is_running(&item) {
                if item.attempt.pid != pending.attempt.pid || item.id != pending.id {
                    document.pending.push(item);
                }
            } else if !ended_in_crash(&item.attempt, crash.as_ref()) {
                document.unfinished_starts = document.unfinished_starts.saturating_add(1);
            }
        }
        document.pending.push(pending);
        self.write(&document)?;
        self.recorded.store(true, Ordering::Release);
        Ok(())
    }

    /// Clears this run's attempt: the start finished, the instance stopped
    /// in order, or the application is leaving in order. Nothing to do when
    /// this run recorded none (the Agent was off), so a crash-loop stop
    /// survives an orderly exit.
    pub fn finish(&self) {
        if self.recorded.swap(false, Ordering::AcqRel) {
            if let Err(error) = self.clear_attempt() {
                log::warn!("the Agent start marker was not cleared: {error}");
            }
        }
    }

    /// Forgets failed starts and this attempt, preserving other live starts.
    pub fn reset(&self) -> Result<(), String> {
        self.recorded.store(false, Ordering::Release);
        self.clear_attempt()
    }

    fn clear_attempt(&self) -> Result<(), String> {
        let _write = FileLock::exclusive(&self.file.with_extension("lock"))
            .map_err(|error| error.to_string())?;
        let mut document = self.read_document();
        document.unfinished_starts = 0;
        document.last_attempt = None;
        document.pending.retain(|item| {
            (item.attempt.pid != std::process::id() || item.id != self.id) && self.is_running(item)
        });
        self.write(&document)
    }

    fn read_document(&self) -> StartMarkerDocument {
        match std::fs::read(&self.file) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!("the Agent start marker is unreadable and counts no starts: {error}");
                StartMarkerDocument::default()
            }),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "the Agent start marker is unreadable and counts no starts: {error}"
                    );
                }
                StartMarkerDocument::default()
            }
        }
    }

    fn lease_path(&self, pending: &PendingStart) -> PathBuf {
        self.file
            .with_extension("owners")
            .join(format!("{}-{}.lock", pending.attempt.pid, pending.id))
    }

    fn is_running(&self, pending: &PendingStart) -> bool {
        !matches!(
            FileLock::try_exclusive(&self.lease_path(pending)),
            Ok(Some(_))
        )
    }

    #[cfg(test)]
    pub(crate) fn file_exists_for_test(&self) -> bool {
        self.file.exists()
    }

    #[cfg(test)]
    pub(crate) fn file_for_test(&self) -> &std::path::Path {
        &self.file
    }

    fn write(&self, document: &StartMarkerDocument) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(document).map_err(|error| error.to_string())?;
        write_atomic(&self.file, &bytes, FileAccess::Shared)
    }
}

fn ended_in_crash(attempt: &StartAttempt, crash: Option<&CrashRecord>) -> bool {
    crash.is_some_and(|crash| {
        crash.pid == attempt.pid && crash.at_unix_millis >= attempt.at_unix_millis
    })
}

#[cfg(test)]
mod tests;
