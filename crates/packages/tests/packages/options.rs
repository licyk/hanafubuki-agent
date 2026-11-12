//! Plugin options: a `configSchema` lowered to the settings vocabulary and
//! the tolerant reading of what the settings hold, each value issue naming
//! its layer and key.

use hanafubuki_agent_foundation::settings::{
    SettingChoice, SettingKind, SettingSpec, SettingValue,
};
use hanafubuki_agent_packages::{lower_config_schema, PluginOptionSchema};
use serde_json::{json, Map, Value};

fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn schema() -> PluginOptionSchema {
    let option = |key: &str, kind, default| SettingSpec {
        default: Some(default),
        ..SettingSpec::new(key, key, kind)
    };
    PluginOptionSchema {
        options: vec![
            option("on", SettingKind::Boolean, SettingValue::Boolean(true)),
            option(
                "count",
                SettingKind::Integer {
                    minimum: Some(1),
                    maximum: Some(16),
                },
                SettingValue::Integer(3),
            ),
            option(
                "ratio",
                SettingKind::Number {
                    minimum: Some(0.0),
                    maximum: Some(1.0),
                },
                SettingValue::Number(0.5),
            ),
            option(
                "name",
                SettingKind::Text {
                    maximum_chars: Some(4),
                    secret: false,
                    nullable: false,
                },
                SettingValue::Text("ab".into()),
            ),
            option(
                "mode",
                SettingKind::Choice {
                    values: vec![
                        SettingChoice {
                            aliases: vec!["quick".into()],
                            ..SettingChoice::new("fast", "Fast")
                        },
                        SettingChoice::new("slow", "Slow"),
                    ],
                },
                SettingValue::Text("fast".into()),
            ),
            option(
                "tags",
                SettingKind::TextList {
                    maximum_items: Some(2),
                    maximum_item_chars: Some(3),
                },
                SettingValue::TextList(Vec::new()),
            ),
        ],
    }
}

fn resolve(set: Value) -> hanafubuki_agent_packages::ResolvedPluginOptions {
    let set = object(set);
    schema().resolve("pluginOptions.row", |key| {
        Some((set.get(key)?.clone(), "layer".to_string()))
    })
}

#[test]
fn a_value_of_the_wrong_kind_falls_back_to_the_default_with_an_issue_naming_layer_and_key() {
    let resolved = resolve(json!({
        "on": "yes",
        "count": "three",
        "ratio": "half",
        "name": 7,
        "mode": "medium",
        "tags": "a",
    }));
    assert_eq!(
        Value::Object(resolved.to_object()),
        json!({ "on": true, "count": 3, "ratio": 0.5, "name": "ab", "mode": "fast", "tags": [] })
    );
    assert!(resolved.values.iter().all(|option| option.layer.is_none()));
    assert_eq!(resolved.issues.len(), 6, "{:?}", resolved.issues);
    for key in ["on", "count", "ratio", "name", "mode", "tags"] {
        let named = format!("layer: pluginOptions.row.{key} ");
        assert!(
            resolved
                .issues
                .iter()
                .any(|issue| issue.starts_with(&named)),
            "{key}: {:?}",
            resolved.issues
        );
    }
}

#[test]
fn numbers_out_of_range_are_clamped_with_a_note_and_texts_too_long_fall_back() {
    let resolved = resolve(json!({
        "count": 40,
        "ratio": -2,
        "name": "abcdef",
        "tags": ["a", "b", "c"],
    }));
    assert_eq!(resolved.value("count"), Some(&SettingValue::Integer(16)));
    assert_eq!(resolved.value("ratio"), Some(&SettingValue::Number(0.0)));
    assert_eq!(
        resolved.value("name"),
        Some(&SettingValue::Text("ab".into()))
    );
    assert_eq!(
        resolved.value("tags"),
        Some(&SettingValue::TextList(Vec::new()))
    );
    let count = &resolved.values[1];
    assert_eq!(count.layer.as_deref(), Some("layer"));
    assert!(
        resolved.issues[0].contains("count is 40, which is not between 1 and 16; 16 is used"),
        "{:?}",
        resolved.issues
    );
}

#[test]
fn values_that_fit_are_used_with_their_layer() {
    let resolved = resolve(json!({ "count": 4.0, "mode": "slow", "tags": ["abc"] }));
    assert!(resolved.issues.is_empty(), "{:?}", resolved.issues);
    assert_eq!(resolved.value("count"), Some(&SettingValue::Integer(4)));
    assert_eq!(
        resolved.value("mode"),
        Some(&SettingValue::Text("slow".into()))
    );
}

#[test]
fn a_former_name_of_a_choice_is_read_as_its_value_and_the_guest_receives_that() {
    let resolved = resolve(json!({ "mode": "quick" }));
    assert!(resolved.issues.is_empty(), "{:?}", resolved.issues);
    assert_eq!(
        resolved.value("mode"),
        Some(&SettingValue::Text("fast".into()))
    );
    let (checked, _) = schema().check_settings("settings", &object(json!({ "mode": "quick" })));
    assert_eq!(Value::Object(checked), json!({ "mode": "fast" }));
}

#[test]
fn a_config_schema_is_lowered_with_its_supported_keywords() {
    let (lowered, issues) = lower_config_schema(&object(json!({
        "type": "object",
        "properties": {
            "greeting": { "type": "string", "maxLength": 20, "description": "What it says\nmore", "default": "hi" },
            "level": { "type": "integer", "minimum": 1, "maximum": 5 },
            "limit": { "type": "integer", "minimum": 0 },
            "style": { "enum": ["plain", "fancy"] },
            "tags": { "type": "array", "maxItems": 3, "items": { "type": "string", "maxLength": 8 } },
            "verbose": { "type": "boolean", "default": "no" },
            "nested": { "type": "object" },
            "either": { "type": ["string", "null"] },
        }
    })));
    let keys = lowered
        .options
        .iter()
        .map(|option| option.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        ["greeting", "level", "limit", "style", "tags", "verbose"]
    );
    assert_eq!(
        lowered.option("limit").unwrap().kind,
        SettingKind::Integer {
            minimum: Some(0),
            maximum: None
        }
    );
    let greeting = lowered.option("greeting").unwrap();
    assert_eq!(greeting.summary, "What it says");
    assert_eq!(
        greeting.kind,
        SettingKind::Text {
            maximum_chars: Some(20),
            secret: false,
            nullable: false
        }
    );
    assert_eq!(greeting.default, Some(SettingValue::Text("hi".into())));
    assert_eq!(
        lowered.option("level").unwrap().kind,
        SettingKind::Integer {
            minimum: Some(1),
            maximum: Some(5)
        }
    );
    assert_eq!(
        lowered.option("tags").unwrap().kind,
        SettingKind::TextList {
            maximum_items: Some(3),
            maximum_item_chars: Some(8)
        }
    );
    assert_eq!(
        lowered.option("style").unwrap().kind,
        SettingKind::Choice {
            values: vec![
                SettingChoice::new("plain", "plain"),
                SettingChoice::new("fancy", "fancy")
            ]
        }
    );
    assert_eq!(lowered.option("style").unwrap().label, "style");
    assert_eq!(lowered.option("verbose").unwrap().default, None);
    assert_eq!(issues.len(), 3, "{issues:?}");
    assert!(issues
        .iter()
        .any(|issue| issue.contains("`nested` is of type `object`")));
    assert!(issues
        .iter()
        .any(|issue| issue.contains("`either` has more than one type")));
    assert!(issues
        .iter()
        .any(|issue| issue.contains("`verbose` has a default")));
}

#[test]
fn a_guests_settings_are_checked_and_what_the_schema_does_not_describe_is_kept() {
    let (lowered, _) = lower_config_schema(&object(json!({
        "properties": {
            "level": { "type": "integer", "minimum": 1, "maximum": 5, "default": 2 },
            "name": { "type": "string" },
            "nested": { "type": "object" },
        }
    })));
    let (checked, resolved) = lowered.check_settings(
        "plugins.\"demo@local\".settings",
        &object(json!({ "level": "high", "name": 3, "nested": { "a": 1 }, "extra": true })),
    );
    assert_eq!(
        Value::Object(checked),
        json!({ "level": 2, "nested": { "a": 1 }, "extra": true })
    );
    assert_eq!(resolved.issues.len(), 2, "{:?}", resolved.issues);
    assert!(resolved.issues[0].starts_with("config.toml: plugins.\"demo@local\".settings.level"));

    let (unset, resolved) = lowered.check_settings("settings", &Map::new());
    assert!(unset.is_empty(), "a default is not added: {unset:?}");
    assert_eq!(resolved.value("level"), Some(&SettingValue::Integer(2)));
}
