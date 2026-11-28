//! One entry of a listing.

use serde_json::Value;

use super::{shapes, ModelCatalogEntry};
use crate::{ModelApiProtocol, ModelReasoningEffort};

const MAX_MODEL_ID_BYTES: usize = 512;
const MAX_REASONING_EFFORTS_PER_MODEL: usize = 32;

/// Reads a model id from Google's resource name, OpenAI's id or Codex's
/// slug. A missing or unusable id skips the entry; other fields are optional.
pub(super) fn decode_entry(protocol: ModelApiProtocol, item: &Value) -> Option<ModelCatalogEntry> {
    let google = matches!(
        protocol,
        ModelApiProtocol::GeminiGenerateContent | ModelApiProtocol::GeminiInteractions
    );
    let id_fields: &[&str] = if google {
        &["name", "id", "slug"]
    } else {
        &["id", "slug"]
    };
    let id = id_fields
        .iter()
        .filter_map(|key| item.get(*key).and_then(Value::as_str))
        .map(|id| protocol.model_id(id))
        .find(|id| !id.trim().is_empty() && id.len() <= MAX_MODEL_ID_BYTES)?;
    let display_name = item
        .get(if google {
            "displayName"
        } else {
            "display_name"
        })
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
        bytes_per_token_millis: shapes::bytes_per_token_millis(item),
        image_input: shapes::image_input(item),
        reasoning: shapes::reasoning(item, reports_levels),
    })
}

/// An explicit embedding-only list cannot serve an Agent turn. Unknown
/// methods and absent lists leave the model available.
pub(super) fn embedding_only(item: &Value) -> bool {
    let Some(methods) = item["supportedGenerationMethods"].as_array() else {
        return false;
    };
    let mut embedding = false;
    for method in methods {
        match method.as_str() {
            Some(
                "embedContent" | "batchEmbedContents" | "asyncBatchEmbedContent" | "embedText",
            ) => embedding = true,
            Some("countTokens" | "countTextTokens") => {}
            _ => return false,
        }
    }
    embedding
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
