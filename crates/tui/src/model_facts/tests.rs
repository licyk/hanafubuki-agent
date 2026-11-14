use hanafubuki_agent::{
    KnownReasoning, ModelMetadata, ModelMetadataSource, ModelReasoningEffort,
    ModelReasoningSupport, SettingChoice, Sourced,
};

use super::{effort_options, entry_badges, step_effort, EffortStep, EffortValue};

fn sourced<T>(value: T, source: ModelMetadataSource) -> Option<Sourced<T>> {
    Some(Sourced {
        value,
        source,
        observed_at_ms: 0,
    })
}

#[test]
fn a_model_shows_what_is_known_of_it_whatever_the_source() {
    assert_eq!(entry_badges(&ModelMetadata::default()), None);
    let known = ModelMetadata {
        context_window: sourced(200_000, ModelMetadataSource::Reported),
        image_input: sourced(true, ModelMetadataSource::Configured),
        reasoning: sourced(
            ModelReasoningSupport::Supported {
                efforts: Vec::new(),
                default: None,
            },
            ModelMetadataSource::Catalog,
        ),
        ..ModelMetadata::default()
    };
    assert_eq!(
        entry_badges(&known).as_deref(),
        Some("200K · vision · reasoning")
    );
    // An OpenAI model reports no window; the built-in table knows it.
    let built_in = ModelMetadata {
        context_window: sourced(272_000, ModelMetadataSource::BuiltIn),
        ..ModelMetadata::default()
    };
    assert_eq!(entry_badges(&built_in).as_deref(), Some("272K"));
    let says_no = ModelMetadata {
        image_input: sourced(false, ModelMetadataSource::Learned),
        reasoning: sourced(
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Reported,
        ),
        ..ModelMetadata::default()
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

fn levels(efforts: &[&str], default: Option<&str>) -> KnownReasoning {
    let effort = |name: &str| ModelReasoningEffort::parse(name).unwrap();
    KnownReasoning::Levels {
        efforts: efforts.iter().map(|name| effort(name)).collect(),
        default: default.map(effort),
    }
}

fn to(effort: &str) -> EffortStep {
    EffortStep::To(ModelReasoningEffort::parse(effort).unwrap())
}

#[test]
fn a_step_moves_one_level_from_the_effort_set() {
    let known = levels(&["low", "medium", "high"], Some("medium"));
    assert_eq!(step_effort(&known, Some("low"), true), to("medium"));
    assert_eq!(step_effort(&known, Some("high"), false), to("medium"));
}

#[test]
fn an_unset_effort_steps_from_the_model_default_else_the_first_level() {
    let known = levels(&["low", "medium", "high"], Some("medium"));
    assert_eq!(step_effort(&known, None, true), to("high"));
    assert_eq!(step_effort(&known, None, false), to("low"));
    // An effort off the levels counts as the default too.
    assert_eq!(step_effort(&known, Some("max"), true), to("high"));
    let no_default = levels(&["low", "high"], None);
    assert_eq!(step_effort(&no_default, None, true), to("high"));
}

#[test]
fn a_step_stops_at_the_ends() {
    let known = levels(&["low", "high"], None);
    assert_eq!(
        step_effort(&known, Some("high"), true),
        EffortStep::Stays("Reasoning is already at the highest level (High).".to_string())
    );
    assert_eq!(
        step_effort(&known, None, false),
        EffortStep::Stays("Reasoning is already at the lowest level (Low).".to_string())
    );
}

#[test]
fn with_no_levels_a_step_says_to_type_one() {
    let wanted = EffortStep::Stays(
        "No reasoning levels are known for this model; use /effort to type one.".to_string(),
    );
    for known in [
        KnownReasoning::Unknown,
        KnownReasoning::Unsupported { learned: true },
        levels(&[], None),
    ] {
        assert_eq!(step_effort(&known, Some("high"), true), wanted);
    }
}
