//! Domain-neutral unknown-field normalization for current JSON contracts.
//!
//! The mechanism traverses only shapes declared by the owning domain. It never
//! supplies defaults, translates old fields, validates values, performs I/O or
//! chooses a recovery policy. Structurally invalid subtrees remain untouched so
//! the owner's strict decoder can decide whether to reset or reject them.

use serde_json::{Map, Value};

/// A subtree whose fields are owned by domain-specific logic and remain intact.
pub const OPAQUE_JSON_SHAPE: JsonShape = JsonShape::Opaque;
/// Maximum emitted JSON Pointer size. Source values are never used as fallback.
pub const MAX_JSON_NORMALIZATION_POINTER_BYTES: usize = 4096;
const OVERSIZED_SEGMENT: &str = "<oversized-field>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonShape {
    /// Preserve the complete subtree without inspecting it.
    Opaque,
    /// Remove unknown fields and recursively normalize declared children.
    Object(&'static [JsonField]),
    /// Normalize every item in a homogeneous array.
    Array(&'static JsonShape),
    /// Preserve dynamic object keys and normalize every mapped value.
    MapValues(&'static JsonShape),
    /// Accept `null`; otherwise normalize using the nested shape.
    Nullable(&'static JsonShape),
    /// Select an object shape using a string discriminator.
    TaggedObject {
        discriminator: &'static str,
        common: &'static [JsonField],
        variants: &'static [JsonVariant],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonField {
    pub name: &'static str,
    pub shape: &'static JsonShape,
}

impl JsonField {
    pub const fn new(name: &'static str, shape: &'static JsonShape) -> Self {
        Self { name, shape }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonVariant {
    pub tag: &'static str,
    pub fields: &'static [JsonField],
}

impl JsonVariant {
    pub const fn new(tag: &'static str, fields: &'static [JsonField]) -> Self {
        Self { tag, fields }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonNormalizationEventKind {
    FieldRemoved,
    ShapeMismatch,
    UnknownVariant,
}

/// Value-free normalization evidence safe for a domain recovery report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonNormalizationEvent {
    pub kind: JsonNormalizationEventKind,
    pub json_pointer: String,
}

/// Normalize unknown fields in place according to one current JSON shape.
///
/// Returns whether fields were removed. Events never contain source values;
/// callers decide whether and how many of them to retain.
pub fn normalize_current_json(
    value: &mut Value,
    shape: &JsonShape,
    mut emit: impl FnMut(JsonNormalizationEvent),
) -> bool {
    normalize_value(value, shape, "", &mut emit)
}

/// Append one escaped segment to an RFC 6901 JSON Pointer.
pub fn json_pointer_child(parent: &str, segment: &str) -> String {
    let pointer = format!("{parent}/{}", escape_json_pointer_segment(segment));
    if pointer.len() <= MAX_JSON_NORMALIZATION_POINTER_BYTES {
        pointer
    } else {
        format!("{parent}/{OVERSIZED_SEGMENT}")
    }
}

/// Escape one RFC 6901 JSON Pointer segment.
pub fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn normalize_value(
    value: &mut Value,
    shape: &JsonShape,
    pointer: &str,
    emit: &mut impl FnMut(JsonNormalizationEvent),
) -> bool {
    match shape {
        JsonShape::Opaque => false,
        JsonShape::Nullable(_) if value.is_null() => false,
        JsonShape::Nullable(nested) => normalize_value(value, nested, pointer, emit),
        JsonShape::Object(fields) => {
            let Some(object) = value.as_object_mut() else {
                emit_event(emit, JsonNormalizationEventKind::ShapeMismatch, pointer);
                return false;
            };
            normalize_object(object, fields, &[], None, pointer, emit)
        }
        JsonShape::Array(item_shape) => {
            let Some(items) = value.as_array_mut() else {
                emit_event(emit, JsonNormalizationEventKind::ShapeMismatch, pointer);
                return false;
            };
            let mut changed = false;
            for (index, item) in items.iter_mut().enumerate() {
                changed |= normalize_value(
                    item,
                    item_shape,
                    &json_pointer_child(pointer, &index.to_string()),
                    emit,
                );
            }
            changed
        }
        JsonShape::MapValues(value_shape) => {
            let Some(object) = value.as_object_mut() else {
                emit_event(emit, JsonNormalizationEventKind::ShapeMismatch, pointer);
                return false;
            };
            let mut changed = false;
            for (key, child) in object {
                changed |=
                    normalize_value(child, value_shape, &json_pointer_child(pointer, key), emit);
            }
            changed
        }
        JsonShape::TaggedObject {
            discriminator,
            common,
            variants,
        } => {
            let Some(object) = value.as_object_mut() else {
                emit_event(emit, JsonNormalizationEventKind::ShapeMismatch, pointer);
                return false;
            };
            let Some(tag) = object.get(*discriminator).and_then(Value::as_str) else {
                emit_event(
                    emit,
                    JsonNormalizationEventKind::UnknownVariant,
                    &json_pointer_child(pointer, discriminator),
                );
                return false;
            };
            let Some(variant) = variants.iter().find(|variant| variant.tag == tag) else {
                emit_event(
                    emit,
                    JsonNormalizationEventKind::UnknownVariant,
                    &json_pointer_child(pointer, discriminator),
                );
                return false;
            };
            normalize_object(
                object,
                common,
                variant.fields,
                Some(discriminator),
                pointer,
                emit,
            )
        }
    }
}

fn normalize_object(
    object: &mut Map<String, Value>,
    common: &[JsonField],
    variant: &[JsonField],
    discriminator: Option<&str>,
    pointer: &str,
    emit: &mut impl FnMut(JsonNormalizationEvent),
) -> bool {
    let unknown: Vec<String> = object
        .keys()
        .filter(|key| {
            discriminator != Some(key.as_str())
                && !common.iter().any(|field| field.name == key.as_str())
                && !variant.iter().any(|field| field.name == key.as_str())
        })
        .cloned()
        .collect();
    let mut changed = false;
    for key in unknown {
        object.remove(&key);
        emit_event(
            emit,
            JsonNormalizationEventKind::FieldRemoved,
            &json_pointer_child(pointer, &key),
        );
        changed = true;
    }
    for field in common.iter().chain(variant) {
        if let Some(child) = object.get_mut(field.name) {
            changed |= normalize_value(
                child,
                field.shape,
                &json_pointer_child(pointer, field.name),
                emit,
            );
        }
    }
    changed
}

fn emit_event(
    emit: &mut impl FnMut(JsonNormalizationEvent),
    kind: JsonNormalizationEventKind,
    pointer: &str,
) {
    emit(JsonNormalizationEvent {
        kind,
        json_pointer: pointer.to_string(),
    });
}
