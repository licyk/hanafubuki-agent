use super::*;

#[test]
fn syncing_an_existing_directory_is_supported_on_every_platform() {
    let directory = tempfile::tempdir().unwrap();
    sync_directory(directory.path()).unwrap();
}

#[test]
fn publishing_a_synced_file_is_atomic_on_every_platform() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.tmp");
    let destination = directory.path().join("destination.json");
    std::fs::write(&destination, b"stale").unwrap();
    let mut file = std::fs::File::create(&source).unwrap();
    std::io::Write::write_all(&mut file, b"durable").unwrap();
    file.sync_all().unwrap();

    publish_file(&source, &destination).unwrap();

    assert!(!source.exists());
    assert_eq!(std::fs::read(destination).unwrap(), b"durable");
}
