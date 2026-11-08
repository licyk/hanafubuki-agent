use super::*;

/// The union of what the copies this replaced checked: the settings mirror
/// and search endpoint readers and writers, the Provider base URL, and the
/// web search endpoint, result and link checks.
#[test]
fn every_policy_needs_an_http_scheme_and_a_host() {
    for policy in [UrlPolicy::HTTP, UrlPolicy::PAGE, UrlPolicy::ENDPOINT] {
        assert!(policy.parse("https://example.com").is_ok());
        assert!(policy.parse("http://127.0.0.1:8080/search/").is_ok());
        assert_eq!(policy.parse("ftp://example.com"), Err(UrlRejection::Scheme));
        assert_eq!(policy.parse("file:///etc/hosts"), Err(UrlRejection::Scheme));
        assert_eq!(
            policy.parse("mailto:user@example.com"),
            Err(UrlRejection::Scheme)
        );
        assert!(matches!(
            policy.parse("not a url"),
            Err(UrlRejection::Invalid(_))
        ));
        assert!(matches!(policy.parse(""), Err(UrlRejection::Invalid(_))));
    }
    // A scheme without an authority never has a host.
    let no_host = Url::parse("data:text/plain,x").unwrap();
    let data = UrlPolicy {
        schemes: &["data"],
        ..UrlPolicy::HTTP
    };
    assert_eq!(data.check(&no_host), Err(UrlRejection::MissingHost));
}

#[test]
fn each_component_is_refused_only_where_the_policy_says() {
    let credentials = [
        "https://user@example.com",
        "https://user:secret@example.com",
        "https://:secret@example.com",
    ];
    for raw in credentials {
        assert!(UrlPolicy::HTTP.parse(raw).is_ok(), "{raw}");
        assert_eq!(
            UrlPolicy::PAGE.parse(raw),
            Err(UrlRejection::Credentials),
            "{raw}"
        );
        assert_eq!(
            UrlPolicy::ENDPOINT.parse(raw),
            Err(UrlRejection::Credentials),
            "{raw}"
        );
    }
    for policy in [UrlPolicy::HTTP, UrlPolicy::PAGE] {
        assert!(policy.parse("https://example.com/?q=1").is_ok());
        assert!(policy.parse("https://example.com/#top").is_ok());
    }
    assert_eq!(
        UrlPolicy::ENDPOINT.parse("https://example.com/?q=1"),
        Err(UrlRejection::Query)
    );
    assert_eq!(
        UrlPolicy::ENDPOINT.parse("https://example.com/?"),
        Err(UrlRejection::Query)
    );
    assert_eq!(
        UrlPolicy::ENDPOINT.parse("https://example.com/#top"),
        Err(UrlRejection::Fragment)
    );
}

#[test]
fn a_length_bound_counts_the_text_as_given() {
    let policy = UrlPolicy::ENDPOINT.within(MAX_URL_BYTES);
    let path = "a".repeat(MAX_URL_BYTES - "https://example.com/".len());
    let longest = format!("https://example.com/{path}");
    assert!(policy.parse(&longest).is_ok());
    assert_eq!(
        policy.parse(&format!("{longest}a")),
        Err(UrlRejection::TooLong {
            limit: MAX_URL_BYTES
        })
    );
    assert!(UrlPolicy::ENDPOINT.parse(&format!("{longest}a")).is_ok());
}
