use super::bounded_trace_content;

#[test]
fn bounded_content_passes_short_values_through_unchanged() {
    let value = "hello world";
    assert_eq!(bounded_trace_content(value), value);
}

#[test]
fn bounded_content_truncates_large_values() {
    let value = "a".repeat(128 * 1024);
    let bounded = bounded_trace_content(&value);
    assert!(bounded.len() <= 64 * 1024);
    assert!(bounded.ends_with('a'));
    assert!(value.starts_with(&bounded));
}

#[test]
fn bounded_content_never_splits_utf8_code_points() {
    let value = "好".repeat(64 * 1024);
    let bounded = bounded_trace_content(&value);
    assert!(bounded.len() <= 64 * 1024);
    assert!(bounded.is_char_boundary(bounded.len()));
    assert!(bounded.chars().all(|character| character == '好'));
}
