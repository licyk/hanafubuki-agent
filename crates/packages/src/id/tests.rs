// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugin-common/src/plugin_id_tests.rs` and the identity
// tests of `codex-rs/core-plugins/src/store_tests.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).

use super::PluginId;

#[test]
fn accepts_dotted_plugin_names() {
    let id = PluginId::new("acme.tools", "marketplace").unwrap();

    assert_eq!(id.key(), "acme.tools@marketplace");
    assert_eq!(id.to_string(), "acme.tools@marketplace");
}

#[test]
fn marketplace_names_preserve_legacy_character_set() {
    PluginId::new("acme", "marketplace_name-1").unwrap();

    let error = PluginId::new("acme", "market.place").unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid marketplace name: only ASCII letters, digits, `_`, and `-` are allowed"
    );
}

#[test]
fn rejects_dot_path_segments() {
    assert!(PluginId::new(".", "marketplace").is_err());
    assert!(PluginId::new("plugin", "..").is_err());
}

#[test]
fn rejects_dots_that_can_alias_path_segments() {
    for name in [".plugin", "plugin.", "plugin..name"] {
        assert!(PluginId::new(name, "marketplace").is_err(), "{name}");
    }
}

#[test]
fn parse_splits_at_the_last_at_sign() {
    let id = PluginId::parse("demo@my-mkt").unwrap();

    assert_eq!(id, PluginId::new("demo", "my-mkt").unwrap());
    assert!(PluginId::parse("demo").is_err());
    assert!(PluginId::parse("@m").is_err());
    assert!(PluginId::parse("p@").is_err());
}

#[test]
fn parse_rejects_path_separators_in_key_segments() {
    assert_eq!(
        PluginId::parse("../../etc@debug").unwrap_err().to_string(),
        "invalid plugin name: dots must separate non-empty name segments in `../../etc@debug`"
    );
    assert_eq!(
        PluginId::parse("sample@../../etc").unwrap_err().to_string(),
        "invalid marketplace name: only ASCII letters, digits, `_`, and `-` are allowed in `sample@../../etc`"
    );
}
