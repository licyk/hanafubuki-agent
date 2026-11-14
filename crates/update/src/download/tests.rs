use std::time::Duration;

use reqwest::Url;

use super::*;
use crate::test_support::{FakeRelease, FakeResponse};

const ARCHIVE: &str = "hanafubuki-agent-9.9.9-x86_64-unknown-linux-gnu.tar.gz";

struct Fixture {
    server: FakeRelease,
    directory: tempfile::TempDir,
    body: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            server: FakeRelease::start(),
            directory: tempfile::tempdir().unwrap(),
            body: (0..200_000u32).map(|index| (index % 251) as u8).collect(),
        }
    }

    fn serve(&self, response: FakeResponse) {
        self.server.route(&format!("/download/{ARCHIVE}"), response);
    }

    fn asset(&self, size: u64) -> ReleaseAsset {
        ReleaseAsset {
            name: ARCHIVE.to_string(),
            url: Url::parse(&self.server.asset_url(ARCHIVE)).unwrap(),
            size,
            sha256: None,
        }
    }

    fn destination(&self) -> std::path::PathBuf {
        self.directory.path().join(ARCHIVE)
    }

    async fn run(
        &self,
        asset: &ReleaseAsset,
        sha256: [u8; 32],
    ) -> (Result<(), UpdateError>, Vec<Progress>) {
        let client = self.server.client(Some("ghp_secret".to_string()));
        let destination = self.destination();
        let mut seen = Vec::new();
        let result = Download {
            http: client.http(),
            asset,
            sha256,
            destination: &destination,
            directory: "/the/directory",
            stall: Duration::from_millis(500),
            progress_every: Duration::ZERO,
        }
        .run(&mut |progress| seen.push(progress))
        .await;
        (result, seen)
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[tokio::test]
async fn the_archive_is_streamed_to_its_file_with_progress_and_its_digest_compared() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()));
    let total = fixture.body.len() as u64;
    let (result, seen) = fixture
        .run(&fixture.asset(total), digest(&fixture.body))
        .await;
    result.unwrap();
    assert_eq!(std::fs::read(fixture.destination()).unwrap(), fixture.body);
    assert_eq!(
        seen.first(),
        Some(&Progress::Downloading { done: 0, total })
    );
    assert_eq!(
        &seen[seen.len() - 2..],
        [
            Progress::Downloading { done: total, total },
            Progress::Verifying
        ]
    );
    let dones: Vec<u64> = seen
        .iter()
        .filter_map(|progress| match progress {
            Progress::Downloading { done, .. } => Some(*done),
            _ => None,
        })
        .collect();
    assert!(dones.windows(2).all(|pair| pair[0] <= pair[1]), "{dones:?}");
}

#[tokio::test]
async fn the_token_never_reaches_the_asset_host() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()));
    let asset = fixture.asset(fixture.body.len() as u64);
    fixture.run(&asset, digest(&fixture.body)).await.0.unwrap();
    let requests = fixture.server.requests();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(
        requests[0].headers["user-agent"],
        format!("hanafubuki-agent/{}", env!("CARGO_PKG_VERSION"))
    );
}

#[tokio::test]
async fn another_digest_fails_after_the_whole_download() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()));
    let asset = fixture.asset(fixture.body.len() as u64);
    let (result, seen) = fixture.run(&asset, digest(b"another archive")).await;
    let error = result.unwrap_err();
    assert_eq!(
        error,
        UpdateError::DigestMismatch {
            archive: ARCHIVE.to_string()
        }
    );
    assert!(error
        .to_string()
        .ends_with("nothing was changed. Try again."));
    assert_eq!(seen.last(), Some(&Progress::Verifying));
}

#[tokio::test]
async fn a_body_shorter_than_the_release_says_ended_early() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()));
    let listed = fixture.body.len() as u64 * 2;
    let (result, _) = fixture
        .run(&fixture.asset(listed), digest(&fixture.body))
        .await;
    assert_eq!(
        result.unwrap_err(),
        UpdateError::Truncated {
            archive: ARCHIVE.to_string(),
            received: fixture.body.len() as u64,
            expected: listed,
        }
    );
}

#[tokio::test]
async fn a_connection_closed_before_its_length_ended_early() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()).truncated_at(1000));
    let total = fixture.body.len() as u64;
    let (result, _) = fixture
        .run(&fixture.asset(total), digest(&fixture.body))
        .await;
    let error = result.unwrap_err();
    assert!(
        matches!(error, UpdateError::Truncated { received, expected, .. } if received <= 1000 && expected == total),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("ended after 0.0 of 0.2 MiB"),
        "{error}"
    );
}

#[tokio::test]
async fn a_body_longer_than_the_release_says_stops_at_once() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()));
    let (result, _) = fixture
        .run(&fixture.asset(1000), digest(&fixture.body))
        .await;
    assert_eq!(
        result.unwrap_err(),
        UpdateError::Overlong {
            archive: ARCHIVE.to_string()
        }
    );
    assert!(std::fs::metadata(fixture.destination()).unwrap().len() <= 1000);
}

#[tokio::test]
async fn a_download_that_stalls_fails() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::ok(fixture.body.clone()).paused_after(100, Duration::from_secs(5)));
    let started = Instant::now();
    let total = fixture.body.len() as u64;
    let (result, _) = fixture
        .run(&fixture.asset(total), digest(&fixture.body))
        .await;
    let error = result.unwrap_err();
    assert!(
        matches!(&error, UpdateError::Download { reason, .. } if reason.starts_with("nothing arrived")),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[tokio::test]
async fn an_answer_that_is_not_a_success_fails() {
    let fixture = Fixture::new();
    fixture.serve(FakeResponse::status(404));
    let (result, _) = fixture.run(&fixture.asset(10), [0; 32]).await;
    assert_eq!(
        result.unwrap_err(),
        UpdateError::Download {
            archive: ARCHIVE.to_string(),
            reason: "the download host answered 404".to_string(),
        }
    );
    assert!(!fixture.destination().exists());
}

#[tokio::test]
async fn a_redirect_is_followed_only_to_https_or_the_test_urls_loopback() {
    let fixture = Fixture::new();
    fixture
        .server
        .route("/elsewhere", FakeResponse::ok(fixture.body.clone()));
    fixture.serve(FakeResponse::redirect(&fixture.server.url("/elsewhere")));
    let total = fixture.body.len() as u64;
    fixture
        .run(&fixture.asset(total), digest(&fixture.body))
        .await
        .0
        .unwrap();

    // TEST-NET-1: not loopback, and never routed anywhere.
    fixture.serve(FakeResponse::redirect("http://192.0.2.1:9/archive"));
    let (result, _) = fixture
        .run(&fixture.asset(total), digest(&fixture.body))
        .await;
    let error = result.unwrap_err();
    assert!(
        matches!(&error, UpdateError::Download { reason, .. } if reason.contains("not https")),
        "{error:?}"
    );
}
