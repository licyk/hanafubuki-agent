//! The settings vocabulary: a file's value read tolerantly, a write checked
//! strictly.

use hanafubuki_agent_foundation::settings::{
    SettingChoice, SettingKind, SettingProblem, SettingSpec, SettingValue,
};
use serde_json::json;

fn integer(minimum: Option<i64>, maximum: Option<i64>) -> SettingSpec {
    SettingSpec::new("count", "Count", SettingKind::Integer { minimum, maximum })
}

fn mode() -> SettingSpec {
    SettingSpec::new(
        "mode",
        "Mode",
        SettingKind::Choice {
            values: vec![
                SettingChoice {
                    aliases: vec!["quick".into()],
                    ..SettingChoice::new("fast", "Fast")
                },
                SettingChoice::new("slow", "Slow"),
                SettingChoice {
                    disabled: Some("needs a key".into()),
                    ..SettingChoice::new("remote", "Remote")
                },
            ],
        },
    )
}

#[test]
fn a_value_out_of_range_is_read_with_its_clamped_value_and_refused_on_write() {
    let count = integer(Some(1), Some(16));
    assert_eq!(count.read(&json!(4.0)), Ok(SettingValue::Integer(4)));
    assert_eq!(
        count.read(&json!(40)),
        Err(SettingProblem::OutOfRange {
            clamped: SettingValue::Integer(16),
            bounds: "between 1 and 16".into()
        })
    );
    assert_eq!(count.check(&json!(4)), Ok(SettingValue::Integer(4)));
    assert_eq!(
        count.check(&json!(17)).unwrap_err(),
        "`count` is between 1 and 16"
    );
    assert!(count
        .check(&json!("4"))
        .unwrap_err()
        .contains("not an integer"));
}

#[test]
fn a_bound_left_out_bounds_nothing_on_its_side() {
    let count = integer(Some(0), None);
    assert_eq!(
        count.check(&json!(1_000_000)),
        Ok(SettingValue::Integer(1_000_000))
    );
    assert_eq!(
        count.read(&json!(u64::MAX)),
        Err(SettingProblem::OutOfRange {
            clamped: SettingValue::Integer(i64::MAX),
            bounds: format!("between 0 and {}", i64::MAX)
        })
    );
    assert_eq!(
        integer(None, None).check(&json!(u64::MAX)).unwrap_err(),
        format!("`count` is at most {}", i64::MAX)
    );
    assert_eq!(
        count.check(&json!(-1)).unwrap_err(),
        "`count` is at least 0"
    );
    assert_eq!(
        integer(None, None).check(&json!(i64::MIN)),
        Ok(SettingValue::Integer(i64::MIN))
    );
    let ratio = SettingSpec::new(
        "ratio",
        "Ratio",
        SettingKind::Number {
            minimum: None,
            maximum: Some(1.0),
        },
    );
    assert_eq!(ratio.check(&json!(-5.5)), Ok(SettingValue::Number(-5.5)));
    assert_eq!(ratio.check(&json!(2)).unwrap_err(), "`ratio` is at most 1");
}

#[test]
fn a_former_name_is_read_as_its_choice_but_never_written() {
    let mode = mode();
    assert_eq!(
        mode.read(&json!("quick")),
        Ok(SettingValue::Text("fast".into()))
    );
    let refused = mode.check(&json!("quick")).unwrap_err();
    assert!(refused.contains("former name of \"fast\""), "{refused}");
    assert!(mode
        .check(&json!("medium"))
        .unwrap_err()
        .contains("fast, slow, remote"));
}

#[test]
fn a_disabled_choice_set_stays_readable_and_a_write_is_refused_with_its_reason() {
    let mode = mode();
    assert_eq!(
        mode.read(&json!("remote")),
        Ok(SettingValue::Text("remote".into()))
    );
    assert_eq!(
        mode.check(&json!("remote")).unwrap_err(),
        "`mode` cannot be remote: needs a key"
    );
    assert_eq!(
        mode.check(&json!("slow")),
        Ok(SettingValue::Text("slow".into()))
    );
}

#[test]
fn null_is_a_value_only_of_a_nullable_text() {
    let text = |nullable| {
        SettingSpec::new(
            "url",
            "URL",
            SettingKind::Text {
                maximum_chars: Some(3),
                secret: false,
                nullable,
            },
        )
    };
    assert_eq!(text(true).check(&json!(null)), Ok(SettingValue::Null));
    assert_eq!(SettingValue::Null.to_json(), json!(null));
    assert!(text(false)
        .check(&json!(null))
        .unwrap_err()
        .contains("not a string"));
    assert!(text(true)
        .check(&json!("abcd"))
        .unwrap_err()
        .contains("longer than 3 characters"));
}

#[test]
fn a_path_that_must_be_absolute_refuses_a_relative_one() {
    let path = |must_be_absolute| {
        SettingSpec::new("dir", "Directory", SettingKind::Path { must_be_absolute })
    };
    let absolute = std::env::temp_dir();
    let absolute = absolute.to_str().unwrap();
    assert_eq!(
        path(true).check(&json!(absolute)),
        Ok(SettingValue::Text(absolute.to_string()))
    );
    assert!(path(true)
        .check(&json!("relative/dir"))
        .unwrap_err()
        .contains("not an absolute path"));
    assert_eq!(
        path(false).check(&json!("relative/dir")),
        Ok(SettingValue::Text("relative/dir".into()))
    );
}

#[test]
fn a_spec_serializes_its_kind_tagged_and_camel_case() {
    let spec = SettingSpec {
        summary: "Items".into(),
        ..SettingSpec::new(
            "tags",
            "Tags",
            SettingKind::TextList {
                maximum_items: Some(2),
                maximum_item_chars: None,
            },
        )
    };
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(
        json["kind"],
        json!({ "type": "textList", "maximumItems": 2, "maximumItemChars": null })
    );
    assert_eq!(json["apply"], json!("live"));
    assert_eq!(json["visibleIf"], json!(null));
}
