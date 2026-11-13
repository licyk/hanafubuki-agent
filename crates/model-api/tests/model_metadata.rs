//! `ModelMetadata` as it will be persisted: a round trip, and a reader that
//! keeps every fact it can use.

use hanafubuki_agent_model_api::{
    ModelMetadata, ModelMetadataSource, ModelReasoningEffort, ModelReasoningSupport,
    ModelStructuredOutputCapability, Sourced,
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
