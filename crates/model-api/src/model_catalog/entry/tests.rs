use serde_json::json;

use super::*;

fn decode_entry(item: &Value) -> Option<ModelCatalogEntry> {
    super::decode_entry(ModelApiProtocol::Responses, item)
}

#[test]
fn an_entry_without_a_usable_id_is_left_out() {
    for item in [
        json!({"display_name": "No id"}),
        json!({"id": 7}),
        json!({"id": null}),
        json!({"id": "  "}),
        json!({"id": "m".repeat(MAX_MODEL_ID_BYTES + 1)}),
        json!({"slug": 7}),
        json!({"slug": "  "}),
        json!({"slug": "m".repeat(MAX_MODEL_ID_BYTES + 1)}),
        json!("model-a"),
        json!(null),
    ] {
        assert_eq!(decode_entry(&item), None, "{item}");
    }
    let longest = "m".repeat(MAX_MODEL_ID_BYTES);
    assert_eq!(decode_entry(&json!({"id": longest})).unwrap().id, longest);
    assert_eq!(decode_entry(&json!({"slug": longest})).unwrap().id, longest);
}

#[test]
fn the_slug_names_the_model_when_the_id_does_not() {
    let entry = decode_entry(&json!({"slug": "glm-5.3", "display_name": "glm-5.3"})).unwrap();
    assert_eq!(entry.id, "glm-5.3");
    assert_eq!(entry.display_name.as_deref(), Some("glm-5.3"));
    let both = decode_entry(&json!({"id": "model-a", "slug": "model-b"})).unwrap();
    assert_eq!(both.id, "model-a");
    for item in [
        json!({"id": null, "slug": "model-b"}),
        json!({"id": 7, "slug": "model-b"}),
        json!({"id": " ", "slug": "model-b"}),
    ] {
        assert_eq!(decode_entry(&item).unwrap().id, "model-b", "{item}");
    }
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
            bytes_per_token_millis: None,
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
fn deepseek_and_hoshimi_efforts_are_read_tolerantly() {
    for fields in [
        json!({"effort": {
            "supported_levels": ["max", null, "high", 42, "low", "none", "high", "future-tier"],
            "default_level": "high",
            "future_field": true
        }}),
        json!({
            "efforts": [
                {"id": "max", "name": "Max"}, null, {"id": "high", "is_default": true},
                {"id": 42}, {"id": "low"}, {"id": "none"}, {"id": "high"},
                {"id": "future-tier"}, {"name": "Missing id"}, "low"
            ],
            "effort_default": "high"
        }),
    ] {
        let mut item = json!({"id": "model-a"});
        item.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let entry = decode_entry(&item).unwrap();
        assert_eq!(
            entry.default_reasoning_effort,
            Some(ModelReasoningEffort::High)
        );
        assert_eq!(
            entry
                .supported_reasoning_efforts
                .iter()
                .map(ModelReasoningEffort::as_str)
                .collect::<Vec<_>>(),
            ["low", "high", "max", "future-tier"]
        );
        assert_eq!(entry.reasoning, Some(true));
    }
}

#[test]
fn effort_shapes_use_the_first_usable_list_and_default() {
    let mut item = json!({
        "id": "model-a",
        "supported_reasoning_levels": [{"effort": "none"}],
        "reasoning": {"supported_efforts": false, "default_effort": 42},
        "effort": {"supported_levels": ["high"], "default_level": "high"},
        "efforts": [{"id": "on"}],
        "effort_default": "on"
    });
    let entry = decode_entry(&item).unwrap();
    assert_eq!(
        entry.supported_reasoning_efforts,
        [ModelReasoningEffort::High]
    );
    assert_eq!(
        entry.default_reasoning_effort,
        Some(ModelReasoningEffort::High)
    );

    item["effort"] = json!({"supported_levels": [null, "none"], "default_level": "none"});
    let entry = decode_entry(&item).unwrap();
    assert_eq!(
        entry.supported_reasoning_efforts,
        [ModelReasoningEffort::parse("on").unwrap()]
    );
    assert_eq!(
        entry.default_reasoning_effort,
        Some(ModelReasoningEffort::parse("on").unwrap())
    );

    item["efforts"] = json!("high");
    item["effort_default"] = json!(false);
    let entry = decode_entry(&item).unwrap();
    assert!(entry.supported_reasoning_efforts.is_empty());
    assert_eq!(entry.default_reasoning_effort, None);
    assert_eq!(entry.reasoning, None);
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
