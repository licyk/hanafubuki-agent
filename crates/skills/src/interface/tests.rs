use super::*;

#[test]
fn an_interface_file_decorates_a_skill() {
    let parsed = parse_interface(
        b"interface:\n  display_name: Demo\n  short_description: Does a thing\n  brand_color: \"#2563EB\"\n  icon_small: assets/icon.svg\npolicy:\n  allow_implicit_invocation: false\ndependencies:\n  tools:\n    - type: mcp\n      value: search-server\n      description: Search\n      transport: streamable_http\n      url: https://example.test/mcp\n",
    );
    assert_eq!(parsed.interface.display_name.as_deref(), Some("Demo"));
    assert_eq!(parsed.interface.brand_color.as_deref(), Some("#2563EB"));
    assert_eq!(
        parsed.interface.icon_small.as_deref(),
        Some("assets/icon.svg")
    );
    assert!(!parsed.allow_implicit_invocation);
    assert_eq!(parsed.dependencies.len(), 1);
    assert_eq!(parsed.dependencies[0].server_id, "search-server");
}

#[test]
fn an_unreadable_interface_file_never_fails_the_skill() {
    // Decoration cannot invalidate a package. Every one of these used to be a
    // hard error that hid an otherwise valid Skill.
    for bytes in [
        b"not: [yaml".as_slice(),
        b"interface:\n  unknown_key: 1\n".as_slice(),
        b"\xff\xfe".as_slice(),
        b"".as_slice(),
    ] {
        let parsed = parse_interface(bytes);
        assert!(parsed.allow_implicit_invocation);
    }
}

#[test]
fn malformed_decoration_is_dropped_field_by_field() {
    let parsed = parse_interface(
        b"interface:\n  brand_color: red\n  icon_small: /etc/passwd\n  icon_large: ../escape.png\ndependencies:\n  tools:\n    - type: mcp\n      value: \"bad id!\"\n      description: no\n    - type: unknown\n      value: other\n",
    );
    assert_eq!(parsed.interface.brand_color, None);
    assert_eq!(parsed.interface.icon_small, None);
    assert_eq!(parsed.interface.icon_large, None);
    assert!(parsed.dependencies.is_empty());
}

#[test]
fn a_field_of_the_wrong_type_is_ignored_and_the_others_kept() {
    let parsed = parse_interface(
        b"interface:\n  display_name: 42\n  short_description: Does a thing\n  brand_color: \"#2563EB\"\npolicy:\n  allow_implicit_invocation: \"no\"\ndependencies:\n  tools:\n    - type: mcp\n      value: search-server\n      description: [not, text]\n    - just a string\n    - type: mcp\n      value: 5\n",
    );
    assert_eq!(parsed.interface.display_name, None);
    assert_eq!(
        parsed.interface.short_description.as_deref(),
        Some("Does a thing")
    );
    assert_eq!(parsed.interface.brand_color.as_deref(), Some("#2563EB"));
    assert!(parsed.allow_implicit_invocation);
    assert_eq!(parsed.dependencies.len(), 1);
    assert_eq!(parsed.dependencies[0].server_id, "search-server");
    assert_eq!(parsed.dependencies[0].description, "");
    assert_eq!(
        parsed.issues,
        vec![
            "ignored interface.display_name: expected a string, found a number",
            "ignored policy.allow_implicit_invocation: expected true or false, found a string",
            "ignored dependencies.tools[0].description: expected a string, found a list",
            "ignored dependencies.tools[1]: expected a mapping, found a string",
            "ignored dependencies.tools[2].value: expected a string, found a number",
        ]
    );
}

#[test]
fn a_file_that_is_not_yaml_is_ignored_with_one_issue() {
    assert_eq!(parse_interface(b"not: [yaml").issues.len(), 1);
    assert_eq!(parse_interface(b"\xff\xfe").issues.len(), 1);
    assert_eq!(parse_interface(b"- a list").issues.len(), 1);
    assert!(parse_interface(b"").issues.is_empty());
    assert!(parse_interface(b"interface:\n  unknown_key: 1\n")
        .issues
        .is_empty());
}
