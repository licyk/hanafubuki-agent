//! One JSON object asked of a model outside any conversation (a title, a
//! recap, a memory extraction, a permission review), down a chain of
//! rungs: the protocol's native JSON-schema output, a forced call of a tool
//! whose arguments are the object, or plain text. The chain starts at the
//! best rung the session gives the route
//! ([`ModelSession::structured_output_capability_for`]); a refusal of a
//! rung teaches the route the next one
//! ([`ModelSession::learn_structured_output`]) and the request is sent
//! again there at once, so later requests pay no refused request. The whole
//! chain is bounded in time and ends when its cancellation is cancelled.

use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::Value;

use crate::{
    ModelError, ModelEvent, ModelInputItem, ModelJsonSchemaOutput, ModelOutputFormat,
    ModelOutputItem, ModelRequest, ModelSession, ModelStructuredOutputCapability, ModelToolChoice,
    ModelToolSpec, ModelUsage,
};

/// What one JSON object is asked with.
pub struct ModelJsonObjectRequest {
    /// What is asked for, as the errors name it ("title", "recap").
    pub subject: &'static str,
    pub model: String,
    pub instructions: String,
    /// The one user message.
    pub input: String,
    /// The object's JSON schema.
    pub schema: Value,
    /// The native output's name and description.
    pub output_name: &'static str,
    pub output_description: &'static str,
    /// The forced tool; `None` skips that rung.
    pub tool: Option<ModelJsonObjectTool>,
    /// Whether the text rung's instructions gain a sentence asking for
    /// only the object, with its schema; `false` keeps instructions that
    /// already ask for it.
    pub schema_in_text: bool,
    /// The bound of the whole chain.
    pub timeout: Duration,
}

/// The forced tool's name and description.
pub struct ModelJsonObjectTool {
    pub name: &'static str,
    pub description: &'static str,
}

/// The model's answer.
#[derive(Debug)]
pub struct ModelJsonObjectAnswer {
    /// The forced tool's arguments, or the message the model wrote,
    /// trimmed. A message may hold more than the object; a tolerant caller
    /// reads it with [`json_object_text`].
    pub text: String,
    /// The rung that answered.
    pub rung: ModelStructuredOutputCapability,
    pub usage: Option<ModelUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelJsonObjectError {
    Cancelled,
    TimedOut,
    Failed(String),
}

impl std::fmt::Display for ModelJsonObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("cancelled"),
            Self::TimedOut => formatter.write_str("timed out"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ModelJsonObjectError {}

/// Asks `session` for the object `request` describes, from the route's
/// rung down.
pub async fn request_json_object(
    session: &dyn ModelSession,
    request: ModelJsonObjectRequest,
    cancellation: CancellationToken,
) -> Result<ModelJsonObjectAnswer, ModelJsonObjectError> {
    if cancellation.is_cancelled() {
        return Err(ModelJsonObjectError::Cancelled);
    }
    let monitor = cancellation.clone();
    let timeout = request.timeout;
    let chain = walk_rungs(session, &request, &cancellation);
    tokio::pin!(chain);
    tokio::select! {
        result = &mut chain => result,
        _ = tokio::time::sleep(timeout) => Err(ModelJsonObjectError::TimedOut),
        _ = wait_for_cancellation(monitor) => Err(ModelJsonObjectError::Cancelled),
    }
}

/// The object in `text`, an answer that should be only that object: a text
/// that starts with `{` is read from there (a fence inside its strings is
/// content); otherwise a fenced block's content, else the first balanced
/// `{…}`, else the whole text, trimmed.
pub fn json_object_text(text: &str) -> &str {
    let text = text.trim();
    if text.starts_with('{') {
        return balanced_object(text).unwrap_or(text);
    }
    fenced_block(text)
        .or_else(|| balanced_object(text))
        .unwrap_or(text)
        .trim()
}

async fn walk_rungs(
    session: &dyn ModelSession,
    request: &ModelJsonObjectRequest,
    cancellation: &CancellationToken,
) -> Result<ModelJsonObjectAnswer, ModelJsonObjectError> {
    let mut rung = usable(
        session.structured_output_capability_for(&request.model),
        session,
        request,
    );
    loop {
        match ask(session, request, rung, cancellation).await {
            Err(Attempt::Refused(error)) => {
                let Some(lower) = lower_rung(rung).map(|lower| usable(lower, session, request))
                else {
                    return Err(ModelJsonObjectError::Failed(error.to_string()));
                };
                log::debug!(
                    "structured {} request: the route refused the {rung:?} rung; retrying at {lower:?}",
                    request.subject
                );
                session.learn_structured_output(&request.model, lower);
                rung = lower;
            }
            Err(Attempt::Failed(error)) => return Err(error),
            Ok(answer) => return Ok(answer),
        }
    }
}

/// How one rung's request ended, when it did not answer.
enum Attempt {
    /// The Provider refused the rung: the next one may answer.
    Refused(ModelError),
    Failed(ModelJsonObjectError),
}

impl From<ModelJsonObjectError> for Attempt {
    fn from(error: ModelJsonObjectError) -> Self {
        Self::Failed(error)
    }
}

async fn ask(
    session: &dyn ModelSession,
    request: &ModelJsonObjectRequest,
    rung: ModelStructuredOutputCapability,
    cancellation: &CancellationToken,
) -> Result<ModelJsonObjectAnswer, Attempt> {
    let subject = request.subject;
    let tool_name = request.tool.as_ref().map(|tool| tool.name);
    let mut stream = session
        .stream(model_request(request, rung)?, cancellation.clone())
        .await
        .map_err(|error| attempt_error(error, rung, cancellation))?;
    let mut message = None;
    let mut arguments = None;
    let mut usage = None;
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event.map_err(|error| attempt_error(error, rung, cancellation))? {
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::AssistantMessage { text, .. },
            } if !text.trim().is_empty() => {
                if message.replace(text).is_some() {
                    return Err(failed(format!(
                        "model returned duplicate {subject} results"
                    )));
                }
            }
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall { call },
            } => {
                if rung != ModelStructuredOutputCapability::RequiredTool
                    || tool_name != Some(call.name.as_str())
                {
                    return Err(failed(format!(
                        "model returned an unexpected tool call for the {subject}"
                    )));
                }
                if arguments.replace(call.arguments.to_string()).is_some() {
                    return Err(failed(format!(
                        "model returned duplicate {subject} results"
                    )));
                }
            }
            ModelEvent::ResponseCompleted { usage: value, .. } => {
                completed = true;
                usage = value;
            }
            _ => {}
        }
    }
    if cancellation.is_cancelled() {
        return Err(ModelJsonObjectError::Cancelled.into());
    }
    if !completed {
        return Err(failed(format!(
            "model stream ended before {subject} completion"
        )));
    }
    // A forced tool answered with text (a gateway that ignores
    // `tool_choice`) is read as the text rung's answer, and teaches nothing.
    let text = arguments
        .or_else(|| message.map(|message| message.trim().to_string()))
        .ok_or_else(|| failed(format!("model returned no {subject} result")))?;
    Ok(ModelJsonObjectAnswer { text, rung, usage })
}

fn model_request(
    request: &ModelJsonObjectRequest,
    rung: ModelStructuredOutputCapability,
) -> Result<ModelRequest, Attempt> {
    let mut model_request = ModelRequest {
        model: request.model.clone(),
        instructions: request.instructions.clone(),
        input: vec![ModelInputItem::user_text(request.input.clone())],
        tools: Vec::new(),
        tool_choice: ModelToolChoice::None,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    };
    match (rung, &request.tool) {
        (ModelStructuredOutputCapability::NativeJsonSchema, _) => {
            let output = ModelJsonSchemaOutput {
                name: request.output_name.to_string(),
                description: Some(request.output_description.to_string()),
                schema: request.schema.clone(),
                strict: true,
            };
            output.validate().map_err(failed)?;
            model_request.output_format = Some(ModelOutputFormat::JsonSchema(output));
        }
        (ModelStructuredOutputCapability::RequiredTool, Some(tool)) => {
            let spec = ModelToolSpec {
                name: tool.name.to_string(),
                description: tool.description.to_string(),
                input_schema: request.schema.clone(),
                strict: true,
            }
            .try_into()
            .map_err(
                |error: hanafubuki_agent_tool_contracts::ToolContractError| {
                    failed(error.to_string())
                },
            )?;
            model_request.tools = vec![spec];
            model_request.tool_choice = ModelToolChoice::Required {
                name: tool.name.to_string(),
            };
        }
        _ if request.schema_in_text => {
            model_request.instructions = format!(
                "{}\n\nAnswer with only a JSON object that matches this JSON schema, with no other text: {}",
                request.instructions, request.schema
            );
        }
        _ => {}
    }
    Ok(model_request)
}

/// `rung`, or the text rung for a forced tool the caller or the session
/// cannot use.
fn usable(
    rung: ModelStructuredOutputCapability,
    session: &dyn ModelSession,
    request: &ModelJsonObjectRequest,
) -> ModelStructuredOutputCapability {
    let capabilities = session.tool_capabilities();
    let forced_tool = request.tool.is_some()
        && capabilities.function_tools
        && capabilities.required_named_tool_choice;
    match rung {
        ModelStructuredOutputCapability::RequiredTool if !forced_tool => {
            ModelStructuredOutputCapability::Text
        }
        rung => rung,
    }
}

fn lower_rung(rung: ModelStructuredOutputCapability) -> Option<ModelStructuredOutputCapability> {
    match rung {
        ModelStructuredOutputCapability::NativeJsonSchema => {
            Some(ModelStructuredOutputCapability::RequiredTool)
        }
        ModelStructuredOutputCapability::RequiredTool => {
            Some(ModelStructuredOutputCapability::Text)
        }
        ModelStructuredOutputCapability::Text => None,
    }
}

fn attempt_error(
    error: ModelError,
    rung: ModelStructuredOutputCapability,
    cancellation: &CancellationToken,
) -> Attempt {
    if cancellation.is_cancelled() {
        return ModelJsonObjectError::Cancelled.into();
    }
    match (rung, &error) {
        (
            ModelStructuredOutputCapability::NativeJsonSchema,
            ModelError::UnsupportedStructuredOutput { .. },
        )
        | (
            ModelStructuredOutputCapability::RequiredTool,
            ModelError::UnsupportedToolChoice { .. },
        ) => Attempt::Refused(error),
        _ => failed(error.to_string()),
    }
}

fn failed(message: impl Into<String>) -> Attempt {
    Attempt::Failed(ModelJsonObjectError::Failed(message.into()))
}

async fn wait_for_cancellation(cancellation: CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// The content of the first fenced block (```` ``` ````, its info string
/// skipped), when it is closed.
fn fenced_block(text: &str) -> Option<&str> {
    let start = text.find("```")?;
    let after_fence = &text[start + 3..];
    let content = &after_fence[after_fence.find('\n')? + 1..];
    Some(&content[..content.find("```")?])
}

/// The first `{` and the `}` that closes it, strings and escapes skipped.
fn balanced_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in text[start..].char_indices() {
        if in_string {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
