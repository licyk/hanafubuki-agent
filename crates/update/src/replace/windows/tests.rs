use std::cell::Cell;
use std::fs;

use super::*;

struct Fixture {
    _directory: tempfile::TempDir,
    executable: ResolvedPath,
    staged: std::path::PathBuf,
    backup: Backup,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("custom name.exe");
        fs::write(&path, b"original").unwrap();
        let staged = directory.path().join("staged.exe");
        fs::write(&staged, b"new").unwrap();
        let executable = ResolvedPath::resolve(&path).unwrap();
        let backup = Backup::of(&executable);
        Self {
            _directory: directory,
            executable,
            staged,
            backup,
        }
    }

    fn replace(&self, rename: &dyn Fn(&Path, &Path) -> io::Result<()>) -> Result<(), UpdateError> {
        replace(&self.executable, &self.staged, &self.backup, rename)
    }

    fn installed(&self) -> Vec<u8> {
        fs::read(self.executable.system()).unwrap()
    }
}

fn held() -> io::Error {
    io::Error::from_raw_os_error(32)
}

#[test]
fn the_backup_is_named_after_the_executable_and_the_time() {
    let fixture = Fixture::new();
    let name = fixture
        .backup
        .native
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        crate::cleanup::is_backup_of(&name, "custom name.exe"),
        "{name}"
    );
    assert_eq!(
        fixture.backup.native.parent(),
        fixture.executable.native().parent()
    );
}

#[test]
fn the_running_executable_is_moved_aside_and_the_new_one_put_in() {
    let fixture = Fixture::new();
    fixture.replace(&rename_retrying).unwrap();
    assert_eq!(fixture.installed(), b"new");
    assert_eq!(fs::read(&fixture.backup.system).unwrap(), b"original");
    assert!(!fixture.staged.exists());
}

#[test]
fn when_the_new_one_cannot_be_put_in_the_previous_one_is_put_back() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.staged).unwrap();
    let error = fixture.replace(&rename_retrying).unwrap_err();
    assert!(matches!(error, UpdateError::Restored { .. }), "{error:?}");
    assert!(error
        .to_string()
        .ends_with("the previous one was put back."));
    assert_eq!(fixture.installed(), b"original");
    assert!(!fixture.backup.system.exists());
}

#[test]
fn when_neither_can_be_put_in_both_paths_are_named() {
    let fixture = Fixture::new();
    let calls = Cell::new(0);
    let rename = |from: &Path, to: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            fs::rename(from, to)
        } else {
            Err(io::Error::other("refused"))
        }
    };
    let error = fixture.replace(&rename).unwrap_err();
    let backup = fixture.backup.native.display().to_string();
    assert_eq!(
        error.to_string(),
        format!(
            "cannot put the new executable at {0} (refused), and the previous one could not be \
             put back (refused). Rename {backup} to {0} by hand.",
            fixture.executable.native_text()
        )
    );
    assert_eq!(fs::read(&fixture.backup.system).unwrap(), b"original");
    assert_eq!(fs::read(&fixture.staged).unwrap(), b"new");
}

#[test]
fn an_executable_another_program_holds_is_left_as_it_was() {
    let fixture = Fixture::new();
    let error = fixture
        .replace(&|_: &Path, _: &Path| Err(held()))
        .unwrap_err();
    assert!(
        matches!(error, UpdateError::MoveAside { held: true, .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("another program holds it open"));
    assert_eq!(fixture.installed(), b"original");
    let other = fixture
        .replace(&|_: &Path, _: &Path| Err(io::Error::other("refused")))
        .unwrap_err();
    assert_eq!(
        other.to_string(),
        format!(
            "cannot move {} aside (refused); nothing was changed.",
            fixture.executable.native_text()
        )
    );
}

#[test]
fn a_held_file_is_tried_again_and_another_failure_is_not() {
    let attempts = Cell::new(0);
    let result = retry(10, Duration::ZERO, || {
        attempts.set(attempts.get() + 1);
        if attempts.get() < 4 {
            Err(held())
        } else {
            Ok(())
        }
    });
    result.unwrap();
    assert_eq!(attempts.get(), 4);

    attempts.set(0);
    let result = retry(10, Duration::ZERO, || {
        attempts.set(attempts.get() + 1);
        Err(held())
    });
    assert_eq!(result.unwrap_err().raw_os_error(), Some(32));
    assert_eq!(attempts.get(), 10);

    attempts.set(0);
    let result = retry(10, Duration::ZERO, || {
        attempts.set(attempts.get() + 1);
        Err(io::Error::other("refused"))
    });
    assert!(result.is_err());
    assert_eq!(attempts.get(), 1);
}
