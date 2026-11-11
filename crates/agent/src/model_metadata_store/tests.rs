use std::time::Instant;

use serde_json::{json, Value};

use super::*;

fn snapshot(window: u64) -> Value {
    json!({
        "routes": [{
            "route": {
                "endpoint": {"protocol": "responses", "baseUrlSha256": "00ff"},
                "model": "model"
            },
            "reported": {
                "contextWindow": {"value": window, "source": "reported", "observedAtMs": 1}
            }
        }],
        "listings": [{
            "endpoint": {"protocol": "responses", "baseUrlSha256": "00ff"},
            "listedAtMs": 1,
            "models": ["model"],
            "held": false
        }]
    })
}

fn decoded(value: Value) -> ModelMetadataSnapshot {
    serde_json::from_value(value).unwrap()
}

/// The store at `model-metadata.json` in `directory`, with the worker group
/// its writer runs in.
fn open_store(directory: &Path) -> (FileModelMetadataStore, BackgroundWorkerGroup) {
    let workers = BackgroundWorkerGroup::contained();
    let store =
        FileModelMetadataStore::open(directory.join("model-metadata.json"), &workers).unwrap();
    (store, workers)
}

fn stop(workers: BackgroundWorkerGroup) {
    let diagnostics = workers.shutdown(Instant::now() + Duration::from_secs(5));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

fn quarantined_files(directory: &Path) -> usize {
    std::fs::read_dir(directory.join(".recovery"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn nothing_saved_is_nothing_known_and_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let (store, workers) = open_store(directory.path());

    assert_eq!(store.load(), ModelMetadataSnapshot::default());
    stop(workers);
    assert!(!directory.path().join("model-metadata.json").exists());
}

#[test]
fn the_newest_snapshot_saved_is_written_by_the_time_the_writer_stops() {
    let directory = tempfile::tempdir().unwrap();
    let (store, workers) = open_store(directory.path());
    for window in [1_000, 2_000, 3_000] {
        store.save(decoded(snapshot(window)));
    }
    stop(workers);

    let file: Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("model-metadata.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(file["version"], 1);
    let (reopened, workers) = open_store(directory.path());
    assert_eq!(reopened.load(), decoded(snapshot(3_000)));
    stop(workers);
}

#[test]
fn unknown_fields_are_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = snapshot(64_000);
    document["version"] = json!(1);
    document["written_by"] = json!("a later Agent");
    document["routes"][0]["note"] = json!("kept for later");
    document["routes"][0]["reported"]["contextWindow"]["confidence"] = json!(0.9);
    std::fs::write(
        directory.path().join("model-metadata.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    let (store, workers) = open_store(directory.path());

    assert_eq!(store.load(), decoded(snapshot(64_000)));
    assert_eq!(quarantined_files(directory.path()), 0);
    stop(workers);
}

#[test]
fn a_file_that_does_not_read_is_quarantined_and_nothing_is_known() {
    for contents in [
        b"{ not json".to_vec(),
        b"[1, 2]".to_vec(),
        serde_json::to_vec(&json!({"version": 2, "routes": []})).unwrap(),
        serde_json::to_vec(&json!({"routes": []})).unwrap(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("model-metadata.json");
        std::fs::write(&file, &contents).unwrap();
        let (store, workers) = open_store(directory.path());

        assert_eq!(store.load(), ModelMetadataSnapshot::default());
        assert!(!file.exists());
        assert_eq!(quarantined_files(directory.path()), 1);
        stop(workers);
    }
}
