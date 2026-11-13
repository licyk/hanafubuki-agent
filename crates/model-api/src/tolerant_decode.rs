//! Tolerant decoding of values a model provider produced, shared by the
//! protocol decoders.
//!
//! Providers are external producers: a value the Agent cannot use must not
//! fail the response it arrived in when the rest of the response is usable.
//! The rules here are the ones every protocol shares; each decoder keeps its
//! own identity checks (response, item, and call ids).

use serde_json::{Map, Value};

use crate::model::{ModelToolArgumentError, ModelUsage};

/// The arguments of a call without arguments, and of a call whose arguments
/// could not be used.
pub(crate) fn empty_arguments() -> Value {
    Value::Object(Map::new())
}

/// Function tool arguments from the raw JSON text a provider streamed.
///
/// Empty (or whitespace-only) text means a call without arguments, `{}`.
/// Text that is not JSON becomes `{}` with an argument error carrying the
/// parser's message and a bounded preview, so the call still pairs with an
/// output that tells the model what went wrong.
pub(crate) fn function_arguments(
    protocol: &'static str,
    raw: &str,
) -> (Value, Option<ModelToolArgumentError>) {
    if raw.trim().is_empty() {
        return (empty_arguments(), None);
    }
    match serde_json::from_str(raw) {
        Ok(arguments) => (arguments, None),
        Err(error) => (
            empty_arguments(),
            Some(unusable_arguments(protocol, error.to_string(), raw)),
        ),
    }
}

/// An argument error for `raw`, recorded at debug without the arguments
/// themselves.
pub(crate) fn unusable_arguments(
    protocol: &'static str,
    reason: String,
    raw: &str,
) -> ModelToolArgumentError {
    log::debug!(
        "model.tool_arguments.unusable protocol={protocol} raw_bytes={} reason={reason}",
        raw.len()
    );
    ModelToolArgumentError::new(reason, raw)
}

/// A token count that is present but not an unsigned integer.
pub(crate) struct InvalidTokenCount;

/// One token count of a provider usage object: `Ok(None)` when the field is
/// absent or null, `Err` (logged at debug) for any other value that is not
/// an unsigned integer.
pub(crate) fn token_count(
    protocol: &'static str,
    usage: &Value,
    field: &str,
) -> Result<Option<u64>, InvalidTokenCount> {
    match usage.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            log::debug!("model.usage.ignored protocol={protocol} field={field} reason=invalid");
            InvalidTokenCount
        }),
    }
}

/// A provider usage object with an input and an output count.
///
/// Usage is diagnostic and never fails a response. No usage, a null usage,
/// a usage that is not an object, a usage with an invalid count, and a usage
/// with neither count are all no usage (`None`); a usage with one count has
/// the missing one as 0. A cache count that does not decode is `None` on its
/// own.
pub(crate) fn token_usage(
    protocol: &'static str,
    usage: Option<&Value>,
    fields: &UsageFields,
) -> Option<ModelUsage> {
    let usage = match usage? {
        Value::Null => return None,
        usage @ Value::Object(_) => usage,
        _ => {
            log::debug!("model.usage.ignored protocol={protocol} reason=not_an_object");
            return None;
        }
    };
    let input = token_count(protocol, usage, fields.input).ok()?;
    let output = token_count(protocol, usage, fields.output).ok()?;
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(ModelUsage {
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
        cache_read_tokens: fields
            .cache_read
            .iter()
            .find_map(|pointer| cache_token_count(protocol, usage, pointer)),
        cache_write_tokens: None,
    })
}

/// Where a protocol's usage object keeps its counts.
pub(crate) struct UsageFields {
    pub(crate) input: &'static str,
    pub(crate) output: &'static str,
    /// JSON pointers to the cache-read count, the first that decodes
    /// winning. The input count already includes it.
    pub(crate) cache_read: &'static [&'static str],
}

/// The cache count at `pointer` of a usage object: `None` when absent, null
/// or not an unsigned integer (logged at debug).
pub(crate) fn cache_token_count(
    protocol: &'static str,
    usage: &Value,
    pointer: &str,
) -> Option<u64> {
    match usage.pointer(pointer) {
        None | Some(Value::Null) => None,
        Some(value) => value.as_u64().or_else(|| {
            log::debug!("model.usage.ignored protocol={protocol} field={pointer} reason=invalid");
            None
        }),
    }
}

#[cfg(test)]
mod tests;
