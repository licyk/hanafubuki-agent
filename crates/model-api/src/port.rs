use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::model::{
    ModelCompactionCapability, ModelCompactionResult, ModelEvent, ModelRequest,
    ModelStructuredOutputCapability,
};
use crate::ModelRouteKey;
use crate::ToolProviderCapabilities;

pub type ModelFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type ModelResult<T> = Result<T, ModelError>;
pub type ModelStream = Box<dyn ModelEventStream>;

/// Errors exposed by a model transport without leaking credentials or request
/// payloads into diagnostic text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    Cancelled,
    InvalidRequest(String),
    RequestFailed {
        attempts: u32,
        message: String,
    },
    RequestRetriesExhausted {
        attempts: u32,
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    IdleTimeout {
        timeout: Duration,
    },
    StreamInterrupted(String),
    Protocol(String),
    Remote {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    ContextWindowExceeded {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    UnsupportedImageInput {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    /// The Provider refused the request's reasoning parameter.
    UnsupportedReasoning {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("model request cancelled"),
            Self::InvalidRequest(message) => write!(formatter, "invalid model request: {message}"),
            Self::RequestFailed { attempts, message } => write!(
                formatter,
                "model request failed after {attempts} attempt(s): {message}"
            ),
            Self::RequestRetriesExhausted {
                attempts,
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "model request retries exhausted attempts={attempts} status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::IdleTimeout { timeout } => {
                write!(formatter, "model stream idle timeout after {timeout:?}")
            }
            Self::StreamInterrupted(message) => {
                write!(formatter, "model response stream interrupted: {message}")
            }
            Self::Protocol(message) => write!(formatter, "model protocol failed: {message}"),
            Self::Remote {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "model remote error status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::ContextWindowExceeded {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "model context window exceeded status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::UnsupportedImageInput {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "model endpoint does not support image input status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::UnsupportedReasoning {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "model endpoint does not support the reasoning parameter status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
        }
    }
}

impl std::error::Error for ModelError {}

/// Pull-based async event stream used by the Agent loop.
///
/// Implementations must return `None` only after a terminal event or a stream
/// error. They must observe the cancellation token supplied to `ModelPort`.
pub trait ModelEventStream: Send {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>>;
}

/// Provider-neutral model sampling port consumed by `hanafubuki-agent-kernel`.
///
/// Implementations own wire encoding and transport behavior. They must not
/// execute Agent tools or mutate conversation state.
pub trait ModelPort: Send + Sync {
    fn compaction_capability(&self) -> ModelCompactionCapability {
        ModelCompactionCapability::LocalSummary
    }

    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        conservative_tool_capabilities()
    }

    fn structured_output_capability(&self) -> ModelStructuredOutputCapability {
        ModelStructuredOutputCapability::TextOnly
    }

    fn route_key(&self, _model: &str) -> Option<ModelRouteKey> {
        None
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>>;

    fn compact(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        Box::pin(async {
            Err(ModelError::InvalidRequest(
                "model transport does not support remote compaction".to_string(),
            ))
        })
    }
}

pub trait ModelSession: Send + Sync {
    fn compaction_capability(&self) -> ModelCompactionCapability {
        ModelCompactionCapability::LocalSummary
    }

    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        conservative_tool_capabilities()
    }

    fn structured_output_capability(&self) -> ModelStructuredOutputCapability {
        ModelStructuredOutputCapability::TextOnly
    }

    fn route_key(&self, _model: &str) -> Option<ModelRouteKey> {
        None
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>>;

    fn compact(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        Box::pin(async {
            Err(ModelError::InvalidRequest(
                "model session does not support remote compaction".to_string(),
            ))
        })
    }
}

struct SharedModelSession {
    port: Arc<dyn ModelPort>,
}

impl ModelSession for SharedModelSession {
    fn compaction_capability(&self) -> ModelCompactionCapability {
        self.port.compaction_capability()
    }

    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        self.port.tool_capabilities()
    }

    fn structured_output_capability(&self) -> ModelStructuredOutputCapability {
        self.port.structured_output_capability()
    }

    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        self.port.route_key(model)
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.port.stream(request, cancellation)
    }

    fn compact(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        self.port.compact(request, cancellation)
    }
}

fn conservative_tool_capabilities() -> ToolProviderCapabilities {
    ToolProviderCapabilities {
        function_tools: true,
        strict_json_schema: false,
        maximum_tool_name_bytes: 64,
        namespace_tools: false,
        native_tool_search: false,
        freeform_tools: false,
        hosted_web_search: false,
        parallel_tool_calls: false,
        multimodal_tool_output_transport:
            hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::Unsupported,
        required_named_tool_choice: false,
    }
}

pub trait ModelSessionFactory {
    fn create_model_session(&self) -> Arc<dyn ModelSession>;
}

impl ModelSessionFactory for Arc<dyn ModelPort> {
    fn create_model_session(&self) -> Arc<dyn ModelSession> {
        Arc::new(SharedModelSession { port: self.clone() })
    }
}
