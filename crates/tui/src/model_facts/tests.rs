use hanafubuki_agent::{
    KnownReasoning, ModelMetadata, ModelMetadataSource, ModelReasoningEffort,
    ModelReasoningSupport, SettingChoice, Sourced,
};

use super::{
    choose_hint, effort_options, fact_cells, fact_columns, fact_details, fact_line, row_facts,
    step_effort, EffortStep, EffortValue, FACTS_HEADER,
};

fn sourced<T>(value: T, source: ModelMetadataSource) -> Option<Sourced<T>> {
    Some(Sourced {
        value,
        source,
        observed_at_ms: 0,
    })
}

fn reasons(
    efforts: &[ModelReasoningEffort],
    default: Option<ModelReasoningEffort>,
    source: ModelMetadataSource,
) -> Option<Sourced<ModelReasoningSupport>> {
    sourced(
        ModelReasoningSupport::Supported {
            efforts: efforts.to_vec(),
            default,
        },
        source,
    )
}

fn takes_none(source: ModelMetadataSource) -> Option<Sourced<ModelReasoningSupport>> {
    sourced(ModelReasoningSupport::Unsupported, source)
}

/// The cells of `metadata`: window, thinking, vision.
fn cells(metadata: &ModelMetadata) -> (String, &'static str, &'static str) {
    let cells = fact_cells(metadata);
    (cells.window, cells.thinking, cells.vision)
}

#[test]
fn a_cell_says_yes_no_or_not_known_whatever_the_source() {
    assert_eq!(
        cells(&ModelMetadata::default()),
        ("?".to_string(), "?", "?")
    );
    let known = ModelMetadata {
        context_window: sourced(1_047_576, ModelMetadataSource::Reported),
        reasoning: reasons(&[], None, ModelMetadataSource::Catalog),
        image_input: sourced(true, ModelMetadataSource::Configured),
        ..ModelMetadata::default()
    };
    assert_eq!(cells(&known), ("1.05M".to_string(), "yes", "yes"));
    for source in [
        ModelMetadataSource::Configured,
        ModelMetadataSource::Reported,
        ModelMetadataSource::Learned,
    ] {
        let no = ModelMetadata {
            context_window: sourced(272_000, ModelMetadataSource::BuiltIn),
            reasoning: takes_none(source),
            image_input: sourced(false, source),
            ..ModelMetadata::default()
        };
        assert_eq!(cells(&no), ("272K".to_string(), "no", "no"), "{source:?}");
    }
    // A guess's "takes none" says nothing.
    let guessed = ModelMetadata {
        reasoning: takes_none(ModelMetadataSource::Catalog),
        ..ModelMetadata::default()
    };
    assert_eq!(cells(&guessed).1, "?");
}

#[test]
fn the_cells_fit_their_header_in_columns_and_name_themselves_on_one_line() {
    let metadata = ModelMetadata {
        context_window: sourced(200_000, ModelMetadataSource::Reported),
        reasoning: reasons(&[], None, ModelMetadataSource::Reported),
        image_input: sourced(false, ModelMetadataSource::Learned),
        ..ModelMetadata::default()
    };
    let cells = fact_cells(&metadata);
    assert_eq!(fact_columns(&cells), "   200K  yes       no    ");
    assert_eq!(fact_columns(&cells).len(), FACTS_HEADER.len());
    assert_eq!(fact_line(&cells), "context 200K · thinking yes · vision no");
    let row = row_facts(&metadata);
    assert_eq!(row.columns, fact_columns(&cells));
    assert_eq!(row.stacked, fact_line(&cells));
}

#[test]
fn the_detail_line_names_each_fact_with_its_source() {
    let details = |metadata: &ModelMetadata| fact_details("m", "P", metadata);
    assert_eq!(
        details(&ModelMetadata::default()),
        "m · P: context not known (compacts on the Provider's error) · thinking not known · vision not known"
    );
    let reported = ModelMetadata {
        context_window: sourced(200_000, ModelMetadataSource::Reported),
        max_output_tokens: sourced(64_000, ModelMetadataSource::Reported),
        reasoning: reasons(
            &[
                ModelReasoningEffort::Low,
                ModelReasoningEffort::Medium,
                ModelReasoningEffort::High,
            ],
            Some(ModelReasoningEffort::Medium),
            ModelMetadataSource::Reported,
        ),
        image_input: sourced(true, ModelMetadataSource::Reported),
        ..ModelMetadata::default()
    };
    assert_eq!(
        details(&reported),
        "m · P: context 200K (reported) · max output 64K (reported) · thinking low, medium, high, default medium (reported) · vision yes (reported)"
    );
    let configured_levels = ModelMetadata {
        reasoning: reasons(
            &[ModelReasoningEffort::Low, ModelReasoningEffort::High],
            None,
            ModelMetadataSource::Configured,
        ),
        ..ModelMetadata::default()
    };
    assert!(details(&configured_levels).contains("thinking low, high (configured)"));
    let guessed = ModelMetadata {
        context_window: sourced(272_000, ModelMetadataSource::BuiltIn),
        reasoning: reasons(&[], None, ModelMetadataSource::Catalog),
        image_input: sourced(true, ModelMetadataSource::Catalog),
        ..ModelMetadata::default()
    };
    assert_eq!(
        details(&guessed),
        "m · P: context 272K (built-in) · thinking yes, levels not known (models.dev) · vision yes (models.dev)"
    );
    let configured_none = ModelMetadata {
        reasoning: takes_none(ModelMetadataSource::Configured),
        ..ModelMetadata::default()
    };
    assert!(details(&configured_none).contains("thinking no (configured)"));
    let learned = ModelMetadata {
        reasoning: takes_none(ModelMetadataSource::Learned),
        image_input: sourced(false, ModelMetadataSource::Learned),
        ..ModelMetadata::default()
    };
    assert!(details(&learned)
        .ends_with("thinking no (learned: the Provider refused it) · vision no (learned)"));
    let guessed_none = ModelMetadata {
        reasoning: takes_none(ModelMetadataSource::BuiltIn),
        ..ModelMetadata::default()
    };
    assert!(details(&guessed_none).contains("thinking not known"));
}

#[test]
fn the_keys_say_what_esc_does_and_name_models_dev_only_while_it_is_off_and_needed() {
    let known = ModelMetadata {
        context_window: sourced(200_000, ModelMetadataSource::Reported),
        reasoning: takes_none(ModelMetadataSource::Reported),
        image_input: sourced(true, ModelMetadataSource::Reported),
        ..ModelMetadata::default()
    };
    assert!(!fact_cells(&known).has_unknown());
    for unknown in [
        ModelMetadata {
            context_window: None,
            ..known.clone()
        },
        ModelMetadata {
            reasoning: None,
            ..known.clone()
        },
        ModelMetadata {
            image_input: None,
            ..known.clone()
        },
    ] {
        assert!(fact_cells(&unknown).has_unknown(), "{unknown:?}");
    }
    assert_eq!(
        choose_hint("esc back", false, false),
        "enter choose · esc back · ? not known"
    );
    assert_eq!(
        choose_hint("esc close", true, false),
        "enter choose · esc close · ? not known · models.dev can fill these: /config modelMetadata"
    );
    assert_eq!(
        choose_hint("esc close", true, true),
        "enter choose · esc close · ? not known"
    );
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
