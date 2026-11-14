//! `ModelMetadata` as it will be persisted: a round trip, and a reader that
//! keeps every fact it can use; and what every surface reads of it: the
//! levels, whether the model reasons and reads images, each source's label.

use hanafubuki_agent_model_api::{
    KnownReasoning, ModelMetadata, ModelMetadataSource, ModelReasoningEffort,
    ModelReasoningSupport, ModelStructuredOutputCapability, Sourced,
};
use serde_json::json;

#[test]
fn metadata_round_trips_with_each_fact_and_its_source() {
    let metadata = ModelMetadata {
        context_window: Some(Sourced {
            value: 200_000,
            source: ModelMetadataSource::Reported,
            observed_at_ms: 1_700_000_000_000,
        }),
        max_output_tokens: None,
        reasoning: Some(Sourced {
            value: ModelReasoningSupport::Supported {
                efforts: vec![ModelReasoningEffort::Low, ModelReasoningEffort::High],
                default: Some(ModelReasoningEffort::High),
            },
            source: ModelMetadataSource::BuiltIn,
            observed_at_ms: 1,
        }),
        image_input: Some(Sourced {
            value: false,
            source: ModelMetadataSource::Learned,
            observed_at_ms: 2,
        }),
        structured_output: Some(Sourced {
            value: ModelStructuredOutputCapability::RequiredTool,
            source: ModelMetadataSource::Learned,
            observed_at_ms: 3,
        }),
    };
    let encoded = serde_json::to_value(&metadata).unwrap();
    assert_eq!(
        encoded,
        json!({
            "contextWindow": {"value": 200_000, "source": "reported", "observedAtMs": 1_700_000_000_000_u64},
            "maxOutputTokens": null,
            "reasoning": {
                "value": {"type": "supported", "efforts": ["low", "high"], "default": "high"},
                "source": "built_in",
                "observedAtMs": 1,
            },
            "imageInput": {"value": false, "source": "learned", "observedAtMs": 2},
            "structuredOutput": {"value": "required_tool", "source": "learned", "observedAtMs": 3},
        })
    );
    assert_eq!(
        serde_json::from_value::<ModelMetadata>(encoded).unwrap(),
        metadata
    );
}

#[test]
fn a_fact_that_does_not_decode_is_unknown_and_the_others_are_kept() {
    let decoded = serde_json::from_value::<ModelMetadata>(json!({
        "contextWindow": {"value": "large", "source": "reported"},
        "maxOutputTokens": {"value": 64_000, "source": "a_future_source"},
        "reasoning": {
            "value": {"type": "supported", "efforts": ["low", "none", 3], "default": "none"},
            "source": "configured",
            "note": "an unknown field",
        },
        "imageInput": {"value": true, "source": "catalog"},
        "structuredOutput": {"value": "a_future_rung", "source": "learned"},
        "toolCalls": {"value": true, "source": "reported"},
    }))
    .unwrap();
    assert_eq!(
        decoded,
        ModelMetadata {
            context_window: None,
            max_output_tokens: None,
            reasoning: Some(Sourced {
                value: ModelReasoningSupport::Supported {
                    efforts: vec![ModelReasoningEffort::Low],
                    default: None,
                },
                source: ModelMetadataSource::Configured,
                observed_at_ms: 0,
            }),
            image_input: Some(Sourced {
                value: true,
                source: ModelMetadataSource::Catalog,
                observed_at_ms: 0,
            }),
            structured_output: None,
        }
    );
    assert_eq!(
        serde_json::from_value::<ModelMetadata>(json!({})).unwrap(),
        ModelMetadata::default()
    );
}

fn reasoning(value: ModelReasoningSupport, source: ModelMetadataSource) -> ModelMetadata {
    ModelMetadata {
        reasoning: Some(Sourced {
            value,
            source,
            observed_at_ms: 0,
        }),
        ..ModelMetadata::default()
    }
}

fn levels(efforts: &[ModelReasoningEffort]) -> ModelReasoningSupport {
    ModelReasoningSupport::Supported {
        efforts: efforts.to_vec(),
        default: Some(ModelReasoningEffort::Medium),
    }
}

#[test]
fn the_known_levels_are_the_configured_or_reported_ones_with_their_default() {
    let efforts = [ModelReasoningEffort::Low, ModelReasoningEffort::Medium];
    for source in [
        ModelMetadataSource::Configured,
        ModelMetadataSource::Reported,
    ] {
        assert_eq!(
            reasoning(levels(&efforts), source).known_reasoning(),
            KnownReasoning::Levels {
                efforts: efforts.to_vec(),
                default: Some(ModelReasoningEffort::Medium),
            },
            "{source:?}"
        );
    }
}

#[test]
fn levels_from_models_dev_or_the_built_in_table_or_not_reported_are_unknown() {
    let efforts = [ModelReasoningEffort::High];
    for source in [ModelMetadataSource::Catalog, ModelMetadataSource::BuiltIn] {
        assert_eq!(
            reasoning(levels(&efforts), source).known_reasoning(),
            KnownReasoning::Unknown,
            "{source:?}"
        );
    }
    assert_eq!(
        reasoning(levels(&[]), ModelMetadataSource::Reported).known_reasoning(),
        KnownReasoning::Unknown
    );
    assert_eq!(
        ModelMetadata::default().known_reasoning(),
        KnownReasoning::Unknown
    );
}

#[test]
fn no_reasoning_is_known_when_configured_reported_or_learned() {
    for (source, learned) in [
        (ModelMetadataSource::Configured, false),
        (ModelMetadataSource::Reported, false),
        (ModelMetadataSource::Learned, true),
    ] {
        assert_eq!(
            reasoning(ModelReasoningSupport::Unsupported, source).known_reasoning(),
            KnownReasoning::Unsupported { learned },
            "{source:?}"
        );
    }
    assert_eq!(
        reasoning(
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Catalog
        )
        .known_reasoning(),
        KnownReasoning::Unknown
    );
}

const SOURCES: [ModelMetadataSource; 5] = [
    ModelMetadataSource::Configured,
    ModelMetadataSource::Learned,
    ModelMetadataSource::Reported,
    ModelMetadataSource::Catalog,
    ModelMetadataSource::BuiltIn,
];

#[test]
fn each_source_has_one_label() {
    assert_eq!(
        SOURCES.map(ModelMetadataSource::label),
        [
            "configured",
            "learned",
            "reported",
            "models.dev",
            "built-in"
        ]
    );
}

#[test]
fn a_model_reasons_whenever_a_fact_says_so_levels_known_or_not() {
    for source in SOURCES {
        for support in [levels(&[ModelReasoningEffort::High]), levels(&[])] {
            assert_eq!(
                reasoning(support, source).supports_reasoning(),
                Some(true),
                "{source:?}"
            );
        }
    }
    // models.dev says it reasons and names no level: thinking is a yes,
    // the levels stay unknown.
    let catalog = reasoning(
        ModelReasoningSupport::Supported {
            efforts: Vec::new(),
            default: None,
        },
        ModelMetadataSource::Catalog,
    );
    assert_eq!(catalog.supports_reasoning(), Some(true));
    assert_eq!(catalog.known_reasoning(), KnownReasoning::Unknown);
    assert_eq!(ModelMetadata::default().supports_reasoning(), None);
}

#[test]
fn a_model_takes_no_reasoning_when_configured_reported_or_learned() {
    for (source, says) in [
        (ModelMetadataSource::Configured, Some(false)),
        (ModelMetadataSource::Learned, Some(false)),
        (ModelMetadataSource::Reported, Some(false)),
        (ModelMetadataSource::Catalog, None),
        (ModelMetadataSource::BuiltIn, None),
    ] {
        assert_eq!(
            reasoning(ModelReasoningSupport::Unsupported, source).supports_reasoning(),
            says,
            "{source:?}"
        );
    }
}

#[test]
fn image_input_is_the_facts_value_whatever_its_source() {
    for source in SOURCES {
        for value in [true, false] {
            let metadata = ModelMetadata {
                image_input: Some(Sourced {
                    value,
                    source,
                    observed_at_ms: 0,
                }),
                ..ModelMetadata::default()
            };
            assert_eq!(metadata.supports_image_input(), Some(value), "{source:?}");
        }
    }
    assert_eq!(ModelMetadata::default().supports_image_input(), None);
}
