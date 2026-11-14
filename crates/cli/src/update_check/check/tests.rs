use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use hanafubuki_agent_tui::TuiEvents;
use hanafubuki_agent_update::test_support::{FakeRelease, FakeResponse};
use hanafubuki_agent_update::UpdateProxy;

use super::super::cache::{UpdateCache, UPDATE_CHECK_FILE};
use super::*;

const NOW: u64 = 1_763_107_200_000;

fn version(text: &str) -> Version {
    text.parse().unwrap()
}

fn cache_text(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(UPDATE_CHECK_FILE)).ok()
}

/// The check of `current` against a release answering `answer`: what it
/// returned, and the cache it left.
fn check_against(
    answer: FakeResponse,
    current: &str,
) -> (Option<Version>, UpdateCache, FakeRelease) {
    let release = FakeRelease::start();
    release.answer_latest(answer);
    let dir = tempfile::tempdir().unwrap();
    let newer = check(&release.client(None), dir.path(), &version(current), NOW);
    (newer, UpdateCache::load(dir.path()), release)
}

fn published(tag: &str) -> FakeResponse {
    let release = FakeRelease::start();
    FakeResponse::json(&release.release_json(tag, &[]))
}

#[test]
fn a_newer_release_is_cached_and_returned_with_one_anonymous_request() {
    let (newer, cache, release) = check_against(published("v0.3.0"), "0.2.0");
    assert_eq!(newer, Some(version("0.3.0")));
    assert_eq!(cache.latest_version, Some(version("0.3.0")));
    assert_eq!(cache.checked_at_ms, Some(NOW));
    let requests = release.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("user-agent").map(String::as_str),
        Some(concat!("hanafubuki-agent/", env!("CARGO_PKG_VERSION")))
    );
    assert!(!requests[0].headers.contains_key("authorization"));
    assert!(!requests[0].headers.contains_key("cookie"));
}

/// Up to date, or a build newer than the latest release: the answer is
/// kept (no request again for 20 hours), and nothing is shown.
#[test]
fn the_same_or_an_older_release_is_cached_and_shows_nothing() {
    for (tag, current) in [
        ("v0.2.0", "0.2.0"),
        ("v0.2.0", "0.2.0+local"),
        ("v0.2.1", "0.3.0"),
    ] {
        let (newer, cache, _release) = check_against(published(tag), current);
        assert_eq!(newer, None, "{tag} against {current}");
        assert_eq!(
            cache.latest_version,
            Some(version(tag.trim_start_matches('v')))
        );
        assert!(!cache.is_due(NOW + 1), "{tag} against {current}");
    }
}

/// A pre-release, a refusal, an answer it cannot read, a release missing:
/// the cache stays as it was, and nothing is shown.
#[test]
fn a_failed_lookup_leaves_the_cache_as_it_was() {
    let release = FakeRelease::start();
    let mut prerelease = release.release_json("v0.3.0-beta.1", &[]);
    prerelease["prerelease"] = true.into();
    let answers = [
        FakeResponse::json(&prerelease),
        FakeResponse::json(&release.release_json("nightly", &[])),
        FakeResponse::status(403),
        FakeResponse::status(404),
        FakeResponse::status(502),
        FakeResponse::ok("not json {"),
    ];
    for answer in answers {
        let dir = tempfile::tempdir().unwrap();
        let earlier = r#"{"latestVersion": "0.2.5", "checkedAtMs": 5}"#;
        fs::write(dir.path().join(UPDATE_CHECK_FILE), earlier).unwrap();
        release.answer_latest(answer.clone());
        let newer = check(&release.client(None), dir.path(), &version("0.2.0"), NOW);
        assert_eq!(newer, None, "{answer:?}");
        assert_eq!(
            cache_text(dir.path()).as_deref(),
            Some(earlier),
            "{answer:?}"
        );
    }
}

#[test]
fn an_unreachable_or_slow_server_leaves_no_cache() {
    // Nothing listens on a server once dropped.
    let gone = FakeRelease::start();
    let client = gone.client(None);
    drop(gone);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(check(&client, dir.path(), &version("0.2.0"), NOW), None);
    assert_eq!(cache_text(dir.path()), None);

    let slow = FakeRelease::start();
    slow.answer_latest(published("v0.3.0").delayed(Duration::from_secs(10)));
    let client = slow.client_with_timeout(None, Duration::from_millis(300));
    assert_eq!(check(&client, dir.path(), &version("0.2.0"), NOW), None);
    assert_eq!(cache_text(dir.path()), None);
}

/// A cache that exists but cannot be read is not written; the answer is
/// still shown.
#[test]
fn an_unwritable_cache_still_shows_a_newer_release() {
    let release = FakeRelease::start();
    release.answer_latest(published("v0.3.0"));
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(UPDATE_CHECK_FILE)).unwrap();
    let newer = check(&release.client(None), dir.path(), &version("0.2.0"), NOW);
    assert_eq!(newer, Some(version("0.3.0")));
    assert!(dir.path().join(UPDATE_CHECK_FILE).is_dir());
}

/// The environment variable that makes this test the child of
/// [`the_process_ends_while_a_lookup_is_held`]: the marker file it waits for.
const CHILD_MARKER: &str = "HANAFUBUKI_AGENT_UPDATE_CHECK_TEST_MARKER";

/// The child: starts the check as the interface's launch does, waits for
/// the parent's marker (the request has arrived), and returns while the
/// lookup is still held.
#[test]
fn child_starts_a_check_and_returns() {
    let Some(marker) = std::env::var_os(CHILD_MARKER) else {
        return;
    };
    let marker = Path::new(&marker).to_path_buf();
    let data_dir = marker.parent().unwrap().to_path_buf();
    // The test release URL, from this child's environment.
    let client = ReleaseClient::for_this_build(&UpdateProxy::Disabled, None).unwrap();
    let installation = Installation::at(&std::env::current_exe().unwrap()).unwrap();
    let events = TuiEvents::new();
    Check {
        client,
        data_dir,
        installation,
    }
    .start(events.update_notifier());
    let deadline = Instant::now() + Duration::from_secs(30);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Nothing waits for the check: the process ends at once while the
/// release lookup is held, and `GITHUB_TOKEN` is not sent.
#[test]
fn the_process_ends_while_a_lookup_is_held() {
    let release = FakeRelease::start();
    release.answer_latest(published("v0.3.0").delayed(Duration::from_secs(20)));
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("request-arrived");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "update_check::check::tests::child_starts_a_check_and_returns",
            "--test-threads=1",
        ])
        .env(CHILD_MARKER, &marker)
        .env("HANAFUBUKI_AGENT_TEST_RELEASE_URL", release.api_url())
        .env("GITHUB_TOKEN", "a-token-the-check-never-sends")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while release.requests().is_empty() {
        assert!(Instant::now() < deadline, "the check made no request");
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(&marker, "").unwrap();
    let marked = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if marked.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("the process waited for the held lookup");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
    let requests = release.requests();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(cache_text(dir.path()), None, "the answer never came");
}
