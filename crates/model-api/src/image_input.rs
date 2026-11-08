use hanafubuki_agent_tool_contracts::ToolOutputContent;

use std::fmt;

use crate::{
    ModelApiProtocol, ModelEndpoint, ModelError, ModelInputItem, ModelRequest, ModelUserContent,
};

pub const IMAGE_INPUT_OMITTED_PLACEHOLDER: &str =
    "image content omitted because the selected model endpoint does not support image input";

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ModelRouteKey {
    protocol: ModelApiProtocol,
    endpoint_identity: String,
    model: String,
}

impl ModelRouteKey {
    pub fn new(protocol: ModelApiProtocol, endpoint: &ModelEndpoint, model: &str) -> Self {
        Self {
            protocol,
            endpoint_identity: endpoint.canonical_identity().to_string(),
            model: model.trim().to_string(),
        }
    }
}

impl fmt::Debug for ModelRouteKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelRouteKey")
            .field("protocol", &self.protocol)
            .field("endpoint_identity", &"[redacted]")
            .field("model", &self.model)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageSanitizationReport {
    pub replaced_user_images: usize,
    pub replaced_tool_images: usize,
    pub replaced_embedded_images: usize,
}

impl ImageSanitizationReport {
    pub fn total_replaced(self) -> usize {
        self.replaced_user_images
            .saturating_add(self.replaced_tool_images)
            .saturating_add(self.replaced_embedded_images)
    }
}

pub fn request_contains_images(request: &ModelRequest) -> bool {
    request.input.iter().any(input_item_contains_image)
}

pub fn sanitize_images_for_unsupported_input(
    request: &ModelRequest,
) -> (ModelRequest, ImageSanitizationReport) {
    let mut sanitized = request.clone();
    let mut report = ImageSanitizationReport::default();
    for item in &mut sanitized.input {
        match item {
            ModelInputItem::UserMessage { content } => {
                for part in content {
                    if matches!(part, ModelUserContent::InputImage { .. }) {
                        *part = ModelUserContent::InputText {
                            text: IMAGE_INPUT_OMITTED_PLACEHOLDER.to_string(),
                        };
                        report.replaced_user_images = report.replaced_user_images.saturating_add(1);
                    }
                }
            }
            ModelInputItem::ToolOutput { output, .. }
            | ModelInputItem::CustomToolOutput { output, .. } => {
                for part in &mut output.content {
                    match part {
                        ToolOutputContent::Image { .. } => {
                            *part = ToolOutputContent::Text {
                                text: IMAGE_INPUT_OMITTED_PLACEHOLDER.to_string(),
                            };
                            report.replaced_tool_images =
                                report.replaced_tool_images.saturating_add(1);
                        }
                        ToolOutputContent::EmbeddedResource {
                            mime_type: Some(mime_type),
                            data: Some(_),
                            ..
                        } if is_image_mime_type(mime_type) => {
                            *part = ToolOutputContent::Text {
                                text: IMAGE_INPUT_OMITTED_PLACEHOLDER.to_string(),
                            };
                            report.replaced_embedded_images =
                                report.replaced_embedded_images.saturating_add(1);
                        }
                        _ => {}
                    }
                }
                debug_assert!(output.validate().is_ok());
            }
            _ => {}
        }
    }
    (sanitized, report)
}

fn input_item_contains_image(item: &ModelInputItem) -> bool {
    match item {
        ModelInputItem::UserMessage { content } => content
            .iter()
            .any(|part| matches!(part, ModelUserContent::InputImage { .. })),
        ModelInputItem::ToolOutput { output, .. }
        | ModelInputItem::CustomToolOutput { output, .. } => output.content.iter().any(|part| {
            matches!(part, ToolOutputContent::Image { .. })
                || matches!(
                    part,
                    ToolOutputContent::EmbeddedResource {
                        mime_type: Some(mime_type),
                        data: Some(_),
                        ..
                    } if is_image_mime_type(mime_type)
                )
        }),
        _ => false,
    }
}

fn is_image_mime_type(mime_type: &str) -> bool {
    mime_type
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
}

pub fn classify_remote_model_error(
    status: Option<u16>,
    code: Option<String>,
    message: String,
    request_id: Option<String>,
) -> ModelError {
    if is_context_window_exceeded(status, code.as_deref(), &message) {
        ModelError::ContextWindowExceeded {
            status,
            code,
            message,
            request_id,
        }
    } else if is_unsupported_image_input(status, code.as_deref(), &message) {
        ModelError::UnsupportedImageInput {
            status,
            code,
            message,
            request_id,
        }
    } else {
        ModelError::Remote {
            status,
            code,
            message,
            request_id,
        }
    }
}

fn is_context_window_exceeded(status: Option<u16>, code: Option<&str>, message: &str) -> bool {
    if code.is_some_and(|code| {
        matches!(
            code.to_ascii_lowercase().as_str(),
            "context_length_exceeded" | "context_window_exceeded" | "prompt_too_long"
        )
    }) {
        return true;
    }
    if status.is_some_and(|status| !matches!(status, 400 | 413 | 422)) {
        return false;
    }
    let diagnostic = message.to_ascii_lowercase();
    [
        "context window exceeded",
        "maximum context length",
        "input exceeds the context window",
        "prompt is too long",
        "too many tokens",
    ]
    .iter()
    .any(|marker| diagnostic.contains(marker))
}

fn is_unsupported_image_input(status: Option<u16>, code: Option<&str>, message: &str) -> bool {
    if status.is_some_and(|status| !matches!(status, 400 | 415 | 422 | 501)) {
        return false;
    }
    let message = format!("{} {message}", code.unwrap_or_default()).to_ascii_lowercase();
    if ["only support text", "only supports text"]
        .iter()
        .any(|hint| message.contains(hint))
    {
        return true;
    }
    let mentions_image = [
        "image",
        "vision",
        "multimodal",
        "multi-modal",
        "modality",
        "modalities",
        "media",
        "attachment",
    ]
    .iter()
    .any(|hint| message.contains(hint));
    let rejects_input = [
        "unsupported",
        "not supported",
        "does not support",
        "doesn't support",
        "do not support",
        "don't support",
        "text only",
        "text-only",
        "invalid content type",
        "unknown content type",
        "cannot process",
        "cannot be processed",
        "cannot handle",
        "can't process",
        "can't handle",
        "unable to process",
    ]
    .iter()
    .any(|hint| message.contains(hint));
    mentions_image && rejects_input
}

#[cfg(test)]
mod tests;
