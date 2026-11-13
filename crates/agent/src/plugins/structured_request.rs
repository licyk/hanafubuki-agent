//! One structured answer from a session's model, outside the session's
//! conversation: what a generated title, a title suggestion and a recap
//! ask for. The answer is a JSON object of a schema, asked down the shared
//! chain of rungs (`request_json_object`: native output, a forced tool,
//! plain text) from the one the route still takes, and read tolerantly.

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{
    json_object_text, request_json_object, ModelJsonObjectError, ModelJsonObjectRequest,
    ModelSession,
};

/// Asks `model_session` for the object `request` describes and returns its
/// JSON text, which the caller decodes.
pub(crate) async fn request_structured(
    model_session: Arc<dyn ModelSession>,
    request: ModelJsonObjectRequest,
    cancellation: CancellationToken,
) -> Result<String, String> {
    let subject = request.subject;
    match request_json_object(model_session.as_ref(), request, cancellation).await {
        Ok(answer) => Ok(json_object_text(&answer.text).to_string()),
        Err(ModelJsonObjectError::Cancelled) => Err(format!("{subject} generation cancelled")),
        Err(ModelJsonObjectError::TimedOut) => Err(format!("{subject} generation timed out")),
        Err(ModelJsonObjectError::Failed(message)) => Err(message),
    }
}

/// Resolves once `cancellation` is cancelled.
pub(crate) async fn wait_for_cancellation(cancellation: CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// `text` cut to at most `max_bytes` on a character boundary.
pub(crate) fn bounded_text(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes.min(text.len()))]
}
