use hanafubuki_agent_web_search::{WebSearchCategory, WebSearchProvider};
use serde_json::json;

use super::{fetch, search};

#[test]
fn web_search_arguments_are_bounded_and_default_nulls() {
    let (request, notes) = search::parse(&json!({
        "query": "  Rust async cancellation  ",
        "maximumResults": 10,
        "provider": "auto",
        "category": "it",
        "timeRange": "month",
        "region": "cn-zh"
    }))
    .unwrap();
    assert_eq!(request.query, "Rust async cancellation");
    assert!(notes.is_empty());

    let (defaults, _) = search::parse(&json!({
        "query": "defaults",
        "maximumResults": null,
        "provider": null,
        "category": null,
        "timeRange": null,
        "region": null
    }))
    .unwrap();
    assert_eq!(defaults.maximum_results, 5);
    assert_eq!(defaults.provider, WebSearchProvider::Auto);
    assert_eq!(defaults.category, WebSearchCategory::General);
    assert_eq!(defaults.time_range, None);
    assert_eq!(defaults.region, None);

    // The tool is the request's one check: the service takes it as valid.
    for arguments in [
        json!({"query": ""}),
        json!({"query": "   "}),
        json!({"query": "x".repeat(513)}),
        json!({"query": "bell\u{7}query"}),
        json!({"query": "valid", "region": ""}),
        json!({"query": "valid", "region": "x".repeat(17)}),
        json!({"query": "valid", "region": "cn_zh"}),
        json!({"query": "valid", "maximumResults": null, "provider": "raw_http", "category": null, "timeRange": null, "region": null}),
        json!("not an object"),
    ] {
        assert!(search::parse(&arguments).is_err(), "{arguments}");
    }
}

#[test]
fn web_search_clamps_its_result_limit_and_ignores_unknown_keys() {
    let (request, notes) = search::parse(&json!({
        "query": "valid", "maximumResults": 11, "provider": null, "category": null,
        "timeRange": null, "region": null
    }))
    .unwrap();
    assert_eq!(request.maximum_results, 10);
    assert_eq!(
        notes.notes(),
        ["maximumResults 11 is above its maximum; 10 was used"]
    );
    // The service's endpoint is configuration, never an argument: an extra
    // key changes nothing.
    let (plain, _) = search::parse(&json!({"query": "valid"})).unwrap();
    let (forged, _) = search::parse(
        &json!({"query": "valid", "endpoint": "http://127.0.0.1", "confirmed": true}),
    )
    .unwrap();
    assert_eq!(format!("{plain:?}"), format!("{forged:?}"));
}

#[test]
fn fetch_web_page_arguments_are_bounded_and_default_nulls() {
    let (request, _) = fetch::parse(&json!({
        "url": " https://example.com/reference#section ",
        "maximumContentBytes": 100000,
        "fromOffset": 4096,
        "contentGeneration": "a".repeat(64)
    }))
    .unwrap();
    assert_eq!(
        request.url.as_str(),
        "https://example.com/reference#section"
    );

    let (defaults, _) = fetch::parse(&json!({
        "url": "https://example.com/",
        "maximumContentBytes": null,
        "fromOffset": null,
        "contentGeneration": null
    }))
    .unwrap();
    assert_eq!(defaults.maximum_content_bytes, 50_000);
    assert_eq!(defaults.from_offset, 0);
    assert_eq!(defaults.content_generation, None);

    // URL safety and the paging contract stay strict.
    for arguments in [
        json!({"url": ""}),
        json!({"url": "file:///etc/passwd"}),
        json!({"url": "https://user:secret@example.com/"}),
        json!({"url": "https://example.com/", "fromOffset": 10}),
        json!({"url": "https://example.com/", "fromOffset": 10, "contentGeneration": "A".repeat(64)}),
        json!({"url": "https://example.com/", "fromOffset": null, "contentGeneration": "a".repeat(63)}),
    ] {
        assert!(fetch::parse(&arguments).is_err(), "{arguments}");
    }
}

#[test]
fn fetch_web_page_clamps_its_byte_limit_and_offset_to_their_caps() {
    let (request, notes) = fetch::parse(&json!({
        "url": "https://example.com/",
        "maximumContentBytes": 100001,
        "fromOffset": 1048577,
        "contentGeneration": "a".repeat(64)
    }))
    .unwrap();
    assert_eq!(request.maximum_content_bytes, 100_000);
    assert_eq!(request.from_offset, 1024 * 1024);
    assert_eq!(notes.notes().len(), 2);
    let (request, notes) = fetch::parse(&json!({
        "url": "https://example.com/", "maximumContentBytes": 0, "confirmed": true
    }))
    .unwrap();
    assert_eq!(request.maximum_content_bytes, 4);
    assert_eq!(
        notes.notes(),
        ["maximumContentBytes 0 is below its minimum; 4 was used"]
    );
}

#[test]
fn argument_errors_name_the_field() {
    assert_eq!(
        search::parse(&json!({"query": 7})).unwrap_err().to_string(),
        "invalid tool argument query: invalid type: integer `7`, expected a string"
    );
    assert_eq!(
        search::parse(&json!({"query": "x", "maximumResults": "many"}))
            .unwrap_err()
            .to_string(),
        "invalid tool argument maximumResults: must be an integer"
    );
    assert_eq!(
        fetch::parse(&json!([])).unwrap_err().to_string(),
        "tool arguments must be an object"
    );
}

/// Strict function schemas list every property as required-and-nullable;
/// an omitted nullable field means the same as `null`.
#[test]
fn omitted_nullable_fields_read_as_null() {
    let search = json!({
        "query": "strict schema",
        "maximumResults": null,
        "provider": null,
        "category": null,
        "timeRange": null,
        "region": null
    });
    let (complete, _) = search::parse(&search).unwrap();
    let mut missing = search;
    missing.as_object_mut().unwrap().remove("region");
    let (partial, _) = search::parse(&missing).unwrap();
    assert_eq!(format!("{complete:?}"), format!("{partial:?}"));

    let fetch_page = json!({
        "url": "https://example.com/",
        "maximumContentBytes": null,
        "fromOffset": null,
        "contentGeneration": null
    });
    fetch::parse(&fetch_page).unwrap();
    let mut missing = fetch_page;
    missing.as_object_mut().unwrap().remove("contentGeneration");
    fetch::parse(&missing).unwrap();
}
