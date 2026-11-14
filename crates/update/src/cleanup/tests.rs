use std::path::Path;

use super::*;

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn only_this_executables_numbered_backups_are_backups() {
    let executable = "hanafubuki-agent.exe";
    assert!(is_backup_of(
        "hanafubuki-agent.exe.1763107200000.outdated",
        executable
    ));
    for name in [
        "hanafubuki-agent.exe",
        "hanafubuki-agent.exe.outdated",
        "hanafubuki-agent.exe..outdated",
        "hanafubuki-agent.exe.17631x.outdated",
        "hanafubuki-agent.exe.1763107200000.outdated.txt",
        "other.exe.1763107200000.outdated",
        "hanafubuki-agent.exe.1763107200000",
    ] {
        assert!(!is_backup_of(name, executable), "{name}");
    }
}

#[test]
fn stale_staging_directories_go_and_everything_else_stays() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join(".hanafubuki-agent-update-abc123")).unwrap();
    fs::write(
        root.join(".hanafubuki-agent-update-abc123/hanafubuki-agent-9.9.9.tar.gz"),
        b"partial",
    )
    .unwrap();
    fs::create_dir(root.join(".hanafubuki-agent-update-def456")).unwrap();
    // `writable`'s probe shares the prefix, but is a file.
    fs::write(root.join(".hanafubuki-agent-update-probe-xyz"), b"").unwrap();
    fs::create_dir(root.join("hanafubuki-agent-update-kept")).unwrap();
    fs::write(root.join("hanafubuki-agent"), b"installed").unwrap();
    remove_stale_staging(root);
    assert_eq!(
        names(root),
        [
            ".hanafubuki-agent-update-probe-xyz",
            "hanafubuki-agent",
            "hanafubuki-agent-update-kept"
        ]
    );
}

#[test]
fn only_this_executables_backups_are_removed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "custom name.exe",
        "custom name.exe.1.outdated",
        "custom name.exe.1763107200000.outdated",
        "other.exe.1763107200000.outdated",
        "custom name.exe.outdated",
    ] {
        fs::write(root.join(name), b"x").unwrap();
    }
    remove_outdated(root, OsStr::new("custom name.exe"));
    assert_eq!(
        names(root),
        [
            "custom name.exe",
            "custom name.exe.outdated",
            "other.exe.1763107200000.outdated"
        ]
    );
}

#[test]
fn the_sweep_is_windows_only() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("custom name.exe"), b"installed").unwrap();
    fs::write(root.join("custom name.exe.1763107200000.outdated"), b"old").unwrap();
    let installation = Installation::at(&root.join("custom name.exe")).unwrap();
    sweep_outdated(&installation);
    assert_eq!(
        root.join("custom name.exe.1763107200000.outdated").exists(),
        !cfg!(windows)
    );
}
