//! One entry of a listing.

use serde_json::Value;

use super::{shapes, ModelCatalogEntry};
use crate::ModelReasoningEffort;

const MAX_MODEL_ID_BYTES: usize = 512;
const MAX_REASONING_EFFORTS_PER_MODEL: usize = 32;

/// The entry `item` describes, or `None` when it has no usable id: none, not
/// a string, blank, or over 512 bytes. Any other field it cannot read is
/// unknown, and a reasoning effort that does not parse is left out.
pub(super) fn decode_entry(item: &Value) -> Option<ModelCatalogEntry> {
    let id = item
        .get("id")?
        .as_str()
        .filter(|id| !id.trim().is_empty() && id.len() <= MAX_MODEL_ID_BYTES)?;
    let display_name = item
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(|name| bounded_text(name, MAX_MODEL_ID_BYTES));
    let default_reasoning_effort = shapes::default_reasoning_effort(item);
    let supported_reasoning_efforts = supported_reasoning_efforts(item);
    let reports_levels =
        default_reasoning_effort.is_some() || !supported_reasoning_efforts.is_empty();
    Some(ModelCatalogEntry {
        id: id.to_owned(),
        display_name,
        default_reasoning_effort,
        supported_reasoning_efforts,
        context_window: shapes::context_window(item),
        max_output_tokens: shapes::max_output_tokens(item),
        image_input: shapes::image_input(item),
        reasoning: shapes::reasoning(item, reports_levels),
    })
}

/// The efforts the entry lists (`shapes::reasoning_efforts`), each once,
/// the first 32, on the ladder's order (`reasoning_effort_rank`) with the
/// efforts off it after, in the entry's order.
fn supported_reasoning_efforts(item: &Value) -> Vec<ModelReasoningEffort> {
    let mut efforts = Vec::new();
    for effort in shapes::reasoning_efforts(item) {
        if efforts.len() == MAX_REASONING_EFFORTS_PER_MODEL {
            break;
        }
        if !efforts.contains(&effort) {
            efforts.push(effort);
        }
    }
    efforts.sort_by_key(|effort| crate::reasoning_effort_rank(effort).unwrap_or(u8::MAX));
    efforts
}

fn bounded_text(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_owned();
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_owned()
}

#[cfg(test)]
mod tests;
