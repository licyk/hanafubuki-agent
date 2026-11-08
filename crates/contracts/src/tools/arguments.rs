//! Validation helpers for function-tool arguments. Each returns the shared
//! `ToolCallValidationError` texts the model reads back.
//!
//! The model's arguments are read tolerantly: an unknown key is ignored, a
//! missing or null optional value is absent (`nullable_*` and `optional_*`
//! alike: one schema lists every property as required-and-nullable for
//! strict providers, but omitting one means the same as null), and an
//! out-of-range limit is clamped with an [`ArgumentNotes`] entry the result
//! carries back. The kernel has already coerced `1.0` and numeric strings
//! where the tool's schema says `integer` or `number`. A required value that
//! is missing or unusable is an error naming its field.

use serde_json::{Map, Value};

pub use super::decode_arguments;
use super::ToolCallValidationError;

pub fn valid_bounded_text(value: &str, maximum: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.trim().is_empty())
        && value.len() <= maximum
        && !value.contains('\0')
        && !value
            .chars()
            .any(|character| matches!(character, '\r' | '\n'))
}

pub fn required_u64(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ToolCallValidationError> {
    arguments
        .get(name)
        .ok_or(ToolCallValidationError::MissingArgument(name))?
        .as_u64()
        .filter(|value| *value >= minimum && *value <= maximum)
        .ok_or(ToolCallValidationError::InvalidArgument(name))
}

pub fn required_usize(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: usize,
    maximum: usize,
) -> Result<usize, ToolCallValidationError> {
    let value = required_u64(arguments, name, minimum as u64, maximum as u64)?;
    usize::try_from(value).map_err(|_| ToolCallValidationError::InvalidArgument(name))
}

pub fn required_text(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum: usize,
    allow_empty: bool,
) -> Result<String, ToolCallValidationError> {
    arguments
        .get(name)
        .ok_or(ToolCallValidationError::MissingArgument(name))?
        .as_str()
        .filter(|value| {
            (allow_empty || !value.is_empty())
                && value.len() <= maximum
                && !value.as_bytes().contains(&0)
        })
        .map(str::to_owned)
        .ok_or(ToolCallValidationError::InvalidArgument(name))
}

pub fn nullable_u64(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: u64,
    maximum: u64,
) -> Result<Option<u64>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_u64(arguments, name, minimum, maximum).map(Some),
    }
}

pub fn nullable_usize(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: usize,
    maximum: usize,
) -> Result<Option<usize>, ToolCallValidationError> {
    nullable_u64(arguments, name, minimum as u64, maximum as u64)?
        .map(|value| {
            usize::try_from(value).map_err(|_| ToolCallValidationError::InvalidArgument(name))
        })
        .transpose()
}

pub fn required_bounded_string(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum: usize,
) -> Result<String, ToolCallValidationError> {
    arguments
        .get(name)
        .ok_or(ToolCallValidationError::MissingArgument(name))?
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.chars().count() <= maximum)
        .map(str::to_owned)
        .ok_or(ToolCallValidationError::InvalidArgument(name))
}

pub fn nullable_bounded_string(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum: usize,
) -> Result<Option<String>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_bounded_string(arguments, name, maximum).map(Some),
    }
}

pub fn required_bool(
    arguments: &Map<String, Value>,
    name: &'static str,
) -> Result<bool, ToolCallValidationError> {
    arguments
        .get(name)
        .ok_or(ToolCallValidationError::MissingArgument(name))?
        .as_bool()
        .ok_or(ToolCallValidationError::InvalidArgument(name))
}

pub fn nullable_bool(
    arguments: &Map<String, Value>,
    name: &'static str,
) -> Result<Option<bool>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or(ToolCallValidationError::InvalidArgument(name)),
    }
}

pub fn nullable_bool_or_default(
    arguments: &Map<String, Value>,
    name: &'static str,
    default: bool,
) -> Result<bool, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or(ToolCallValidationError::InvalidArgument(name)),
    }
}

pub fn nullable_text_or_default(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum: usize,
) -> Result<String, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(value) => value
            .as_str()
            .filter(|value| value.len() <= maximum && !value.as_bytes().contains(&0))
            .map(str::to_owned)
            .ok_or(ToolCallValidationError::InvalidArgument(name)),
    }
}

/// Read an optional argument of an action-dispatch tool.
///
/// An absent key and an explicit null are both "not supplied": one schema
/// serves every action, so most properties are irrelevant to any single call.
pub fn optional_bounded_string(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum: usize,
) -> Result<Option<String>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_bounded_string(arguments, name, maximum).map(Some),
    }
}

pub fn optional_text(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum: usize,
) -> Result<Option<String>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|value| value.len() <= maximum && !value.as_bytes().contains(&0))
            .map(str::to_owned)
            .map(Some)
            .ok_or(ToolCallValidationError::InvalidArgument(name)),
    }
}

pub fn optional_bool(
    arguments: &Map<String, Value>,
    name: &'static str,
) -> Result<Option<bool>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or(ToolCallValidationError::InvalidArgument(name)),
    }
}

pub fn optional_u64(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: u64,
    maximum: u64,
) -> Result<Option<u64>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_u64(arguments, name, minimum, maximum).map(Some),
    }
}

pub fn optional_string_array(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum_items: usize,
    maximum_length: usize,
) -> Result<Vec<String>, ToolCallValidationError> {
    let Some(value) = arguments.get(name) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) if items.len() <= maximum_items => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|item| valid_bounded_text(item, maximum_length, false))
                    .map(str::to_owned)
                    .ok_or(ToolCallValidationError::InvalidArgument(name))
            })
            .collect(),
        _ => Err(ToolCallValidationError::InvalidArgument(name)),
    }
}

/// Resolve an argument the selected action requires.
pub fn required_for_action<T>(
    value: Option<T>,
    name: &'static str,
) -> Result<T, ToolCallValidationError> {
    value.ok_or(ToolCallValidationError::MissingArgument(name))
}

pub fn required_operation_id(
    arguments: &Map<String, Value>,
) -> Result<String, ToolCallValidationError> {
    arguments
        .get("operationId")
        .ok_or(ToolCallValidationError::MissingArgument("operationId"))?
        .as_str()
        .filter(|value| !value.is_empty() && value.chars().count() <= 256)
        .map(str::to_owned)
        .ok_or(ToolCallValidationError::InvalidIdentifier("operationId"))
}

/// Rejects a non-null argument that another action of the same tool takes
/// (`known` lists every property the tool's shared schema declares), naming
/// the accepted set so the model can correct itself in one turn. A key the
/// schema does not know and a null are ignored: one schema serves every
/// action, and only a value meant for a different action is a sign the model
/// confused the actions.
pub fn reject_other_action_arguments(
    arguments: &Map<String, Value>,
    accepted: &[&str],
    known: &[&str],
) -> Result<(), ToolCallValidationError> {
    if let Some(name) = arguments.iter().find_map(|(name, value)| {
        (!value.is_null() && known.contains(&name.as_str()) && !accepted.contains(&name.as_str()))
            .then_some(name)
    }) {
        return Err(ToolCallValidationError::UnexpectedArgument {
            name: name.clone(),
            accepted: accepted.join(", "),
        });
    }
    Ok(())
}

/// How a handler adjusted the model's arguments instead of rejecting them,
/// for the result to say: an out-of-range limit is clamped, never widened
/// past the bound that guards it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArgumentNotes(Vec<String>);

impl ArgumentNotes {
    pub fn push(&mut self, note: impl Into<String>) {
        self.0.push(note.into());
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn notes(&self) -> &[String] {
        &self.0
    }

    /// The one line the result carries, or `None` without notes.
    pub fn render(&self) -> Option<String> {
        (!self.0.is_empty()).then(|| format!("Argument notes: {}.", self.0.join("; ")))
    }
}

/// A limit argument: absent or null is `default`; a value outside
/// `minimum..=maximum` is clamped into it with a note. A value that is not an
/// integer is an error naming the field.
pub fn clamped_u64(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: u64,
    maximum: u64,
    default: u64,
    notes: &mut ArgumentNotes,
) -> Result<u64, ToolCallValidationError> {
    Ok(optional_clamped_u64(arguments, name, minimum, maximum, notes)?.unwrap_or(default))
}

/// [`clamped_u64`] without a default: absent or null is `None`.
pub fn optional_clamped_u64(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: u64,
    maximum: u64,
    notes: &mut ArgumentNotes,
) -> Result<Option<u64>, ToolCallValidationError> {
    let value = match arguments.get(name) {
        None | Some(Value::Null) => return Ok(None),
        Some(value) => value,
    };
    let clamped = if let Some(value) = value.as_u64() {
        value.clamp(minimum, maximum)
    } else if value.as_i64().is_some() {
        // A negative integer is below any unsigned minimum.
        minimum
    } else {
        return Err(ToolCallValidationError::InvalidArgumentAt {
            path: name.to_string(),
            reason: "must be an integer".to_string(),
        });
    };
    if value.as_u64() != Some(clamped) {
        let bound = if value.as_u64().is_some_and(|value| value > maximum) {
            "above its maximum"
        } else {
            "below its minimum"
        };
        notes.push(format!("{name} {value} is {bound}; {clamped} was used"));
    }
    Ok(Some(clamped))
}

/// [`clamped_u64`] for a `usize` limit.
pub fn clamped_usize(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: usize,
    maximum: usize,
    default: usize,
    notes: &mut ArgumentNotes,
) -> Result<usize, ToolCallValidationError> {
    Ok(optional_clamped_usize(arguments, name, minimum, maximum, notes)?.unwrap_or(default))
}

/// [`optional_clamped_u64`] for a `usize` limit.
pub fn optional_clamped_usize(
    arguments: &Map<String, Value>,
    name: &'static str,
    minimum: usize,
    maximum: usize,
    notes: &mut ArgumentNotes,
) -> Result<Option<usize>, ToolCallValidationError> {
    optional_clamped_u64(arguments, name, minimum as u64, maximum as u64, notes)?
        .map(|value| {
            usize::try_from(value).map_err(|_| ToolCallValidationError::InvalidArgument(name))
        })
        .transpose()
}
