//! The crash record: the one fatal panic a process leaves behind.
//!
//! A fatal panic aborts the process, so the in-memory record
//! ([`super::recorded_panics`]) dies with it. Before it aborts, the panic
//! hook writes that panic to a small file the application names at start-up
//! ([`configure_crash_record`]), so the next run can tell how the last one
//! ended: the Agent's crash-loop marker does not count a start whose run
//! ended in a fatal panic, since every fatal panic is on a thread that is
//! not the Agent's (the Agent's threads are contained).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value};

use super::faults::PanicFault;

static CRASH_RECORD: OnceLock<PathBuf> = OnceLock::new();

/// The fatal panic a run ended in, as the next run reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashRecord {
    /// The process that panicked.
    pub pid: u32,
    pub at_unix_millis: u64,
    pub thread: String,
    pub location: String,
    /// Bounded by [`super::MAX_PANIC_MESSAGE_BYTES`].
    pub message: String,
}

/// Names the file a fatal panic is written to before the process aborts.
/// Once per process; a later call is ignored and returns `false`.
pub fn configure_crash_record(path: PathBuf) -> bool {
    CRASH_RECORD.set(path).is_ok()
}

/// The crash record at `path`, if one is there and reads.
pub fn read_crash_record(path: &Path) -> Option<CrashRecord> {
    let document: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let text = |key: &str| document.get(key)?.as_str().map(str::to_string);
    Some(CrashRecord {
        pid: u32::try_from(document.get("pid")?.as_u64()?).ok()?,
        at_unix_millis: document.get("atUnixMillis")?.as_u64()?,
        thread: text("thread")?,
        location: text("location")?,
        message: text("message")?,
    })
}

/// Writes `fault` as the crash record, best effort: the process is about
/// to abort, so a failure is only reported on stderr. The file is replaced
/// whole (a temporary file renamed over it).
pub(super) fn write_crash_record(fault: &PanicFault) {
    let Some(path) = CRASH_RECORD.get() else {
        return;
    };
    let document = json!({
        "pid": std::process::id(),
        "atUnixMillis": fault.at_unix_millis,
        "thread": fault.thread,
        "location": fault.location,
        "message": fault.message,
    });
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let written = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&temporary)?;
        file.write_all(document.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = written {
        let _ = writeln!(
            std::io::stderr(),
            "the crash record {} was not written: {error}",
            path.display()
        );
    }
}
