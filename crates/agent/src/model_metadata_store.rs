//! `model-metadata.json`: what the Agent knows about each model it reaches
//! (the kernel's `ModelMetadataSnapshot`), kept between runs. It is the
//! Agent's own state, read tolerantly: an unknown field is ignored and an
//! entry or fact that does not read is left out (the snapshot's own
//! decoding), while a file that does not read at all (over its size bound,
//! not a JSON object, or of another version) is quarantined and the run
//! starts with nothing known. A change is handed to a writer in the Agent's
//! worker group, which writes the newest snapshot through a temporary file
//! renamed over the old one, and the last one before it stops.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use hanafubuki_agent_config::{write_atomic, FileAccess};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::persistence::quarantine_file;
use hanafubuki_agent_kernel::{ModelMetadataSnapshot, ModelMetadataStore};
use serde::Serialize;
use serde_json::Value;

const VERSION: u64 = 1;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// How long the writer waits for a snapshot before it looks whether the
/// Agent is stopping.
const WRITER_POLL: Duration = Duration::from_millis(100);

pub(crate) struct FileModelMetadataStore {
    path: PathBuf,
    writer: Sender<ModelMetadataSnapshot>,
}

impl FileModelMetadataStore {
    /// The store at `path`, its writer started in `workers`.
    pub(crate) fn open(path: PathBuf, workers: &BackgroundWorkerGroup) -> Result<Self, String> {
        let (writer, snapshots) = mpsc::channel();
        let file = path.clone();
        workers.spawn("agent-model-metadata-writer", move |cancellation| {
            write_snapshots(&file, &snapshots, &cancellation)
        })?;
        Ok(Self { path, writer })
    }
}

impl ModelMetadataStore for FileModelMetadataStore {
    fn load(&self) -> ModelMetadataSnapshot {
        read(&self.path)
    }

    fn save(&self, snapshot: ModelMetadataSnapshot) {
        if self.writer.send(snapshot).is_err() {
            log::warn!("the model metadata was not saved: its writer has stopped");
        }
    }
}

#[derive(Serialize)]
struct Document<'a> {
    version: u64,
    #[serde(flatten)]
    snapshot: &'a ModelMetadataSnapshot,
}

/// Writes the newest snapshot waiting, until the group stops the writer
/// and nothing waits.
fn write_snapshots(
    path: &Path,
    snapshots: &Receiver<ModelMetadataSnapshot>,
    cancellation: &CancellationToken,
) {
    loop {
        let mut snapshot = match snapshots.recv_timeout(WRITER_POLL) {
            Ok(snapshot) => snapshot,
            Err(RecvTimeoutError::Timeout) if !cancellation.is_cancelled() => continue,
            Err(_) => return,
        };
        while let Ok(newer) = snapshots.try_recv() {
            snapshot = newer;
        }
        if let Err(error) = write(path, &snapshot) {
            log::warn!("the model metadata was not saved: {error}");
        }
    }
}

fn write(path: &Path, snapshot: &ModelMetadataSnapshot) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&Document {
        version: VERSION,
        snapshot,
    })
    .map_err(|error| error.to_string())?;
    write_atomic(path, &bytes, FileAccess::Shared)
}

fn read(path: &Path) -> ModelMetadataSnapshot {
    let bytes = match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_FILE_BYTES => {
            return quarantined(path, "it is over 16 MiB");
        }
        Ok(_) => std::fs::read(path),
        Err(error) => Err(error),
    };
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ModelMetadataSnapshot::default();
        }
        Err(error) => {
            log::warn!(
                "the model metadata {} cannot be read, so nothing is known: {error}",
                path.display()
            );
            return ModelMetadataSnapshot::default();
        }
    };
    let document = match serde_json::from_slice::<Value>(&bytes) {
        Ok(document @ Value::Object(_)) => document,
        _ => return quarantined(path, "it is not a JSON object"),
    };
    if document.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return quarantined(path, "it is not of version 1");
    }
    serde_json::from_value(document)
        .unwrap_or_else(|_| quarantined(path, "its routes or listings do not read"))
}

fn quarantined(path: &Path, reason: &str) -> ModelMetadataSnapshot {
    log::warn!(
        "the model metadata {} is quarantined and nothing is known: {reason}",
        path.display()
    );
    if let Err(error) = quarantine_file(path) {
        log::warn!(
            "the model metadata {} was not quarantined: {error}",
            path.display()
        );
    }
    ModelMetadataSnapshot::default()
}

#[cfg(test)]
mod tests;
