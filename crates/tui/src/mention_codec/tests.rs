use super::*;

#[test]
fn labels_round_trip_unicode_and_markdown_without_losing_identity() {
    for label in ["@任务", "@build [debug](x)", "@a\\b", "$skill", "@a][@bad"] {
        let encoded = encode(label, "thread://52");
        let (display, bindings) = decode(&format!("before {encoded} after"));
        assert_eq!(display, format!("before {label} after"));
        assert_eq!(bindings.len(), 1);
        assert_eq!(&display[bindings[0].range.clone()], label);
        assert_eq!(bindings[0].path, "thread://52");
    }
}

#[test]
fn ordinary_markdown_is_not_a_bound_mention() {
    for text in [
        "[docs](https://example.org)",
        "[$foo](https://example.org)",
        "[@unfinished](",
        "[unrelated]",
    ] {
        let (display, bindings) = decode(text);
        assert_eq!(display, text);
        assert!(bindings.is_empty());
    }
}
