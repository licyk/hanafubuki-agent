//! Helpers for tests that spawn processes.

/// Writes `contents` to `path` and makes it executable, so a test can spawn
/// it while other tests in the same process spawn their own children.
///
/// Writing the file here would race them: a child forked while this
/// process holds the file open for writing inherits that descriptor until
/// it execs, and spawning the file meanwhile fails with `ETXTBSY` ("Text
/// file busy"). Renaming a finished temporary file into place does not help,
/// because the inherited descriptor names the same inode. So a `/bin/sh`
/// child writes the file (`printf` is a builtin, so it forks nothing while
/// the file is open) and has exited before this returns: no process holds
/// the file open for writing when the test spawns it.
#[cfg(unix)]
pub fn write_executable_script(path: &std::path::Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    let status = std::process::Command::new("/bin/sh")
        .args(["-c", "printf '%s' \"$1\" > \"$2\"", "sh", contents])
        .arg(path)
        .status()
        .expect("spawn /bin/sh to write a test script");
    assert!(
        status.success(),
        "writing the test script {} failed: {status}",
        path.display()
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("mark the test script executable");
}
