use serde_json::json;

use super::*;

#[test]
fn an_entry_without_a_usable_id_is_left_out() {
    for item in [
        json!({"display_name": "No id"}),
        json!({"id": 7}),
        json!({"id": null}),
        json!({"id": "  "}),
        json!({"id": "m".repeat(MAX_MODEL_ID_BYTES + 1)}),
        json!("model-a"),
        json!(null),
    ] {
        assert_eq!(decode_entry(&item), None, "{item}");
    }
    let longest = "m".repeat(MAX_MODEL_ID_BYTES);
    assert_eq!(decode_entry(&json!({"id": longest})).unwrap().id, longest);
}

#[test]
fn a_field_that_cannot_be_read_is_unknown() {
    let entry = decode_entry(&json!({
        "id": "model-a",
        "display_name": 3,
        "default_reasoning_level": "none",
        "supported_reasoning_levels": "low",
        "context_length": "large",
        "max_output_tokens": -1,
        "vision": "yes",
        "reasoning": {"mandatory": false},
        "unknown": {"nested": true},
    }))
    .unwrap();
    assert_eq!(
        entry,
        ModelCatalogEntry {
            id: "model-a".into(),
            display_name: None,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            context_window: None,
            max_output_tokens: None,
            image_input: None,
            reasoning: None,
        }
    );
}

#[test]
fn an_effort_that_does_not_parse_is_left_out_and_the_rest_kept() {
    let entry = decode_entry(&json!({
        "id": "model-a",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            {"effort": "none"},
            {"effort": "low"},
            {"effort": "not an effort"},
            {"description": "no effort"},
            "high",
            {"effort": "low"},
            {"effort": "future-tier"},
        ],
    }))
    .unwrap();
    assert_eq!(
        entry.default_reasoning_effort,
        Some(ModelReasoningEffort::Medium)
    );
    assert_eq!(
        entry
            .supported_reasoning_efforts
            .iter()
            .map(ModelReasoningEffort::as_str)
            .collect::<Vec<_>>(),
        ["low", "future-tier"]
    );
    assert_eq!(entry.reasoning, Some(true));
}

#[test]
fn reported_efforts_stop_at_their_bound() {
    let levels = (0..MAX_REASONING_EFFORTS_PER_MODEL + 8)
        .map(|index| json!({"effort": format!("tier-{index}")}))
        .collect::<Vec<_>>();
    let entry =
        decode_entry(&json!({"id": "model-a", "supported_reasoning_levels": levels})).unwrap();
    assert_eq!(
        entry.supported_reasoning_efforts.len(),
        MAX_REASONING_EFFORTS_PER_MODEL
    );
    assert_eq!(entry.supported_reasoning_efforts[0].as_str(), "tier-0");
}

#[test]
fn a_long_display_name_is_cut_on_a_character_boundary() {
    let name = format!("{}é", "n".repeat(MAX_MODEL_ID_BYTES - 1));
    let entry = decode_entry(&json!({"id": "model-a", "display_name": name})).unwrap();
    assert_eq!(
        entry.display_name.as_deref(),
        Some("n".repeat(MAX_MODEL_ID_BYTES - 1).as_str())
    );
}
