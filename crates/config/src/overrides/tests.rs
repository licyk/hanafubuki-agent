// Derived from OpenAI Codex (https://github.com/openai/codex), the tests of
// `codex-rs/common/src/config_override.rs` at tag `rust-v0.93.0`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified for hanafubuki-agent: tests of the parsed document added.

use super::*;

#[test]
fn parses_basic_scalar() {
    let v = parse_toml_value("42").expect("parse");
    assert_eq!(v.as_integer(), Some(42));
}

#[test]
fn parses_bool() {
    let true_literal = parse_toml_value("true").expect("parse");
    assert_eq!(true_literal.as_bool(), Some(true));

    let false_literal = parse_toml_value("false").expect("parse");
    assert_eq!(false_literal.as_bool(), Some(false));
}

#[test]
fn fails_on_unquoted_string() {
    assert!(parse_toml_value("hello").is_err());
}

#[test]
fn parses_array() {
    let v = parse_toml_value("[1, 2, 3]").expect("parse");
    let arr = v.as_array().expect("array");
    assert_eq!(arr.len(), 3);
}

#[test]
fn parses_inline_table() {
    let v = parse_toml_value("{a = 1, b = 2}").expect("parse");
    let tbl = v.as_table().expect("table");
    assert_eq!(tbl.get("a").unwrap().as_integer(), Some(1));
    assert_eq!(tbl.get("b").unwrap().as_integer(), Some(2));
}

#[test]
fn dotted_keys_build_nested_tables_and_a_later_override_wins() {
    let overrides = ConfigOverrides::parse(&[
        "memory.enabled=true",
        "memory.maxUnusedDays=7",
        "logLevel=debug",
        "permissionMode=\"manual\"",
        "permissionMode='full_access'",
        "network.proxy=http://127.0.0.1:8080/?a=b",
    ])
    .unwrap();
    let expected: Table = toml::from_str(
        r#"
        logLevel = "debug"
        permissionMode = "full_access"
        [memory]
        enabled = true
        maxUnusedDays = 7
        [network]
        proxy = "http://127.0.0.1:8080/?a=b"
        "#,
    )
    .unwrap();
    assert_eq!(overrides.document(), &expected);
}

#[test]
fn an_override_without_a_key_or_an_equals_sign_is_refused() {
    assert_eq!(
        ConfigOverrides::parse(&["memory.enabled"]).unwrap_err(),
        "Invalid override (missing '='): memory.enabled"
    );
    assert_eq!(
        ConfigOverrides::parse(&[" =true"]).unwrap_err(),
        "Empty key in override:  =true"
    );
    assert!(ConfigOverrides::parse::<&str>(&[]).unwrap().is_empty());
}
