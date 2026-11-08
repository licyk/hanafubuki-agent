use serde_json::json;

use super::normalize_tool_arguments;

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "count": { "type": "integer", "minimum": 1, "maximum": 8 },
            "ratio": { "type": "number" },
            "label": { "type": "string" },
            "either": { "type": ["integer", "string"] },
            "nullable": { "type": ["integer", "null"] },
            "optional": { "type": "boolean" },
            "required": { "type": "boolean" },
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "size": { "type": "integer" } }
                }
            }
        },
        "required": ["required"]
    })
}

#[test]
fn integral_floats_and_numeric_strings_become_numbers_where_the_schema_says_so() {
    let normalized = normalize_tool_arguments(
        &schema(),
        &json!({
            "count": 3.0,
            "ratio": " 0.5 ",
            "label": "7",
            "either": "7",
            "steps": [{ "size": "2" }, { "size": 4.0 }]
        }),
    );
    assert_eq!(
        normalized,
        json!({
            "count": 3,
            "ratio": 0.5,
            "label": "7",
            "either": "7",
            "steps": [{ "size": 2 }, { "size": 4 }]
        })
    );
}

#[test]
fn ambiguous_values_are_left_for_the_handler_to_report() {
    let arguments = json!({ "count": 2.5, "ratio": "half", "nullable": null });
    assert_eq!(normalize_tool_arguments(&schema(), &arguments), arguments);
    // Out of range stays out of range: the pass applies no bound.
    let arguments = json!({ "count": 50 });
    assert_eq!(normalize_tool_arguments(&schema(), &arguments), arguments);
}

#[test]
fn a_null_optional_property_is_absent_and_unknown_keys_stay() {
    let normalized = normalize_tool_arguments(
        &schema(),
        &json!({ "optional": null, "required": null, "unknown": 1 }),
    );
    assert_eq!(normalized, json!({ "required": null, "unknown": 1 }));
}

#[test]
fn a_closed_schema_drops_keys_it_does_not_declare() {
    let schema = json!({
        "type": "object",
        "properties": { "path": { "type": "string" } },
        "additionalProperties": false
    });
    assert_eq!(
        normalize_tool_arguments(&schema, &json!({ "path": "a", "extra": true })),
        json!({ "path": "a" })
    );
}
