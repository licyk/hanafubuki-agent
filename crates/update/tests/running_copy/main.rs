//! A copy of this executable replaced while it runs, on every system. Its
//! `main` answers `--version` as `hanafubuki-agent <version>`, as the CLI
//! does, and updates itself through the library when asked; without a
//! harness, `main` otherwise runs the scenarios in turn: the copy, in a
//! directory with a space and non-ASCII characters, is archived the way
//! `release.yml` archives a release (`tar czf`, `7z a -tzip`) and served by
//! the fake release, then it reinstalls the same version over itself
//! (`force`) while it and another copy run. Not the CLI's binary: a debug
//! one is over a gigabyte.

#[cfg(windows)]
mod windows_cargo;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use hanafubuki_agent_update::test_support::{client_of, FakeArchive, FakeAsset, FakeRelease};
use hanafubuki_agent_update::{
    asset_name, current_version, entry_path, install_blocking, this_target, Installation, Version,
};

/// Updates the running copy through the library.
const UPDATE_SELF: &str = "--update-self";
/// Waits for its standard input to end, then exits.
const WAIT: &str = "--wait";
/// The fake release's latest-release URL, for [`UPDATE_SELF`].
const API_VARIABLE: &str = "RUNNING_COPY_API";
/// `--version` prints this version instead of this build's.
const REPORTS_VARIABLE: &str = "RUNNING_COPY_REPORTS";
/// `--version` answers nothing for a minute.
const HANG_VARIABLE: &str = "RUNNING_COPY_HANG";

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => answer_version(),
        Some(UPDATE_SELF) => update_self(),
        Some(WAIT) => wait(),
        _ => scenarios(),
    }
}

fn answer_version() {
    if std::env::var_os(HANG_VARIABLE).is_some() {
        std::thread::sleep(Duration::from_secs(60));
    }
    let version = std::env::var(REPORTS_VARIABLE).unwrap_or_else(|_| current_version().to_string());
    println!("hanafubuki-agent {version}");
}

fn update_self() {
    let api = std::env::var(API_VARIABLE).expect("the fake release's URL");
    let client = client_of(&api, None, Duration::from_secs(30));
    let installation = Installation::current().expect("the running copy");
    let release = client.latest_blocking().expect("the fake release");
    match install_blocking(&client, &installation, &release, true, &mut |_| {}) {
        Ok(installed) => {
            println!(
                "installed {} backup {:?}",
                installed.version, installed.backup
            );
            // This process goes on running the file it started from.
            println!("still running");
        }
        Err(error) => {
            println!("failed: {}: {error}", error.kind());
            std::process::exit(3);
        }
    }
}

fn wait() {
    let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
    println!("done");
}

fn scenarios() {
    let Some(target) = this_target() else {
        eprintln!("running copy: skipped, this build has no release target");
        return;
    };
    let version = current_version();
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("工具 bin");
    std::fs::create_dir(&directory).unwrap();
    let running = directory.join(format!("custom name{}", std::env::consts::EXE_SUFFIX));
    let this = std::env::current_exe().unwrap();
    std::fs::copy(&this, &running).unwrap();
    let bytes = std::fs::read(&this).unwrap();
    eprintln!("running copy: {} bytes", bytes.len());

    let archive = release_archive(root.path(), &version, target, &bytes);
    let server = FakeRelease::start();
    server.publish(
        &format!("v{version}"),
        &[FakeAsset::new(&asset_name(&version, target), archive)],
    );
    let copy = Copy {
        running,
        directory,
        api: server.api_url(),
        bytes,
    };

    let first_backup = copy.is_replaced_while_it_and_another_copy_run();
    copy.failures_leave_it_as_it_was();
    copy.an_interrupted_update_is_recovered_by_the_next(first_backup.as_deref());
    copy.a_backup_no_process_runs_is_swept();
    #[cfg(windows)]
    windows_cargo::cargo_install_over_a_running_copy(root.path());
    eprintln!("running copy: every scenario passed");
}

struct Copy {
    running: PathBuf,
    directory: PathBuf,
    api: String,
    bytes: Vec<u8>,
}

impl Copy {
    /// The copy updates itself while another copy waits; both go on, the
    /// file is a new one, and the next start answers.
    fn is_replaced_while_it_and_another_copy_run(&self) -> Option<PathBuf> {
        let before = identity(&self.running);
        let mut waiting = Command::new(&self.running)
            .arg(WAIT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = self.update(&[]);
        assert_success(&output);
        assert!(
            stdout(&output).contains("still running"),
            "{}",
            stdout(&output)
        );
        assert_ne!(identity(&self.running), before, "the file was replaced");
        assert_eq!(std::fs::read(&self.running).unwrap(), self.bytes);
        let backups = self.backups();
        if cfg!(windows) {
            assert_eq!(backups.len(), 1, "{backups:?}");
            assert_eq!(std::fs::read(&backups[0]).unwrap(), self.bytes);
        } else {
            assert!(backups.is_empty(), "{backups:?}");
        }
        assert_eq!(finish(&mut waiting), "done", "the other copy went on");
        let next = Command::new(&self.running)
            .arg("--version")
            .output()
            .unwrap();
        assert_success(&next);
        assert_eq!(
            stdout(&next).trim(),
            format!("hanafubuki-agent {}", current_version())
        );
        eprintln!(
            "running copy: replaced while it and another copy ran ({before:?} -> {:?}), \
             backups {backups:?}",
            identity(&self.running)
        );
        backups.into_iter().next()
    }

    /// A held lock, a new executable that reports another version and one
    /// that never answers: each leaves the file as it was.
    fn failures_leave_it_as_it_was(&self) {
        let lock_path = self.directory.join(format!(
            ".{}.update.lock",
            self.running.file_name().unwrap().to_string_lossy()
        ));
        let lock = std::fs::File::create(&lock_path).unwrap();
        lock.try_lock().unwrap();
        self.fails_and_leaves_it(&[], "lock held");
        lock.unlock().unwrap();
        drop(lock);
        self.fails_and_leaves_it(&[(REPORTS_VARIABLE, "0.0.9")], "wrong version");
        self.fails_and_leaves_it(&[(HANG_VARIABLE, "1")], "version timeout");
    }

    fn fails_and_leaves_it(&self, environment: &[(&str, &str)], kind: &str) {
        let before = identity(&self.running);
        let backups = self.backups();
        let output = self.update(environment);
        assert_eq!(output.status.code(), Some(3), "{}", describe(&output));
        assert!(
            stdout(&output).starts_with(&format!("failed: {kind}: ")),
            "{}",
            describe(&output)
        );
        assert_eq!(identity(&self.running), before);
        assert_eq!(std::fs::read(&self.running).unwrap(), self.bytes);
        assert_eq!(self.backups(), backups);
        // Windows may hold the image of the new executable a moment after
        // its process ends, and then the staging directory stays until the
        // next update removes it (as the next scenario checks).
        let staging = self.staging();
        assert!(staging.is_empty() || cfg!(windows), "{staging:?}");
        eprintln!("running copy: {kind}: left as it was (staging left: {staging:?})");
    }

    /// A staging directory a killed update left is removed by the next
    /// update, and so is (on Windows) a backup no process runs any more.
    fn an_interrupted_update_is_recovered_by_the_next(&self, first_backup: Option<&Path>) {
        let stale = self.directory.join(".hanafubuki-agent-update-interrupted");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("partial.tar.gz"), b"partial").unwrap();
        assert_success(&self.update(&[]));
        assert!(!stale.exists(), "the staging directory was removed");
        if let Some(first_backup) = first_backup {
            assert!(!first_backup.exists(), "{first_backup:?} was removed");
        }
        assert_eq!(self.backups().len(), usize::from(cfg!(windows)));
        eprintln!("running copy: an interrupted update was recovered");
    }

    /// Windows: once its process ended, the last backup goes at the sweep.
    fn a_backup_no_process_runs_is_swept(&self) {
        let installation = Installation::at(&self.running).unwrap();
        // Windows may release the image of an ended process a moment late.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            hanafubuki_agent_update::sweep_outdated(&installation);
            if self.backups().is_empty() || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(self.backups().is_empty(), "{:?}", self.backups());
        eprintln!("running copy: the sweep left no backup");
    }

    fn update(&self, environment: &[(&str, &str)]) -> Output {
        let mut command = Command::new(&self.running);
        command.arg(UPDATE_SELF).env(API_VARIABLE, &self.api);
        for (name, value) in environment {
            command.env(name, value);
        }
        command.output().unwrap()
    }

    fn backups(&self) -> Vec<PathBuf> {
        self.entries(|name| name.ends_with(".outdated"))
    }

    fn staging(&self) -> Vec<PathBuf> {
        self.entries(|name| name.starts_with(".hanafubuki-agent-update-"))
    }

    fn entries(&self, matching: impl Fn(&str) -> bool) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| matching(&path.file_name().unwrap().to_string_lossy()))
            .collect();
        entries.sort();
        entries
    }
}

/// `version`'s archive for `target` holding `executable`, made as
/// `release.yml` makes it when its archiver is here, else by the test
/// support's writer.
fn release_archive(root: &Path, version: &Version, target: &str, executable: &[u8]) -> Vec<u8> {
    let entry = entry_path(version, target);
    let (name, binary) = entry.split_once('/').unwrap();
    let dist = root.join("dist");
    std::fs::create_dir_all(dist.join(name)).unwrap();
    std::fs::write(dist.join(name).join(binary), executable).unwrap();
    for file in ["LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.txt"] {
        std::fs::write(dist.join(name).join(file), file).unwrap();
    }
    let archive = dist.join(asset_name(version, target));
    let mut command = if cfg!(windows) {
        let mut command = Command::new("7z");
        command.args(["a", "-tzip"]).arg(&archive).arg(name);
        command
    } else {
        let mut command = Command::new("tar");
        command
            .env("COPYFILE_DISABLE", "1")
            .arg("czf")
            .arg(&archive)
            .arg(name);
        command
    };
    let made = command
        .current_dir(&dist)
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if made {
        eprintln!("running copy: archived with {:?}", command.get_program());
        std::fs::read(archive).unwrap()
    } else {
        eprintln!(
            "running copy: no {:?} here; archived with the test writer",
            command.get_program()
        );
        FakeArchive::release(version, executable).bytes()
    }
}

/// What tells a replaced file from the one it replaced: its inode on Unix,
/// its last write on Windows (whose creation time a file renamed in under
/// the same name inherits).
fn identity(path: &Path) -> String {
    let metadata = std::fs::metadata(path).unwrap();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("inode {}", metadata.ino())
    };
    #[cfg(windows)]
    let identity = format!("written {:?}", metadata.modified().unwrap());
    identity
}

fn finish(child: &mut Child) -> String {
    drop(child.stdin.take());
    let mut output = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut output).unwrap();
    assert!(child.wait().unwrap().success());
    output.trim().to_string()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn describe(output: &Output) -> String {
    format!(
        "{}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", describe(output));
}
