//! The release lookup against the fake release server on loopback: the
//! request it sends, the answers it takes and those it refuses. No test
//! here reaches any other address.

use std::time::{Duration, Instant};

use hanafubuki_agent_update::test_support::{sha256_digest, FakeAsset, FakeRelease, FakeResponse};
use hanafubuki_agent_update::{
    asset_name, ReleaseClient, UpdateError, UpdateProxy, Version, RELEASE_TARGETS,
};
use serde_json::{json, Value};

/// The shape of GitHub's answer for v0.2.0: its eight assets with their
/// names, sizes, digests and every field GitHub adds. Synthetic: built in
/// that shape without contacting GitHub, so its digests are not the real
/// archives'.
const V0_2_0: &str = include_str!("fixtures/v0.2.0-synthetic.json");

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn the_request_names_the_binary_and_asks_for_githubs_json_without_a_token() {
    let server = FakeRelease::start();
    server.publish("v0.3.0", &[]);
    server.client(None).latest_blocking().unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.path,
        "/repos/licyk/hanafubuki-agent/releases/latest"
    );
    assert_eq!(request.headers["accept"], "application/vnd.github+json");
    assert_eq!(
        request.headers["user-agent"],
        format!("hanafubuki-agent/{}", env!("CARGO_PKG_VERSION"))
    );
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("cookie"));
}

#[test]
fn a_token_is_sent_only_when_given() {
    let server = FakeRelease::start();
    server.publish("v0.3.0", &[]);
    server
        .client(Some("ghp_example".to_string()))
        .latest_blocking()
        .unwrap();
    server
        .client(Some(String::new()))
        .latest_blocking()
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests[0].headers["authorization"], "Bearer ghp_example");
    assert!(!requests[1].headers.contains_key("authorization"));
}

#[test]
fn githubs_answer_for_v0_2_0_reads_with_an_archive_for_every_target() {
    let server = FakeRelease::start();
    server.answer_latest(FakeResponse::ok(V0_2_0));
    let release = server.client(None).latest_blocking().unwrap();
    assert_eq!(release.version, Version::new(0, 2, 0));
    assert_eq!(release.assets.len(), 8);
    for target in RELEASE_TARGETS {
        let name = asset_name(&release.version, target);
        let asset = release.asset(&name).unwrap_or_else(|| panic!("{name}"));
        assert!(asset.sha256.is_some(), "{name}");
        assert!(asset.size > 15_000_000, "{name}");
        assert_eq!(
            asset.url.as_str(),
            format!("https://github.com/licyk/hanafubuki-agent/releases/download/v0.2.0/{name}")
        );
    }
}

#[test]
fn a_published_release_lists_each_asset_with_its_size_and_digest() {
    let server = FakeRelease::start();
    let archive = FakeAsset::new("a.tar.gz", b"archive bytes".to_vec());
    let bare = FakeAsset::new("b.zip", b"other".to_vec()).without_digest();
    server.publish("v9.9.9", &[archive.clone(), bare]);
    let release = server.client(None).latest_blocking().unwrap();
    assert_eq!(release.version, Version::new(9, 9, 9));
    let asset = release.asset("a.tar.gz").unwrap();
    assert_eq!(asset.size, 13);
    assert_eq!(
        format!("sha256:{}", hex::encode(asset.sha256.unwrap())),
        sha256_digest(b"archive bytes")
    );
    assert_eq!(asset.url.as_str(), server.asset_url("a.tar.gz"));
    assert_eq!(release.asset("b.zip").unwrap().sha256, None);
}

#[test]
fn unknown_fields_are_ignored_and_an_asset_that_does_not_read_is_left_out() {
    let server = FakeRelease::start();
    let mut body = server.release_json("v0.3.0", &[FakeAsset::new("good", b"x".to_vec())]);
    body["something_new"] = json!({ "nested": [true] });
    let assets = body["assets"].as_array_mut().unwrap();
    assets[0]["new_field"] = json!("ignored");
    assets.push(json!({ "name": "no url", "size": 1 }));
    assets.push(json!({
        "name": "plain http elsewhere",
        "size": 1,
        "browser_download_url": "http://example.com/x",
        "digest": sha256_digest(b"x"),
    }));
    assets.push(json!("not an asset"));
    server.answer_latest(FakeResponse::json(&body));
    let release = server.client(None).latest_blocking().unwrap();
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["good"]);
}

#[test]
fn each_status_gives_its_error() {
    let server = FakeRelease::start();
    for (status, expected, sentence) in [
        (
            401,
            UpdateError::TokenRejected,
            "GitHub rejected GITHUB_TOKEN (401); unset it or use a valid token.",
        ),
        (
            403,
            UpdateError::RateLimited { status: 403 },
            "GitHub refused the release lookup (403): its rate limit was probably reached. \
             Try again later, or set GITHUB_TOKEN.",
        ),
        (
            429,
            UpdateError::RateLimited { status: 429 },
            "GitHub refused the release lookup (429): its rate limit was probably reached. \
             Try again later, or set GITHUB_TOKEN.",
        ),
        (
            404,
            UpdateError::NoRelease,
            "the repository has no published release yet (404).",
        ),
        (
            502,
            UpdateError::Status { status: 502 },
            "GitHub answered 502 to the release lookup; try again later.",
        ),
        (
            500,
            UpdateError::Status { status: 500 },
            "GitHub answered 500 to the release lookup; try again later.",
        ),
    ] {
        server.answer_latest(FakeResponse::status(status));
        let error = server.client(None).latest_blocking().unwrap_err();
        assert_eq!(error, expected, "{status}");
        assert_eq!(error.to_string(), sentence);
    }
}

#[test]
fn a_draft_a_pre_release_or_a_tag_that_is_not_a_version_is_refused() {
    let server = FakeRelease::start();
    let draft = json!({ "draft": true });
    let pre = json!({ "prerelease": true });
    for (tag, change, reason) in [
        ("v0.3.0", &draft, "it is a draft"),
        ("v0.3.0", &pre, "it is a pre-release"),
        (
            "nightly",
            &json!({}),
            "the tag \"nightly\" is not v<version>",
        ),
        (
            "v0.3.0-beta.1",
            &json!({}),
            "the tag \"v0.3.0-beta.1\" is a pre-release version",
        ),
    ] {
        let mut body = server.release_json(tag, &[]);
        for (key, value) in change.as_object().unwrap() {
            body[key] = value.clone();
        }
        server.answer_latest(FakeResponse::json(&body));
        let error = server.client(None).latest_blocking().unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "GitHub's answer is not a release this version understands ({reason}); \
                 nothing was changed."
            )
        );
    }
    server.answer_latest(FakeResponse::ok("<html>a proxy's page</html>"));
    let error = server.client(None).latest_blocking().unwrap_err();
    assert!(
        matches!(error, UpdateError::NotUnderstood { .. }),
        "{error}"
    );
}

#[test]
fn an_answer_over_one_mib_is_refused() {
    let server = FakeRelease::start();
    let padding = "x".repeat(1024 * 1024);
    let body: Value = json!({ "tag_name": "v0.3.0", "padding": padding });
    server.answer_latest(FakeResponse::json(&body));
    let error = server.client(None).latest_blocking().unwrap_err();
    assert_eq!(
        error,
        UpdateError::NotUnderstood {
            reason: "it is longer than 1 MiB".to_string()
        }
    );
    // Just under the bound reads.
    let padding = "x".repeat(1024 * 1024 - 64);
    server.answer_latest(FakeResponse::json(
        &json!({ "tag_name": "v0.3.0", "p": padding }),
    ));
    server.client(None).latest_blocking().unwrap();
}

#[test]
fn a_lookup_that_does_not_end_in_time_times_out() {
    let server = FakeRelease::start();
    let client = server.client_with_timeout(None, Duration::from_millis(300));
    // No answer at all, then an answer that stalls part-way.
    server.answer_latest(FakeResponse::ok(V0_2_0).delayed(Duration::from_secs(20)));
    let started = Instant::now();
    let error = client.latest_blocking().unwrap_err();
    assert_eq!(
        error,
        UpdateError::Unreachable {
            host: "127.0.0.1".to_string(),
            timed_out: true
        }
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        error.to_string(),
        "could not reach 127.0.0.1 (timed out); check the network and network.proxy. \
         Nothing was changed."
    );
    server.answer_latest(FakeResponse::ok(V0_2_0).paused_after(100, Duration::from_secs(20)));
    let started = Instant::now();
    assert_eq!(client.latest_blocking().unwrap_err().kind(), "timed out");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_refused_connection_or_a_cut_answer_cannot_reach_the_api() {
    let server = FakeRelease::start();
    server.answer_latest(FakeResponse::ok(V0_2_0).truncated_at(100));
    let error = server.client(None).latest_blocking().unwrap_err();
    assert_eq!(
        error,
        UpdateError::Unreachable {
            host: "127.0.0.1".to_string(),
            timed_out: false
        }
    );
    // A server that has stopped: nothing listens on its port.
    let gone = FakeRelease::start();
    let client = gone.client(None);
    drop(gone);
    let error = client.latest_blocking().unwrap_err();
    assert_eq!(error.kind(), "unreachable");
    assert_eq!(
        error.to_string(),
        "could not reach 127.0.0.1 (the connection failed); check the network and \
         network.proxy. Nothing was changed."
    );
}

#[test]
fn one_client_looks_up_again_from_a_new_runtime_each_time() {
    let server = FakeRelease::start();
    server.publish("v0.3.0", &[]);
    let client = server.client(None);
    for _ in 0..3 {
        let client = client.clone();
        std::thread::spawn(move || client.latest_blocking())
            .join()
            .unwrap()
            .unwrap();
    }
    client.latest_blocking().unwrap();
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn the_async_lookup_runs_on_the_callers_runtime() {
    let server = FakeRelease::start();
    server.publish("v1.2.3", &[]);
    let client = server.client(None);
    let release = runtime().block_on(client.latest()).unwrap();
    assert_eq!(release.version, Version::new(1, 2, 3));
}

#[test]
fn a_debug_build_without_the_test_url_looks_nothing_up() {
    if std::env::var_os("HANAFUBUKI_AGENT_TEST_RELEASE_URL").is_some() {
        // Set around this run: the inner rule is tested in the crate.
        return;
    }
    let error = ReleaseClient::for_this_build(&UpdateProxy::System, None).unwrap_err();
    assert_eq!(error, UpdateError::DebugBuild);
    assert_eq!(
        error.to_string(),
        "a debug build looks up releases only at HANAFUBUKI_AGENT_TEST_RELEASE_URL, which is \
         not set; nothing was changed."
    );
}

#[test]
fn the_fake_serves_assets_and_its_failure_hooks() {
    let server = FakeRelease::start();
    server.publish("v0.3.0", &[FakeAsset::new("a.bin", vec![7; 1000])]);
    let runtime = runtime();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let get = |url: String| {
        runtime.block_on(async {
            let response = http.get(url).send().await?;
            let status = response.status().as_u16();
            response.bytes().await.map(|body| (status, body.len()))
        })
    };
    assert_eq!(get(server.asset_url("a.bin")).unwrap(), (200, 1000));
    assert_eq!(get(server.url("/nothing")).unwrap().0, 404);
    server.route(
        "/download/a.bin",
        FakeResponse::ok(vec![7; 1000]).truncated_at(10),
    );
    assert!(get(server.asset_url("a.bin")).is_err());
    server.route(
        "/moved",
        FakeResponse::redirect(&server.url("/download/b.bin")),
    );
    server.route("/download/b.bin", FakeResponse::ok(vec![1; 5]));
    assert_eq!(get(server.url("/moved?x=1")).unwrap(), (200, 5));
    let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            "/download/a.bin",
            "/nothing",
            "/download/a.bin",
            "/moved?x=1",
            "/download/b.bin"
        ]
    );
}
