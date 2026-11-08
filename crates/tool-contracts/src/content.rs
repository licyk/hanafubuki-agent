use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{canonical_json, ToolContractError};

pub const MAX_TOOL_OUTPUT_ITEMS: usize = 64;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputImageDetail {
    Low,
    High,
    Auto,
    Original,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolOutputContent {
    Text {
        text: String,
    },
    Structured {
        value: Value,
    },
    Image {
        mime_type: String,
        data: String,
        detail: Option<ToolOutputImageDetail>,
    },
    Audio {
        mime_type: String,
        data: String,
    },
    ResourceLink {
        name: String,
        uri: String,
        mime_type: Option<String>,
    },
    EmbeddedResource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
        data: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    pub content: Vec<ToolOutputContent>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn json(value: Value, is_error: bool) -> Result<Self, ToolContractError> {
        let value = canonical_json(&value);
        let output = Self {
            content: vec![ToolOutputContent::Structured {
                value: value.clone(),
            }],
            structured_content: Some(value),
            is_error,
        };
        output.validate()?;
        Ok(output)
    }

    pub fn validate(&self) -> Result<(), ToolContractError> {
        if self.content.len() > MAX_TOOL_OUTPUT_ITEMS {
            return Err(ToolContractError::TooManyOutputItems);
        }
        for item in &self.content {
            match item {
                ToolOutputContent::Image {
                    mime_type, data: _, ..
                } if !mime_type.starts_with("image/") || !valid_mime(mime_type) => {
                    return Err(ToolContractError::InvalidMimeType)
                }
                ToolOutputContent::Audio { mime_type, data: _ }
                    if !mime_type.starts_with("audio/") || !valid_mime(mime_type) =>
                {
                    return Err(ToolContractError::InvalidMimeType)
                }
                ToolOutputContent::Image { data, .. } | ToolOutputContent::Audio { data, .. }
                    if !valid_base64(data) =>
                {
                    return Err(ToolContractError::OutputTooLarge)
                }
                ToolOutputContent::EmbeddedResource {
                    data: Some(data), ..
                } if !valid_base64(data) => return Err(ToolContractError::OutputTooLarge),
                ToolOutputContent::ResourceLink { uri, .. }
                | ToolOutputContent::EmbeddedResource { uri, .. }
                    if !valid_external_uri(uri) =>
                {
                    return Err(ToolContractError::InvalidExternalUrl)
                }
                _ => {}
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|_| ToolContractError::OutputTooLarge)?;
        if bytes.len() > MAX_TOOL_OUTPUT_BYTES {
            return Err(ToolContractError::OutputTooLarge);
        }
        Ok(())
    }

    pub fn structured(value: Value, is_error: bool) -> Self {
        let value = canonical_json(&value);
        Self {
            content: vec![ToolOutputContent::Structured {
                value: value.clone(),
            }],
            structured_content: Some(value),
            is_error,
        }
    }

    /// Explicit bounded-text projection for protocols without typed tool
    /// result modalities. Binary payloads are represented by safe metadata,
    /// never silently dropped or embedded into an arbitrary JSON wrapper.
    pub fn legacy_text_projection(&self) -> Result<String, ToolContractError> {
        self.validate()?;
        let mut projected = Vec::with_capacity(self.content.len());
        for item in &self.content {
            projected.push(match item {
                ToolOutputContent::Text { text } => Value::String(text.clone()),
                ToolOutputContent::Structured { value } => canonical_json(value),
                ToolOutputContent::Image { mime_type, .. } => serde_json::json!({
                    "type": "unsupported_image",
                    "mimeType": mime_type,
                    "message": "The provider cannot receive binary image tool output."
                }),
                ToolOutputContent::Audio { mime_type, .. } => serde_json::json!({
                    "type": "unsupported_audio",
                    "mimeType": mime_type,
                    "message": "The provider cannot receive binary audio tool output."
                }),
                ToolOutputContent::ResourceLink {
                    name,
                    uri,
                    mime_type,
                } => serde_json::json!({
                    "type": "resource_link",
                    "name": name,
                    "uri": uri,
                    "mimeType": mime_type,
                }),
                ToolOutputContent::EmbeddedResource {
                    uri,
                    mime_type,
                    text,
                    data,
                } => serde_json::json!({
                    "type": "embedded_resource",
                    "uri": uri,
                    "mimeType": mime_type,
                    "text": text,
                    "binaryDataOmitted": data.is_some(),
                }),
            });
        }
        serde_json::to_string(&serde_json::json!({
            "content": projected,
            "structuredContent": self.structured_content,
            "isError": self.is_error,
        }))
        .map_err(|_| ToolContractError::OutputTooLarge)
    }
}

impl From<Value> for ToolOutput {
    fn from(value: Value) -> Self {
        Self::structured(value, false)
    }
}

/// The base type of a media type: parameters dropped, whitespace trimmed,
/// ASCII lowercased (`Image/PNG; charset=binary` is `image/png`). Producers
/// store this form; [`ToolOutput::validate`] checks it, since consumers pass
/// the type verbatim into data URLs and provider media fields.
pub fn canonical_media_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Base64 with its padding restored when a producer omitted it. Only padding
/// is repaired: another alphabet or embedded whitespace still fails
/// [`ToolOutput::validate`].
pub fn canonical_base64(mut value: String) -> String {
    if value.len() % 4 > 1 && !value.ends_with('=') {
        let missing = 4 - value.len() % 4;
        value.extend(std::iter::repeat_n('=', missing));
    }
    value
}

/// Validation is indifferent to padding. Stored data is padded anyway: the
/// MCP boundary applies [`canonical_base64`] and in-tree producers encode
/// with `STANDARD`, since providers receive the data verbatim.
const BASE64_INDIFFERENT: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
);

fn valid_base64(value: &str) -> bool {
    value.len() <= MAX_TOOL_OUTPUT_BYTES && BASE64_INDIFFERENT.decode(value).is_ok()
}

fn valid_mime(value: &str) -> bool {
    value.len() <= 128
        && value.contains('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'+' | b'.'))
}

fn valid_external_uri(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && !rest.is_empty()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}
