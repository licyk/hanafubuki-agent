//! One structured answer from a session's model, outside the session's
//! conversation: what a generated title, a title suggestion and a recap
//! ask for. The answer is a JSON object of a schema: the provider's native
//! JSON-schema output where it has one, otherwise a forced call of a tool
//! whose arguments are that object. The request is bounded in time and
//! ends when its cancellation is cancelled.

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{
    ModelEvent, ModelInputItem, ModelJsonSchemaOutput, ModelOutputFormat, ModelOutputItem,
    ModelRequest, ModelSession, ModelStructuredOutputCapability, ModelToolChoice, ModelToolSpec,
};

/// What one structured request asks for.
pub(crate) struct StructuredRequest {
    /// What is asked for, as the errors name it ("title", "recap").
    pub(crate) subject: &'static str,
    pub(crate) model: String,
    pub(crate) instructions: String,
    /// The one user message.
    pub(crate) input: String,
    /// The answer's JSON schema.
    pub(crate) schema: serde_json::Value,
    /// The native output's name and description.
    pub(crate) output_name: &'static str,
    pub(crate) output_description: &'static str,
    /// The forced tool's name and description, where the provider has no
    /// native output.
    pub(crate) tool_name: &'static str,
    pub(crate) tool_description: &'static str,
    pub(crate) timeout: Duration,
}

/// Asks `model_session` for the object `request` describes and returns its
/// JSON text, which the caller decodes.
pub(crate) async fn request_structured(
    model_session: Arc<dyn ModelSession>,
    request: StructuredRequest,
    cancellation: CancellationToken,
) -> Result<String, String> {
    let subject = request.subject;
    let timeout = request.timeout;
    let tool_name = request.tool_name;
    let request = match model_session.structured_output_capability() {
        ModelStructuredOutputCapability::NativeJsonSchema => native_request(request)?,
        ModelStructuredOutputCapability::TextOnly => tool_request(model_session.as_ref(), request)?,
    };
    let expected_tool = (request.output_format.is_none()).then_some(tool_name);
    let monitor = cancellation.clone();
    let sampling = async {
        let mut stream = model_session
            .stream(request, cancellation.clone())
            .await
            .map_err(|error| error.to_string())?;
        let mut result = None;
        let mut completed = false;
        while let Some(event) = stream.next().await {
            match event.map_err(|error| error.to_string())? {
                ModelEvent::OutputItemCompleted {
                    item: ModelOutputItem::AssistantMessage { text, .. },
                } if expected_tool.is_none() => {
                    if result.replace(text).is_some() {
                        return Err(format!("model returned duplicate {subject} results"));
                    }
                }
                ModelEvent::OutputItemCompleted {
                    item: ModelOutputItem::ToolCall { call },
                } if expected_tool == Some(call.name.as_str()) => {
                    if result.replace(call.arguments.to_string()).is_some() {
                        return Err(format!("model returned duplicate {subject} results"));
                    }
                }
                ModelEvent::ResponseCompleted { .. } => completed = true,
                _ => {}
            }
        }
        if cancellation.is_cancelled() {
            return Err(format!("{subject} generation cancelled"));
        }
        if !completed {
            return Err(format!("model stream ended before {subject} completion"));
        }
        result.ok_or_else(|| format!("model returned no {subject} result"))
    };
    tokio::pin!(sampling);
    tokio::select! {
        result = &mut sampling => result,
        _ = tokio::time::sleep(timeout) => Err(format!("{subject} generation timed out")),
        _ = wait_for_cancellation(monitor) => Err(format!("{subject} generation cancelled")),
    }
}

fn native_request(request: StructuredRequest) -> Result<ModelRequest, String> {
    let output = ModelJsonSchemaOutput {
        name: request.output_name.to_string(),
        description: Some(request.output_description.to_string()),
        schema: request.schema,
        strict: true,
    };
    output.validate()?;
    Ok(ModelRequest {
        model: request.model,
        instructions: request.instructions,
        input: vec![ModelInputItem::user_text(request.input)],
        tools: Vec::new(),
        tool_choice: ModelToolChoice::None,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: Some(ModelOutputFormat::JsonSchema(output)),
    })
}

fn tool_request(
    model_session: &dyn ModelSession,
    request: StructuredRequest,
) -> Result<ModelRequest, String> {
    let capabilities = model_session.tool_capabilities();
    if !capabilities.function_tools || !capabilities.required_named_tool_choice {
        return Err(format!(
            "provider cannot return a required structured {}",
            request.subject
        ));
    }
    let tool = ModelToolSpec {
        name: request.tool_name.to_string(),
        description: request.tool_description.to_string(),
        input_schema: request.schema,
        strict: true,
    }
    .try_into()
    .map_err(|error: hanafubuki_agent_tool_contracts::ToolContractError| error.to_string())?;
    Ok(ModelRequest {
        model: request.model,
        instructions: request.instructions,
        input: vec![ModelInputItem::user_text(request.input)],
        tools: vec![tool],
        tool_choice: ModelToolChoice::Required {
            name: request.tool_name.to_string(),
        },
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    })
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
