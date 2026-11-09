use super::*;

#[test]
fn credentials_resolve_from_settings_and_stay_out_of_debug_output() {
    let key = McpCredentialKey::parse("mcp-local-token").unwrap();
    let configured = |generation: u64| {
        BTreeMap::from([(
            key.clone(),
            McpCredentialSettings {
                value: "private-value".into(),
                generation,
            },
        )])
    };

    let secrets = resolve_connection_secrets(&stdio_config(), &configured(3)).unwrap();
    assert!(!format!("{:?}", secrets.stdio_environment).contains("private-value"));

    let changed = resolve_connection_secrets(&stdio_config(), &configured(4)).unwrap();
    assert_ne!(
        secrets.credential_reference_generation,
        changed.credential_reference_generation
    );

    // A referenced but unconfigured credential is a hard failure, not a blank.
    assert!(resolve_connection_secrets(&stdio_config(), &BTreeMap::new()).is_err());
}

#[test]
fn credential_values_are_bounded_but_never_transformed() {
    assert!(validate_credential_value("").is_err());
    assert!(validate_credential_value("with\0nul").is_err());
    assert!(validate_credential_value(&"x".repeat(MAX_MCP_SECRET_BYTES + 1)).is_err());

    // Anything else is kept exactly as written, whitespace and all.
    validate_credential_value("  sk-token with spaces  ").unwrap();
}

#[test]
fn a_server_secret_key_is_a_valid_credential_key() {
    let server = McpServerId::parse("github").unwrap();
    let key = |name: &str| McpCredentialKey::for_server_secret(&server, name);

    assert_eq!(key("GITHUB_TOKEN").as_str(), "github-GITHUB_TOKEN");
    assert_eq!(key("X-Api-Key").as_str(), "github-X-Api-Key");
    assert_eq!(key("Ac\u{e7}ent!").as_str(), "github-Ac_ent_");
    let long = key(&"N".repeat(300));
    assert_eq!(long.as_str().len(), 128);
    for name in ["", "GITHUB_TOKEN", "a b", "\u{e7}\u{e7}", &"N".repeat(300)] {
        let key = key(name);
        assert_eq!(McpCredentialKey::parse(key.as_str()).unwrap(), key);
    }
    let longest = McpServerId::parse("s".repeat(128)).unwrap();
    let key = McpCredentialKey::for_server_secret(&longest, "TOKEN");
    assert_eq!(McpCredentialKey::parse(key.as_str()).unwrap(), key);
}
