use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::*;

const HOUR_MS: u64 = 60 * 60 * 1000;
const NOW: u64 = 1_763_107_200_000;

fn version(text: &str) -> Version {
    text.parse().unwrap()
}

fn write(dir: &Path, contents: &str) {
    fs::write(dir.join(UPDATE_CHECK_FILE), contents).unwrap();
}

fn written(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join(UPDATE_CHECK_FILE)).unwrap()).unwrap()
}

/// The files moved into `.recovery/`.
fn recovered(dir: &Path) -> Vec<String> {
    match fs::read_dir(dir.join(".recovery")) {
        Ok(entries) => entries
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn record(dir: &Path, latest: &str, at: u64) {
    UpdateCache::update(dir, |cache| {
        cache.latest_version = Some(version(latest));
        cache.checked_at_ms = Some(at);
    })
    .unwrap();
}

#[test]
fn no_file_is_an_empty_cache_and_the_check_is_due() {
    let dir = tempfile::tempdir().unwrap();
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.latest_version, None);
    assert_eq!(cache.checked_at_ms, None);
    assert_eq!(cache.dismissed_version, None);
    assert!(cache.is_due(NOW));
}

#[test]
fn each_field_reads_alone() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), r#"{"latestVersion": "0.3.0"}"#);
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.latest_version, Some(version("0.3.0")));
    assert!(cache.is_due(NOW), "never answered");

    write(dir.path(), &format!(r#"{{"checkedAtMs": {NOW}}}"#));
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.checked_at_ms, Some(NOW));
    assert_eq!(cache.latest_version, None);

    write(dir.path(), r#"{"dismissedVersion": "0.4.0"}"#);
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.dismissed_version, Some(version("0.4.0")));
    assert!(!cache.lossy);
}

#[test]
fn the_check_is_due_after_twenty_hours_or_when_dated_in_the_future() {
    let cache = |at| UpdateCache {
        checked_at_ms: Some(at),
        ..UpdateCache::default()
    };
    assert!(!cache(NOW).is_due(NOW));
    assert!(!cache(NOW - 20 * HOUR_MS + 1).is_due(NOW));
    assert!(cache(NOW - 20 * HOUR_MS).is_due(NOW));
    assert!(cache(NOW - 48 * HOUR_MS).is_due(NOW));
    // A clock set back.
    assert!(cache(NOW + 1).is_due(NOW));
}

#[test]
fn a_field_that_does_not_read_is_absent_and_the_file_moved_aside_before_a_write() {
    let dir = tempfile::tempdir().unwrap();
    let original = r#"{"latestVersion": "0.3.0-rc.1", "checkedAtMs": "yesterday", "dismissedVersion": 3, "later": {"kept": true}}"#;
    write(dir.path(), original);
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.latest_version, None, "a pre-release is not a release");
    assert_eq!(cache.checked_at_ms, None);
    assert_eq!(cache.dismissed_version, None);
    assert!(cache.is_due(NOW));
    // Reading moves nothing.
    assert!(recovered(dir.path()).is_empty());

    record(dir.path(), "0.3.0", NOW);
    assert_eq!(recovered(dir.path()), [original]);
    assert_eq!(
        written(dir.path()),
        json!({"latestVersion": "0.3.0", "checkedAtMs": NOW, "later": {"kept": true}})
    );
}

#[test]
fn unknown_keys_are_kept_and_a_newer_versions_file_reads_the_same() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &format!(
            r#"{{"latestVersion": "9.1.0", "checkedAtMs": {NOW}, "dismissedVersion": "9.0.0", "channel": "stable", "seen": [1, 2]}}"#
        ),
    );
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.latest_version, Some(version("9.1.0")));
    assert_eq!(cache.dismissed_version, Some(version("9.0.0")));
    assert!(!cache.is_due(NOW + HOUR_MS));

    record(dir.path(), "9.2.0", NOW + 21 * HOUR_MS);
    assert_eq!(
        written(dir.path()),
        json!({
            "latestVersion": "9.2.0",
            "checkedAtMs": NOW + 21 * HOUR_MS,
            "dismissedVersion": "9.0.0",
            "channel": "stable",
            "seen": [1, 2],
        })
    );
    assert!(recovered(dir.path()).is_empty());
}

#[test]
fn a_file_that_is_not_a_json_object_is_moved_aside_at_once() {
    for contents in ["[1, 2]", "\"0.3.0\"", "not json {", ""] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), contents);
        let cache = UpdateCache::load(dir.path());
        assert_eq!(cache.latest_version, None, "{contents}");
        assert!(cache.is_due(NOW), "{contents}");
        assert!(!dir.path().join(UPDATE_CHECK_FILE).exists(), "{contents}");
        assert_eq!(recovered(dir.path()), [contents]);
        record(dir.path(), "0.3.0", NOW);
        assert_eq!(written(dir.path())["latestVersion"], "0.3.0");
    }
}

#[test]
fn a_file_too_large_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let large = format!(r#"{{"pad": "{}"}}"#, "x".repeat(MAX_CACHE_BYTES as usize));
    write(dir.path(), &large);
    let cache = UpdateCache::load(dir.path());
    assert!(cache.is_due(NOW));
    assert_eq!(recovered(dir.path()).len(), 1);
}

/// A directory in the file's place stands for a file that cannot be read.
#[test]
fn a_file_that_cannot_be_read_is_never_written_and_the_check_is_not_due() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(UPDATE_CHECK_FILE);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("inside"), "mine").unwrap();
    let cache = UpdateCache::load(dir.path());
    assert!(!cache.is_due(NOW));
    let error = UpdateCache::update(dir.path(), |cache| {
        cache.latest_version = Some(version("0.3.0"));
    })
    .unwrap_err();
    assert!(error.contains("not read"), "{error}");
    assert_eq!(fs::read_to_string(path.join("inside")).unwrap(), "mine");
    assert!(recovered(dir.path()).is_empty());
}

#[test]
fn a_dismissal_written_meanwhile_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    record(dir.path(), "0.3.0", NOW - 30 * HOUR_MS);
    // This instance read the cache at launch...
    let at_launch = UpdateCache::load(dir.path());
    assert!(at_launch.is_due(NOW));
    // ...another instance's prompt was told to skip 0.3.0...
    UpdateCache::update(dir.path(), |cache| {
        cache.dismissed_version = Some(version("0.3.0"));
    })
    .unwrap();
    // ...and this instance's check then answers.
    record(dir.path(), "0.3.1", NOW);
    assert_eq!(
        written(dir.path()),
        json!({"latestVersion": "0.3.1", "checkedAtMs": NOW, "dismissedVersion": "0.3.0"})
    );
}

#[test]
fn a_write_is_atomic_and_private() {
    let dir = tempfile::tempdir().unwrap();
    record(dir.path(), "0.3.0", NOW);
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [UPDATE_CHECK_FILE]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join(UPDATE_CHECK_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
