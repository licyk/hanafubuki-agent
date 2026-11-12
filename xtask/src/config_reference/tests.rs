use super::*;

#[test]
fn keys_are_the_literals_spec_new_is_given() {
    let source = r#"
fn section() -> SettingsSection {
    let kind = SettingKind::Boolean;
    vec![
        described(SettingSpec::new("memory.enabled", "Memory", kind), "On"),
        SettingSpec { advanced: true, ..settings::SettingSpec::new("logLevel", "Log", kind) },
        SettingSpec::new(format!("memory.{key}"), "Built", kind),
    ]
}
"#;
    let keys: Vec<_> = spec_keys(source)
        .unwrap()
        .into_iter()
        .map(|(_, key)| key)
        .collect();
    assert_eq!(
        keys,
        [
            Some("memory.enabled".to_string()),
            Some("logLevel".to_string()),
            None
        ]
    );
}

#[test]
fn rows_are_read_from_the_config_toml_section_alone() {
    let text = "\
# Reference

## config.toml [#config-toml]

### Memory [#memory]

| Key              | Values  |
| ---------------- | ------- |
| `memory.enabled` | boolean |

Text with `inline` code.

| `tui.keymap.CONTEXT.ACTION` | keys |

## providers.toml [#providers-toml]

| `enabled` | boolean |
";
    assert_eq!(
        section_rows(text),
        ["memory.enabled", "tui.keymap.CONTEXT.ACTION"]
            .map(str::to_string)
            .into()
    );
}
