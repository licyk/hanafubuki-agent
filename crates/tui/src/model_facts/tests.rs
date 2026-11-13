use hanafubuki_agent::{ModelCatalogEntry, SettingChoice};

use super::{effort_options, entry_badges, EffortValue};

fn entry() -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: "model".to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window: None,
        max_output_tokens: None,
        image_input: None,
        reasoning: None,
    }
}

#[test]
fn an_entry_shows_only_what_it_reports() {
    assert_eq!(entry_badges(&entry()), None);
    let reported = ModelCatalogEntry {
        context_window: Some(200_000),
        image_input: Some(true),
        reasoning: Some(true),
        ..entry()
    };
    assert_eq!(
        entry_badges(&reported).as_deref(),
        Some("200K · vision · reasoning")
    );
    let says_no = ModelCatalogEntry {
        image_input: Some(false),
        reasoning: Some(false),
        ..entry()
    };
    assert_eq!(entry_badges(&says_no), None);
}

fn options(choices: &[SettingChoice], current: Option<&str>) -> Vec<(String, bool)> {
    effort_options(choices, current)
        .into_iter()
        .map(|option| (option.label, option.current))
        .collect()
}

fn labelled(options: &[(&str, bool)]) -> Vec<(String, bool)> {
    options
        .iter()
        .map(|(label, current)| (label.to_string(), *current))
        .collect()
}

#[test]
fn the_effort_options_are_the_levels_then_default_and_custom() {
    let levels = [
        SettingChoice::new("low", "low"),
        SettingChoice::new("medium", "medium (model default)"),
    ];
    assert_eq!(
        options(&levels, Some("low")),
        labelled(&[
            ("low", true),
            ("medium (model default)", false),
            ("Default", false),
            ("Custom…", false),
        ])
    );
    assert_eq!(
        options(&levels, None),
        labelled(&[
            ("low", false),
            ("medium (model default)", false),
            ("Default", true),
            ("Custom…", false),
        ])
    );
}

#[test]
fn with_no_levels_only_default_and_custom_are_offered() {
    assert_eq!(
        options(&[], None),
        labelled(&[("Default", true), ("Custom…", false)])
    );
}

#[test]
fn a_current_effort_off_the_levels_is_the_custom_one() {
    let levels = [SettingChoice::new("high", "high")];
    let offered = effort_options(&levels, Some("max"));
    assert_eq!(
        offered
            .iter()
            .map(|option| (option.label.clone(), option.current))
            .collect::<Vec<_>>(),
        labelled(&[("high", false), ("Default", false), ("Custom: max", true)])
    );
    assert_eq!(offered[2].value, EffortValue::Custom);
    assert_eq!(offered[0].value, EffortValue::Level("high".to_string()));
    assert_eq!(offered[1].value, EffortValue::Default);
}
