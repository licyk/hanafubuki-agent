use hanafubuki_agent_foundation::json_normalization::{
    normalize_current_json, JsonField, JsonNormalizationEvent, JsonNormalizationEventKind,
    JsonShape, JsonVariant, MAX_JSON_NORMALIZATION_POINTER_BYTES, OPAQUE_JSON_SHAPE,
};
use serde_json::json;

const CHILD_FIELDS: &[JsonField] = &[JsonField::new("path", &OPAQUE_JSON_SHAPE)];
const CHILD_SHAPE: JsonShape = JsonShape::Object(CHILD_FIELDS);
const NULLABLE_CHILD_SHAPE: JsonShape = JsonShape::Nullable(&CHILD_SHAPE);
const ROOT_FIELDS: &[JsonField] = &[
    JsonField::new("child", &CHILD_SHAPE),
    JsonField::new("nullable", &NULLABLE_CHILD_SHAPE),
    JsonField::new("dynamic", &OPAQUE_JSON_SHAPE),
];
const ROOT_SHAPE: JsonShape = JsonShape::Object(ROOT_FIELDS);

const COLLECTION_ITEM_FIELDS: &[JsonField] = &[JsonField::new("kept", &OPAQUE_JSON_SHAPE)];
const COLLECTION_ITEM_SHAPE: JsonShape = JsonShape::Object(COLLECTION_ITEM_FIELDS);
const COLLECTION_ARRAY_SHAPE: JsonShape = JsonShape::Array(&COLLECTION_ITEM_SHAPE);
const COLLECTION_MAP_SHAPE: JsonShape = JsonShape::MapValues(&COLLECTION_ITEM_SHAPE);
const COLLECTION_FIELDS: &[JsonField] = &[
    JsonField::new("items", &COLLECTION_ARRAY_SHAPE),
    JsonField::new("entries", &COLLECTION_MAP_SHAPE),
];
const COLLECTION_SHAPE: JsonShape = JsonShape::Object(COLLECTION_FIELDS);

const ACTIVE_FIELDS: &[JsonField] = &[JsonField::new("payload", &CHILD_SHAPE)];
const IDLE_FIELDS: &[JsonField] = &[JsonField::new("reason", &OPAQUE_JSON_SHAPE)];
const STATE_VARIANTS: &[JsonVariant] = &[
    JsonVariant::new("active", ACTIVE_FIELDS),
    JsonVariant::new("idle", IDLE_FIELDS),
];
const STATE_SHAPE: JsonShape = JsonShape::TaggedObject {
    discriminator: "type",
    common: &[JsonField::new("revision", &OPAQUE_JSON_SHAPE)],
    variants: STATE_VARIANTS,
};

#[test]
fn nested_objects_remove_unknown_fields_and_preserve_opaque_and_null_subtrees() {
    let mut value = json!({
        "child": {"path": "/tmp/demo", "old/key~name": true},
        "nullable": null,
        "dynamic": {"arbitrary": {"nested": true}},
        "oldRoot": true
    });
    let mut events = Vec::new();

    assert!(normalize_current_json(&mut value, &ROOT_SHAPE, |event| {
        events.push(event)
    }));

    assert_eq!(
        value,
        json!({
            "child": {"path": "/tmp/demo"},
            "nullable": null,
            "dynamic": {"arbitrary": {"nested": true}}
        })
    );
    assert!(events.contains(&JsonNormalizationEvent {
        kind: JsonNormalizationEventKind::FieldRemoved,
        json_pointer: "/child/old~1key~0name".to_string(),
    }));
    assert!(events.contains(&JsonNormalizationEvent {
        kind: JsonNormalizationEventKind::FieldRemoved,
        json_pointer: "/oldRoot".to_string(),
    }));

    events.clear();
    assert!(!normalize_current_json(&mut value, &ROOT_SHAPE, |event| {
        events.push(event)
    }));
    assert!(events.is_empty());
}

#[test]
fn known_tagged_objects_use_the_selected_shape() {
    let mut value = json!({
        "type": "active",
        "revision": 3,
        "payload": {"path": "/tmp/demo", "removed": true},
        "reason": "belongs to another variant",
        "removedRoot": true
    });
    let mut events = Vec::new();

    assert!(normalize_current_json(&mut value, &STATE_SHAPE, |event| {
        events.push(event)
    }));

    assert_eq!(
        value,
        json!({
            "type": "active",
            "revision": 3,
            "payload": {"path": "/tmp/demo"}
        })
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == JsonNormalizationEventKind::FieldRemoved)
            .count(),
        3
    );
}

#[test]
fn unknown_variants_and_shape_mismatches_are_reported_without_mutation() {
    let unknown = json!({"type": "future", "futurePayload": {"keep": true}});
    let mismatch = json!([{"type": "active"}]);

    for (mut value, expected_kind, expected_pointer) in [
        (unknown, JsonNormalizationEventKind::UnknownVariant, "/type"),
        (mismatch, JsonNormalizationEventKind::ShapeMismatch, ""),
    ] {
        let original = value.clone();
        let mut events = Vec::new();
        assert!(!normalize_current_json(&mut value, &STATE_SHAPE, |event| {
            events.push(event)
        }));
        assert_eq!(value, original);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, expected_kind);
        assert_eq!(events[0].json_pointer, expected_pointer);
    }
}

#[test]
fn event_pointers_are_bounded_without_exposing_source_values() {
    let oversized_key = "x".repeat(MAX_JSON_NORMALIZATION_POINTER_BYTES + 1);
    let mut value = json!({(oversized_key): "secret-value"});
    let mut events = Vec::new();

    assert!(normalize_current_json(&mut value, &ROOT_SHAPE, |event| {
        events.push(event)
    }));

    assert_eq!(events.len(), 1);
    assert!(events[0].json_pointer.len() <= MAX_JSON_NORMALIZATION_POINTER_BYTES);
    assert_eq!(events[0].json_pointer, "/<oversized-field>");
    assert!(!events[0].json_pointer.contains("secret-value"));
}

#[test]
fn arrays_and_dynamic_map_values_normalize_children_without_removing_dynamic_keys() {
    let mut value = json!({
        "items": [
            {"kept": 1, "removed": "array-secret"},
            {"kept": 2}
        ],
        "entries": {
            "dynamic/key~one": {"kept": 3, "removed": "map-secret"},
            "second": {"kept": 4}
        }
    });
    let mut events = Vec::new();

    assert!(normalize_current_json(
        &mut value,
        &COLLECTION_SHAPE,
        |event| events.push(event)
    ));

    assert_eq!(
        value,
        json!({
            "items": [{"kept": 1}, {"kept": 2}],
            "entries": {
                "dynamic/key~one": {"kept": 3},
                "second": {"kept": 4}
            }
        })
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.json_pointer.as_str())
            .collect::<Vec<_>>(),
        vec!["/items/0/removed", "/entries/dynamic~1key~0one/removed"]
    );
}

#[test]
fn collection_shape_mismatches_are_reported_without_mutation() {
    for (shape, mut value, pointer) in [
        (&COLLECTION_ARRAY_SHAPE, json!({"not": "an array"}), ""),
        (&COLLECTION_MAP_SHAPE, json!([{"kept": 1}]), ""),
    ] {
        let original = value.clone();
        let mut events = Vec::new();

        assert!(!normalize_current_json(&mut value, shape, |event| {
            events.push(event)
        }));
        assert_eq!(value, original);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, JsonNormalizationEventKind::ShapeMismatch);
        assert_eq!(events[0].json_pointer, pointer);
    }
}
