//! The bounds the integration tests cannot shorten: a stalled download and
//! a staged executable that never answers, each leaving the installed file
//! and no staging directory. The rest is `tests/engine`.

use std::time::Instant;

use super::*;
use crate::test_support::{FakeArchive, FakeAsset, FakeRelease, FakeResponse};

const SHORT: Bounds = Bounds {
    stall: Duration::from_millis(300),
    version: Duration::from_millis(300),
    progress_every: Duration::ZERO,
};

struct Fixture {
    directory: tempfile::TempDir,
    installation: Installation,
    server: FakeRelease,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("hanafubuki-agent");
        std::fs::write(&executable, b"installed").unwrap();
        let installation = Installation::at(&executable).unwrap();
        Self {
            directory,
            installation,
            server: FakeRelease::start(),
        }
    }

    async fn install(&self) -> Result<Installed, UpdateError> {
        let client = self.server.client(None);
        let release = client.latest().await?;
        install_within(
            &client,
            &self.installation,
            &release,
            false,
            &mut |_| {},
            SHORT,
        )
        .await
    }

    fn left_as_it_was(&self) {
        let mut names: Vec<String> = std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [".hanafubuki-agent.update.lock", "hanafubuki-agent"]);
        assert_eq!(
            std::fs::read(self.installation.executable().system()).unwrap(),
            b"installed"
        );
    }
}

fn newer() -> Version {
    Version::new(current_version().major + 1, 0, 0)
}

#[tokio::test]
async fn a_stalled_download_fails_and_leaves_the_executable() {
    let fixture = Fixture::new();
    let archive = FakeArchive::release(&newer(), "unused").bytes();
    let name = FakeArchive::name(&newer());
    fixture.server.publish(
        &format!("v{}", newer()),
        &[FakeAsset::new(&name, archive.clone())],
    );
    fixture.server.route(
        &format!("/download/{name}"),
        FakeResponse::ok(archive).paused_after(10, Duration::from_secs(5)),
    );
    let started = Instant::now();
    let error = fixture.install().await.unwrap_err();
    assert!(
        matches!(&error, UpdateError::Download { reason, .. } if reason.starts_with("nothing arrived")),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(4));
    fixture.left_as_it_was();
}

#[cfg(unix)]
#[tokio::test]
async fn a_staged_executable_that_never_answers_is_killed_and_left() {
    let fixture = Fixture::new();
    let archive = FakeArchive::release(&newer(), "#!/bin/sh\nexec sleep 30\n").bytes();
    fixture.server.publish(
        &format!("v{}", newer()),
        &[FakeAsset::new(&FakeArchive::name(&newer()), archive)],
    );
    std::fs::set_permissions(
        fixture.installation.executable().system(),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let started = Instant::now();
    let error = fixture.install().await.unwrap_err();
    assert_eq!(error, UpdateError::VersionTimeout { seconds: 0 });
    assert!(started.elapsed() < Duration::from_secs(10));
    fixture.left_as_it_was();
}
