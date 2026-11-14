use serde_json::json;

use super::*;

const DIGEST: &str = "sha256:6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b";

fn asset(name: &str, url: &str) -> Value {
    json!({ "name": name, "browser_download_url": url, "size": 10, "digest": DIGEST })
}

fn decode(value: Value) -> Result<Release, UpdateError> {
    Release::from_api_json(value.to_string().as_bytes(), false)
}

fn reason(error: UpdateError) -> String {
    match error {
        UpdateError::NotUnderstood { reason } => reason,
        other => panic!("not NotUnderstood: {other:?}"),
    }
}

#[test]
fn a_release_reads_with_unknown_fields_and_without_optional_ones() {
    let release = decode(json!({
        "tag_name": "v0.3.0",
        "future": { "anything": [1, 2] },
        "assets": [{
            "name": "a.tar.gz",
            "browser_download_url": "https://example.com/a.tar.gz",
            "size": 10,
            "digest": DIGEST,
            "uploader": { "login": "someone" },
        }],
    }))
    .unwrap();
    assert_eq!(release.version, Version::new(0, 3, 0));
    let asset = release.asset("a.tar.gz").unwrap();
    assert_eq!(asset.size, 10);
    assert_eq!(asset.url.as_str(), "https://example.com/a.tar.gz");
    assert_eq!(hex::encode(asset.sha256.unwrap()), &DIGEST[7..]);
    assert!(decode(json!({ "tag_name": "v0.3.0" }))
        .unwrap()
        .assets
        .is_empty());
    assert!(
        decode(json!({ "tag_name": "v0.3.0", "draft": null, "assets": null }))
            .unwrap()
            .assets
            .is_empty()
    );
}

#[test]
fn an_asset_that_does_not_read_is_left_out_and_the_rest_kept() {
    let release = decode(json!({
        "tag_name": "v0.3.0",
        "assets": [
            asset("good", "https://example.com/good"),
            { "name": "no-size", "browser_download_url": "https://example.com/x" },
            { "name": "bad-size", "browser_download_url": "https://example.com/x", "size": -1 },
            { "browser_download_url": "https://example.com/x", "size": 1 },
            asset("not-a-url", "not a url"),
            asset("plain-http", "http://example.com/x"),
            asset("loopback-http", "http://127.0.0.1:1/x"),
            asset("ftp", "ftp://example.com/x"),
            "a string",
            7,
        ],
    }))
    .unwrap();
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["good"]);
}

#[test]
fn plain_http_to_this_machine_is_taken_only_from_the_test_url() {
    let body = json!({
        "tag_name": "v0.3.0",
        "assets": [
            asset("v4", "http://127.0.0.1:9/x"),
            asset("v4-other", "http://127.8.9.10/x"),
            asset("v6", "http://[::1]:9/x"),
            asset("name", "http://LOCALHOST:9/x"),
            asset("remote", "http://example.com/x"),
            asset("lookalike", "http://127.0.0.1.example.com/x"),
        ],
    })
    .to_string();
    let release = Release::from_api_json(body.as_bytes(), true).unwrap();
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["v4", "v4-other", "v6", "name"]);
    assert!(Release::from_api_json(body.as_bytes(), false)
        .unwrap()
        .assets
        .is_empty());
}

#[test]
fn only_a_sha256_digest_of_64_hex_digits_is_kept() {
    let upper = format!("sha256:{}", DIGEST[7..].to_ascii_uppercase());
    let short = &DIGEST[..DIGEST.len() - 2];
    let long = format!("{DIGEST}00");
    let sha512 = format!("sha512:{}", &DIGEST[7..]);
    let not_hex = format!("sha256:{}", "z".repeat(64));
    let cases = [
        (json!(upper), true),
        (json!(short), false),
        (json!(long), false),
        (json!(sha512), false),
        (json!(not_hex), false),
        (json!(DIGEST[7..]), false),
        (json!(null), false),
        (json!(42), false),
    ];
    for (digest, kept) in cases {
        let mut value = asset("a", "https://example.com/a");
        value["digest"] = digest.clone();
        let release = decode(json!({ "tag_name": "v1.0.0", "assets": [value] })).unwrap();
        // A digest of the wrong type fails the asset; one of the wrong form
        // only its digest.
        let sha256 = release.asset("a").and_then(|asset| asset.sha256);
        assert_eq!(sha256.is_some(), kept, "{digest}");
    }
    let mut missing = asset("a", "https://example.com/a");
    missing.as_object_mut().unwrap().remove("digest");
    let release = decode(json!({ "tag_name": "v1.0.0", "assets": [missing] })).unwrap();
    assert_eq!(release.asset("a").unwrap().sha256, None);
}

#[test]
fn a_draft_a_pre_release_or_a_tag_that_is_not_a_version_is_not_understood() {
    let cases = [
        (
            json!({ "tag_name": "v0.3.0", "draft": true }),
            "it is a draft",
        ),
        (
            json!({ "tag_name": "v0.3.0", "prerelease": true }),
            "it is a pre-release",
        ),
        (
            json!({ "tag_name": "nightly" }),
            r#"the tag "nightly" is not v<version>"#,
        ),
        (
            json!({ "tag_name": "v0.3.0-beta.1" }),
            r#"the tag "v0.3.0-beta.1" is a pre-release version"#,
        ),
        (json!({ "name": "no tag" }), "it names no tag"),
        (json!({ "tag_name": null }), "it names no tag"),
        (json!({ "tag_name": 3 }), "it is not a release"),
        (
            json!({ "tag_name": "v0.3.0", "draft": "no" }),
            "it is not a release",
        ),
        (
            json!({ "tag_name": "v0.3.0", "assets": {} }),
            "it is not a release",
        ),
        (json!([]), "it is not a release"),
    ];
    for (body, expected) in cases {
        assert_eq!(
            reason(decode(body.clone()).unwrap_err()),
            expected,
            "{body}"
        );
    }
    for body in [&b"not json"[..], b"", b"{\"tag_name\": \"v1.0.0\""] {
        assert_eq!(
            reason(Release::from_api_json(body, false).unwrap_err()),
            "it is not a release"
        );
    }
}

#[test]
fn a_quoted_tag_is_escaped_and_cut_short() {
    let hostile = format!("\u{1b}]8;;evil\u{7}{}", "x".repeat(100));
    let text = decode(json!({ "tag_name": hostile }))
        .unwrap_err()
        .to_string();
    assert!(
        !text.contains('\u{1b}') && !text.contains('\u{7}'),
        "{text}"
    );
    assert!(text.contains(r#""\u{1b}]8;;evil\u{7}xxx"#), "{text}");
    assert!(text.contains("…"), "{text}");
    assert!(!text.contains(&"x".repeat(40)), "{text}");
}

#[test]
fn an_asset_is_found_by_its_exact_name() {
    let release = decode(json!({
        "tag_name": "v0.3.0",
        "assets": [asset("a.tar.gz", "https://example.com/a")],
    }))
    .unwrap();
    assert!(release.asset("a.tar.gz").is_some());
    for name in ["A.tar.gz", "a.tar.gz ", "a.tar", ""] {
        assert!(release.asset(name).is_none(), "{name:?}");
    }
}

#[test]
fn the_release_notes_url_is_built_from_the_version() {
    assert_eq!(
        release_notes_url(&Version::new(0, 3, 0)),
        "https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.0"
    );
}
