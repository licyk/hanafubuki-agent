//! The standalone Agent's log file, `<data_dir>/logs/hanafubuki-agent.log`.
//!
//! A host keeps its own logger: nothing here runs unless the standalone
//! binary installs it ([`install_file_logger`]). The file is rotated by
//! size: past [`MAX_LOG_FILE_BYTES`] it becomes `hanafubuki-agent.log.1`, the
//! older ones move up and the oldest beyond [`ROTATED_LOG_FILES`] is
//! removed. What a record may contain is the callers' rule (AGENTS.md, Agent
//! Content Logging): content only at `trace`, bounded, never credentials.

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use log::{Level, LevelFilter, Log, Metadata, Record};
use parking_lot::Mutex;

pub const LOG_FILE_NAME: &str = "hanafubuki-agent.log";
/// The size past which the log file is rotated.
pub const MAX_LOG_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The rotated files kept beside the current one.
pub const ROTATED_LOG_FILES: usize = 3;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

thread_local! {
    static WRITING: Cell<bool> = const { Cell::new(false) };
}

/// Makes the file logger in `logs_dir` the process's logger, recording at
/// `level` and above. Once per process.
pub fn install_file_logger(logs_dir: &Path, level: LevelFilter) -> Result<(), String> {
    let file = LogFile::open(
        logs_dir.join(LOG_FILE_NAME),
        MAX_LOG_FILE_BYTES,
        ROTATED_LOG_FILES,
    )?;
    LOGGER
        .set(FileLogger::new(file, level))
        .map_err(|_| "the Agent's file logger is already installed".to_string())?;
    let logger = LOGGER.get().expect("the file logger was just set");
    log::set_logger(logger).map_err(|error| format!("install the Agent's file logger: {error}"))?;
    log::set_max_level(level);
    Ok(())
}

/// Changes the installed file logger's level, as a configuration read after
/// it was installed says.
pub fn set_log_level(level: LevelFilter) {
    if let Some(logger) = LOGGER.get() {
        logger.set_level(level);
        log::set_max_level(level);
    }
}

/// Whether this thread is writing a log record now. The panic hook's
/// `logging_blocked` (`panic::install_panic_hook`): a panic raised while a
/// record is written must not log again from inside the logger.
pub fn logging_blocked() -> bool {
    WRITING.try_with(Cell::get).unwrap_or(true)
}

pub(crate) struct FileLogger {
    level: AtomicUsize,
    file: Mutex<LogFile>,
}

impl FileLogger {
    pub(crate) fn new(file: LogFile, level: LevelFilter) -> Self {
        Self {
            level: AtomicUsize::new(level as usize),
            file: Mutex::new(file),
        }
    }

    pub(crate) fn set_level(&self, level: LevelFilter) {
        self.level.store(level as usize, Ordering::Relaxed);
    }
}

impl Log for FileLogger {
    /// Another crate's records below `info` are left out whatever the level:
    /// the HTTP and TLS stacks trace request details the content rules keep
    /// out of the file.
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() as usize <= self.level.load(Ordering::Relaxed)
            && (metadata.level() <= Level::Info
                || metadata.target().starts_with("hanafubuki_agent"))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let _writing = Writing::enter();
        let line = format!(
            "{} {:<5} {}: {}\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            record.level(),
            record.target(),
            record.args()
        );
        self.file.lock().write(line.as_bytes());
    }

    fn flush(&self) {
        let _writing = Writing::enter();
        self.file.lock().flush();
    }
}

/// Marks this thread as writing a record until dropped, a panic's unwinding
/// included.
struct Writing(bool);

impl Writing {
    fn enter() -> Self {
        Self(
            WRITING
                .try_with(|writing| writing.replace(true))
                .unwrap_or(true),
        )
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        let _ = WRITING.try_with(|writing| writing.set(self.0));
    }
}

/// The log file and its rotation. A write that fails is dropped: the logger
/// has nowhere to report it.
pub(crate) struct LogFile {
    path: PathBuf,
    file: Option<File>,
    written: u64,
    max_bytes: u64,
    rotated: usize,
}

impl LogFile {
    pub(crate) fn open(path: PathBuf, max_bytes: u64, rotated: usize) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        let file = append(&path).map_err(|error| format!("open {}: {error}", path.display()))?;
        let written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            path,
            file: Some(file),
            written,
            max_bytes,
            rotated,
        })
    }

    pub(crate) fn write(&mut self, line: &[u8]) {
        if self.written > 0 && self.written + line.len() as u64 > self.max_bytes {
            self.rotate();
        }
        if let Some(file) = self.file.as_mut() {
            if file.write_all(line).is_ok() {
                self.written += line.len() as u64;
            }
        }
    }

    fn flush(&mut self) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
    }

    fn rotate(&mut self) {
        // Windows renames no file that is open.
        self.file = None;
        let _ = fs::remove_file(self.rotated_path(self.rotated));
        for index in (1..self.rotated).rev() {
            let _ = fs::rename(self.rotated_path(index), self.rotated_path(index + 1));
        }
        if self.rotated > 0 {
            let _ = fs::rename(&self.path, self.rotated_path(1));
        } else {
            let _ = fs::remove_file(&self.path);
        }
        self.file = append(&self.path).ok();
        self.written = 0;
    }

    fn rotated_path(&self, index: usize) -> PathBuf {
        let mut path = self.path.clone().into_os_string();
        path.push(format!(".{index}"));
        PathBuf::from(path)
    }
}

fn append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests;
