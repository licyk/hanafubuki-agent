use super::*;

#[test]
fn the_proxy_is_read_from_the_settings_string_form() {
    assert_eq!(UpdateProxy::from_setting("system"), UpdateProxy::System);
    assert_eq!(
        UpdateProxy::from_setting(" disabled "),
        UpdateProxy::Disabled
    );
    assert_eq!(
        UpdateProxy::from_setting("socks5h://user:secret@proxy:1080"),
        UpdateProxy::Explicit("socks5h://user:secret@proxy:1080".to_string())
    );
    assert_eq!(UpdateProxy::default(), UpdateProxy::System);
}

#[test]
fn neither_the_proxy_url_nor_the_token_is_shown() {
    let proxy = UpdateProxy::from_setting("http://user:secret@proxy:8080");
    assert_eq!(format!("{proxy:?}"), "Explicit(<redacted>)");
    let client = ReleaseClient::new(
        Url::parse("https://example.com/latest").unwrap(),
        false,
        &proxy,
        Some("ghp_secret".to_string()),
        LOOKUP_TIMEOUT,
    )
    .unwrap();
    let shown = format!("{client:?}");
    assert!(!shown.contains("secret"), "{shown}");
    assert!(shown.contains("token: true"), "{shown}");
    assert!(client.token.as_ref().unwrap().is_sensitive());
}

#[test]
fn an_empty_token_is_no_token_and_a_bad_one_is_refused() {
    let new = |token: &str| {
        ReleaseClient::new(
            Url::parse("https://example.com/latest").unwrap(),
            false,
            &UpdateProxy::Disabled,
            Some(token.to_string()),
            LOOKUP_TIMEOUT,
        )
    };
    assert!(new("").unwrap().token.is_none());
    assert_eq!(new("a\nb").unwrap_err(), UpdateError::InvalidToken);
}

#[test]
fn an_unusable_proxy_is_refused() {
    let error = ReleaseClient::new(
        Url::parse("https://example.com/latest").unwrap(),
        false,
        &UpdateProxy::from_setting("not a url"),
        None,
        LOOKUP_TIMEOUT,
    )
    .unwrap_err();
    assert_eq!(error, UpdateError::Proxy);
}

#[test]
fn each_status_maps_to_its_error() {
    for (status, expected) in [
        (200, None),
        (204, None),
        (401, Some(UpdateError::TokenRejected)),
        (403, Some(UpdateError::RateLimited { status: 403 })),
        (429, Some(UpdateError::RateLimited { status: 429 })),
        (404, Some(UpdateError::NoRelease)),
        (500, Some(UpdateError::Status { status: 500 })),
        (302, Some(UpdateError::Status { status: 302 })),
    ] {
        assert_eq!(
            status_error(StatusCode::from_u16(status).unwrap()),
            expected,
            "{status}"
        );
    }
}

#[cfg(debug_assertions)]
#[test]
fn a_debug_build_looks_up_only_the_test_url() {
    use std::ffi::OsString;

    let endpoint = |value: &str| test_endpoint(Some(OsString::from(value).as_os_str()));
    assert_eq!(test_endpoint(None).unwrap_err(), UpdateError::DebugBuild);
    assert_eq!(endpoint("  ").unwrap_err(), UpdateError::DebugBuild);
    for url in [
        "http://127.0.0.1:8765/latest.json",
        "http://[::1]:8765/latest.json",
        "http://localhost/latest",
        "https://releases.example.com/latest",
    ] {
        let (parsed, loopback_http) = endpoint(url).unwrap();
        assert_eq!(parsed.as_str(), Url::parse(url).unwrap().as_str());
        assert!(loopback_http, "{url}");
    }
    for url in [
        "http://example.com/latest",
        "http://192.168.1.2/latest",
        "file:///tmp/latest.json",
        "127.0.0.1:8765",
        "not a url",
    ] {
        assert!(
            matches!(endpoint(url), Err(UpdateError::TestUrl { .. })),
            "{url}"
        );
    }
}

#[cfg(all(debug_assertions, unix))]
#[test]
fn a_test_url_that_is_not_utf8_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let error = test_endpoint(Some(OsStr::from_bytes(b"http://127.0.0.1/\xff"))).unwrap_err();
    assert!(matches!(error, UpdateError::TestUrl { .. }));
}

/// Runs only in a release build (`cargo test --release`): the test variable
/// does not exist there, so the lookup asks GitHub whatever is set.
#[cfg(not(debug_assertions))]
#[test]
fn a_release_build_cannot_be_redirected() {
    std::env::set_var(
        "HANAFUBUKI_AGENT_TEST_RELEASE_URL",
        "http://127.0.0.1:1/latest",
    );
    let client = ReleaseClient::for_this_build(&UpdateProxy::Disabled, None).unwrap();
    assert_eq!(client.endpoint.as_str(), LATEST_RELEASE_API);
    assert!(!client.loopback_http);
}
