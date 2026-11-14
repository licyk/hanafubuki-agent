//! The update of a released binary against the fake release on loopback:
//! what is installed, what is refused, and that every failure leaves the
//! installed executable byte for byte and no staging directory. On Unix
//! the executables are shell scripts that print a version; the steps that
//! run one are Unix-only here (`tests/running_copy` runs real executables
//! on every system).

use std::path::{Path, PathBuf};

use hanafubuki_agent_update::test_support::{FakeArchive, FakeAsset, FakeRelease, FakeResponse};
use hanafubuki_agent_update::{
    current_version, install_blocking, Installation, Installed, Progress, Release, ReleaseClient,
    UpdateError, Version,
};
use serde_json::Value;

/// A released binary in a directory of its own, and a fake release.
struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    executable: PathBuf,
    original: Vec<u8>,
    server: FakeRelease,
}

impl Fixture {
    /// An installed executable that reports `version` on Unix.
    fn reporting(version: &Version) -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("bin dir");
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join(executable_name());
        let original = reporting(version);
        std::fs::write(&executable, &original).unwrap();
        make_executable(&executable);
        Self {
            _root: root,
            directory,
            executable,
            original,
            server: FakeRelease::start(),
        }
    }

    /// Publishes `version` with an archive whose executable is `executable`.
    fn publish(&self, version: &Version, executable: impl Into<Vec<u8>>) -> FakeAsset {
        let asset = FakeAsset::new(
            &FakeArchive::name(version),
            FakeArchive::release(version, executable).bytes(),
        );
        self.server
            .publish(&format!("v{version}"), std::slice::from_ref(&asset));
        asset
    }

    /// Publishes `version` as `asset`, the release's answer altered by `edit`.
    fn publish_edited(&self, version: &Version, asset: FakeAsset, edit: impl FnOnce(&mut Value)) {
        let mut answer = self
            .server
            .release_json(&format!("v{version}"), std::slice::from_ref(&asset));
        edit(&mut answer);
        self.server.answer_latest(FakeResponse::json(&answer));
        self.server.route(
            &format!("/download/{}", asset.name),
            FakeResponse::ok(asset.body),
        );
    }

    fn client(&self) -> ReleaseClient {
        self.server.client(Some("ghp_secret".to_string()))
    }

    fn installation(&self) -> Installation {
        Installation::at(&self.executable).unwrap()
    }

    fn release(&self) -> Release {
        self.client().latest_blocking().unwrap()
    }

    fn install(&self, force: bool) -> (Result<Installed, UpdateError>, Vec<Progress>) {
        let mut seen = Vec::new();
        let result = install_blocking(
            &self.client(),
            &self.installation(),
            &self.release(),
            force,
            &mut |progress| seen.push(progress),
        );
        (result, seen)
    }

    fn asset_requests(&self) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|request| request.path.starts_with("/download/"))
            .count()
    }

    /// The installed executable as it was, and nothing left beside it but
    /// the update's lock.
    fn left_as_it_was(&self) {
        assert_eq!(std::fs::read(&self.executable).unwrap(), self.original);
        for name in names(&self.directory) {
            assert!(
                name == executable_name() || name.ends_with(".update.lock"),
                "{name} left beside the executable"
            );
        }
    }
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "custom name.exe"
    } else {
        "custom name"
    }
}

/// A shell script that prints `hanafubuki-agent <version>`.
fn reporting(version: &Version) -> Vec<u8> {
    format!("#!/bin/sh\necho 'hanafubuki-agent {version}'\n").into_bytes()
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn newer() -> Version {
    Version::new(current_version().major + 1, 0, 0)
}

#[test]
fn the_same_version_is_left_alone_without_force() {
    let current = current_version();
    let fixture = Fixture::reporting(&current);
    fixture.publish(&current, reporting(&current));
    let (installed, seen) = fixture.install(false);
    assert_eq!(
        installed.unwrap(),
        Installed {
            version: current,
            backup: None,
            already: true
        }
    );
    assert!(seen.is_empty());
    assert_eq!(fixture.asset_requests(), 0);
    fixture.left_as_it_was();
    assert_eq!(names(&fixture.directory), [executable_name()]);
}

#[test]
fn an_older_release_is_refused_even_with_force() {
    let older = Version::new(0, 0, 1);
    let fixture = Fixture::reporting(&current_version());
    fixture.publish(&older, reporting(&older));
    for force in [false, true] {
        let error = fixture.install(force).0.unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "refusing to replace {} with the older release 0.0.1; --force only reinstalls \
                 the same version.",
                current_version()
            )
        );
    }
    assert_eq!(fixture.asset_requests(), 0);
    fixture.left_as_it_was();
}

#[test]
fn metadata_that_cannot_be_installed_fails_before_any_download() {
    let version = newer();
    let fixture = Fixture::reporting(&current_version());
    let archive = FakeArchive::name(&version);

    let other = FakeAsset::new("hanafubuki-agent-other-target.tar.gz", "unused");
    fixture.server.publish(&format!("v{version}"), &[other]);
    let error = fixture.install(false).0.unwrap_err();
    assert!(
        matches!(&error, UpdateError::NoArchive { archive: name, .. } if *name == archive),
        "{error:?}"
    );
    assert!(error.to_string().contains("try again in a few minutes"));

    let without = FakeAsset::new(&archive, "unused").without_digest();
    fixture.server.publish(&format!("v{version}"), &[without]);
    assert_eq!(
        fixture.install(false).0.unwrap_err().to_string(),
        format!("release {version} gives no SHA-256 digest for {archive}; nothing was changed.")
    );

    let malformed = FakeAsset::new(&archive, "unused").with_digest("sha256:broken");
    fixture.server.publish(&format!("v{version}"), &[malformed]);
    assert!(matches!(
        fixture.install(false).0.unwrap_err(),
        UpdateError::NoDigest { .. }
    ));

    // An asset that is not https is left out by the lookup: no archive.
    let asset = FakeAsset::new(&archive, "unused");
    fixture.publish_edited(&version, asset, |answer| {
        answer["assets"][0]["browser_download_url"] = "http://192.0.2.1/archive".into();
    });
    assert!(matches!(
        fixture.install(false).0.unwrap_err(),
        UpdateError::NoArchive { .. }
    ));

    assert_eq!(fixture.asset_requests(), 0);
    fixture.left_as_it_was();
    assert_eq!(names(&fixture.directory), [executable_name()]);
}

#[test]
fn a_held_lock_stops_the_update_before_the_download() {
    let version = newer();
    let fixture = Fixture::reporting(&current_version());
    fixture.publish(&version, reporting(&version));
    let lock_path = fixture
        .directory
        .join(format!(".{}.update.lock", executable_name()));
    let lock = std::fs::File::create(&lock_path).unwrap();
    lock.try_lock().unwrap();
    let error = fixture.install(false).0.unwrap_err();
    assert_eq!(
        error,
        UpdateError::LockHeld {
            executable: fixture
                .installation()
                .executable()
                .native_text()
                .to_string(),
            lock: format!(".{}.update.lock", executable_name()),
        }
    );
    assert!(error.to_string().ends_with("try again when it ends."));
    assert_eq!(fixture.asset_requests(), 0);
    fixture.left_as_it_was();
}

#[test]
fn a_stale_staging_directory_is_removed_under_the_lock() {
    let current = current_version();
    let fixture = Fixture::reporting(&current);
    fixture.publish(&current, reporting(&current));
    let stale = fixture
        .directory
        .join(".hanafubuki-agent-update-interrupted");
    std::fs::create_dir(&stale).unwrap();
    std::fs::write(stale.join("partial.tar.gz"), b"partial").unwrap();
    // The update itself fails on Windows (a script is no executable) or
    // succeeds on Unix; the sweep comes first either way.
    let _ = fixture.install(true);
    assert!(!stale.exists());
}

#[test]
fn a_corrupt_short_or_long_download_leaves_the_executable() {
    let version = newer();
    let fixture = Fixture::reporting(&current_version());
    let archive = FakeArchive::name(&version);
    let bytes = FakeArchive::release(&version, reporting(&version)).bytes();
    let size = bytes.len() as u64;
    let cases: Vec<(FakeAsset, &str)> = vec![
        (
            FakeAsset::new(&archive, bytes.clone())
                .with_digest(&format!("sha256:{}", "0".repeat(64))),
            "does not match its SHA-256 digest",
        ),
        (
            FakeAsset::new(&archive, bytes.clone()).listed_size(size + 10),
            "ended after",
        ),
        (
            FakeAsset::new(&archive, bytes.clone()).listed_size(size - 10),
            "is longer than the release says",
        ),
    ];
    for (asset, expected) in cases {
        fixture.server.publish(&format!("v{version}"), &[asset]);
        let error = fixture.install(false).0.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        fixture.left_as_it_was();
    }
    let asset = FakeAsset::new(&archive, bytes.clone());
    fixture.server.publish(&format!("v{version}"), &[asset]);
    fixture.server.route(
        &format!("/download/{archive}"),
        FakeResponse::ok(bytes).truncated_at(100),
    );
    let error = fixture.install(false).0.unwrap_err();
    assert!(matches!(error, UpdateError::Truncated { .. }), "{error:?}");
    fixture.left_as_it_was();
}

#[test]
fn an_archive_without_the_executable_as_a_file_leaves_the_executable() {
    let version = newer();
    let fixture = Fixture::reporting(&current_version());
    let entry = hanafubuki_agent_update::entry_path(
        &version,
        hanafubuki_agent_update::this_target().unwrap(),
    );
    let cases = [
        FakeArchive::new().file("hanafubuki-agent", reporting(&version)),
        FakeArchive::new().symlink(&entry, "/bin/sh"),
        // Over 16 times the archive: zeros compress far better than that.
        FakeArchive::new().file(&entry, vec![0; 4 * 1024 * 1024]),
    ];
    for archive in cases {
        let asset = FakeAsset::new(&FakeArchive::name(&version), archive.bytes());
        fixture.server.publish(&format!("v{version}"), &[asset]);
        let error = fixture.install(false).0.unwrap_err();
        assert!(matches!(error, UpdateError::NoEntry { .. }), "{error:?}");
        fixture.left_as_it_was();
    }
}

#[test]
fn the_token_is_sent_to_the_api_and_never_to_the_asset_host() {
    let version = newer();
    let fixture = Fixture::reporting(&current_version());
    fixture.publish(&version, reporting(&version));
    let _ = fixture.install(false);
    let requests = fixture.server.requests();
    assert!(fixture.asset_requests() > 0);
    for request in requests {
        let authorization = request.headers.get("authorization");
        if request.path.starts_with("/download/") {
            assert_eq!(authorization, None, "{}", request.path);
        } else {
            assert_eq!(authorization.map(String::as_str), Some("Bearer ghp_secret"));
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    #[test]
    fn a_newer_release_replaces_the_executable_by_a_rename() {
        let version = newer();
        let fixture = Fixture::reporting(&current_version());
        std::fs::set_permissions(&fixture.executable, std::fs::Permissions::from_mode(0o751))
            .unwrap();
        fixture.publish(&version, reporting(&version));
        let inode = std::fs::metadata(&fixture.executable).unwrap().ino();
        let (installed, seen) = fixture.install(false);
        assert_eq!(
            installed.unwrap(),
            Installed {
                version: version.clone(),
                backup: None,
                already: false
            }
        );
        assert_eq!(
            std::fs::read(&fixture.executable).unwrap(),
            reporting(&version)
        );
        let metadata = std::fs::metadata(&fixture.executable).unwrap();
        assert_ne!(
            metadata.ino(),
            inode,
            "a new file, not a write into the old"
        );
        assert_eq!(metadata.permissions().mode() & 0o777, 0o751);
        assert!(matches!(seen[0], Progress::Downloading { done: 0, .. }));
        assert_eq!(
            seen[seen.len() - 3..],
            [
                Progress::Verifying,
                Progress::Checking,
                Progress::Installing
            ]
        );
        assert_eq!(
            names(&fixture.directory),
            [".custom name.update.lock", "custom name"]
        );
    }

    #[test]
    fn force_reinstalls_the_same_version() {
        let current = current_version();
        let fixture = Fixture::reporting(&current);
        let script = format!("#!/bin/sh\necho 'hanafubuki-agent {current}'\n# reinstalled\n");
        fixture.publish(&current, script.clone());
        let installed = fixture.install(true).0.unwrap();
        assert!(!installed.already);
        assert_eq!(
            std::fs::read(&fixture.executable).unwrap(),
            script.as_bytes()
        );
    }

    #[test]
    fn an_executable_another_process_already_updated_is_left_alone() {
        let version = newer();
        let fixture = Fixture::reporting(&version);
        fixture.publish(&version, reporting(&version));
        let installed = fixture.install(false).0.unwrap();
        assert!(installed.already);
        assert_eq!(fixture.asset_requests(), 0);
        fixture.left_as_it_was();
    }

    #[test]
    fn an_executable_newer_than_the_release_is_never_replaced_by_it() {
        let release = newer();
        let installed_version = Version::new(release.major + 1, 0, 0);
        let fixture = Fixture::reporting(&installed_version);
        fixture.publish(&release, reporting(&release));
        let installed = fixture.install(false).0.unwrap();
        assert_eq!(
            installed,
            Installed {
                version: installed_version,
                backup: None,
                already: true
            }
        );
        assert_eq!(fixture.asset_requests(), 0);
        fixture.left_as_it_was();
    }

    #[test]
    fn a_new_executable_that_fails_or_reports_another_version_leaves_the_executable() {
        let version = newer();
        let fixture = Fixture::reporting(&current_version());
        let cases = [
            (
                "#!/bin/sh\necho 'cannot run here' >&2\nexit 1\n".to_string(),
                "does not run here (cannot run here)",
            ),
            (
                "#!/bin/sh\necho 'hanafubuki-agent 0.0.9'\n".to_string(),
                "reports \"hanafubuki-agent 0.0.9\"",
            ),
        ];
        for (executable, expected) in cases {
            fixture.publish(&version, executable);
            let error = fixture.install(false).0.unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            fixture.left_as_it_was();
        }
    }

    #[test]
    fn through_a_symlink_the_link_stays_and_its_target_changes() {
        let version = newer();
        let fixture = Fixture::reporting(&current_version());
        fixture.publish(&version, reporting(&version));
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("hanafubuki-agent");
        std::os::unix::fs::symlink(&fixture.executable, &link).unwrap();
        let installation = Installation::at(&link).unwrap();
        install_blocking(
            &fixture.client(),
            &installation,
            &fixture.release(),
            false,
            &mut |_| {},
        )
        .unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap(), fixture.executable);
        assert_eq!(std::fs::read(&link).unwrap(), reporting(&version));
        assert_eq!(names(links.path()), ["hanafubuki-agent"]);
    }

    #[test]
    fn an_unwritable_directory_is_refused_before_the_download() {
        let version = newer();
        let fixture = Fixture::reporting(&current_version());
        fixture.publish(&version, reporting(&version));
        let installation = fixture.installation();
        let release = fixture.release();
        std::fs::set_permissions(&fixture.directory, std::fs::Permissions::from_mode(0o555))
            .unwrap();
        let result = install_blocking(
            &fixture.client(),
            &installation,
            &release,
            false,
            &mut |_| {},
        );
        std::fs::set_permissions(&fixture.directory, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        // root writes anywhere.
        if let Err(error) = result {
            assert!(matches!(error, UpdateError::Write { .. }), "{error:?}");
            assert!(error.to_string().ends_with("nothing was changed."));
            assert_eq!(fixture.asset_requests(), 0);
            fixture.left_as_it_was();
        }
    }
}
