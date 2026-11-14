use super::*;

fn executable(directory: &std::path::Path) -> ResolvedPath {
    let path = directory.join("custom name");
    std::fs::write(&path, b"installed").unwrap();
    ResolvedPath::resolve(path).unwrap()
}

#[test]
fn a_second_update_of_the_same_executable_is_refused_until_the_first_ends() {
    let directory = tempfile::tempdir().unwrap();
    let executable = executable(directory.path());
    let first = UpdateLock::acquire(&executable).unwrap();
    let error = UpdateLock::acquire(&executable).unwrap_err();
    assert_eq!(
        error,
        UpdateError::LockHeld {
            executable: executable.native_text().to_string(),
            lock: ".custom name.update.lock".to_string(),
        }
    );
    assert!(error.to_string().starts_with("another update of "));
    drop(first);
    let _again = UpdateLock::acquire(&executable).unwrap();
    // The file stays: removing it would race an updater that opened it.
    assert!(directory.path().join(".custom name.update.lock").is_file());
}

#[test]
fn the_lock_names_the_executable() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        lock_name(&executable(directory.path())),
        ".custom name.update.lock"
    );
}

#[cfg(unix)]
#[test]
fn an_unwritable_directory_is_a_write_failure() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let executable = executable(directory.path());
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = UpdateLock::acquire(&executable);
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    // root writes anywhere.
    if let Err(error) = result {
        assert!(
            matches!(
                error,
                UpdateError::Write {
                    what: "the update lock",
                    ..
                }
            ),
            "{error:?}"
        );
    }
}
