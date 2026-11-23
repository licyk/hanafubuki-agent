use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use reqwest::Url;

use hanafubuki_agent_foundation::agent_settings::{
    parse_model_request_header, validate_model_secret_value,
};

use crate::{ModelApiProtocol, ModelError, ModelHttpProxy};

const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024;
const MAX_REQUEST_ATTEMPTS: u32 = 10;
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq, Eq)]
pub struct ModelCredential(Arc<str>);

impl std::fmt::Debug for ModelCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("ModelCredential")
            .field(&"[redacted]")
            .finish()
    }
}

impl ModelCredential {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_model_secret_value("credential", &value).map_err(ModelError::InvalidRequest)?;
        Ok(Self(Arc::from(value)))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ModelHeader {
    name: HeaderName,
    value: HeaderValue,
    raw_value: Arc<str>,
}

impl std::fmt::Debug for ModelHeader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelHeader")
            .field("name", &self.name)
            .field("value", &"[redacted]")
            .finish()
    }
}

impl ModelHeader {
    pub fn new(name: &str, value: impl Into<String>) -> Result<Self, ModelError> {
        let raw_value = value.into();
        let (name, value) =
            parse_model_request_header(name, &raw_value).map_err(ModelError::InvalidRequest)?;
        Ok(Self {
            name,
            value,
            raw_value: Arc::from(raw_value),
        })
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub(crate) fn parts(&self) -> (HeaderName, HeaderValue) {
        (self.name.clone(), self.value.clone())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ModelEndpoint {
    base_url: Url,
}

impl std::fmt::Debug for ModelEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut endpoint = self.base_url.clone();
        if endpoint.query().is_some() {
            endpoint.set_query(Some("[redacted]"));
        }
        formatter
            .debug_struct("ModelEndpoint")
            .field("base_url", &endpoint)
            .finish()
    }
}

impl ModelEndpoint {
    /// Parses a Provider base URL the settings already validated
    /// (`validate_agent_provider_base_url`); it is not checked again here.
    pub fn parse(value: &str) -> Result<Self, ModelError> {
        let mut base_url = Url::parse(value)
            .map_err(|_| ModelError::InvalidRequest("invalid model provider base URL".into()))?;
        let normalized_path = format!("{}/", base_url.path().trim_end_matches('/'));
        base_url.set_path(&normalized_path);
        Ok(Self { base_url })
    }

    /// The base URL's host, when it has one.
    pub fn host(&self) -> Option<&str> {
        self.base_url.host_str()
    }

    pub(crate) fn responses_url(&self) -> Result<Url, ModelError> {
        self.join_endpoint("responses", "Responses")
    }

    pub(crate) fn responses_compact_url(&self) -> Result<Url, ModelError> {
        self.join_endpoint("responses/compact", "Responses compact")
    }

    pub(crate) fn chat_completions_url(&self) -> Result<Url, ModelError> {
        self.join_endpoint("chat/completions", "Chat Completions")
    }

    pub(crate) fn anthropic_messages_url(&self) -> Result<Url, ModelError> {
        self.join_endpoint("messages", "Anthropic Messages")
    }

    pub(crate) fn generate_content_url(&self, model: &str) -> Result<Url, ModelError> {
        let model = ModelApiProtocol::GeminiGenerateContent.model_id(model);
        let mut endpoint = self.models_url()?;
        endpoint
            .path_segments_mut()
            .map_err(|_| ModelError::InvalidRequest("invalid Gemini model endpoint".into()))?
            .push(&format!("{model}:streamGenerateContent"));
        set_query_parameter(&mut endpoint, "alt", "sse");
        Ok(endpoint)
    }

    pub(crate) fn interactions_url(&self) -> Result<Url, ModelError> {
        self.join_endpoint("interactions", "Gemini Interactions")
    }

    pub(crate) fn models_url(&self) -> Result<Url, ModelError> {
        self.join_endpoint("models", "models")
    }

    pub(crate) fn canonical_identity(&self) -> &str {
        self.base_url.as_str()
    }

    fn join_endpoint(&self, path: &str, label: &str) -> Result<Url, ModelError> {
        let query = self.base_url.query().map(str::to_owned);
        let mut base = self.base_url.clone();
        base.set_query(None);
        let mut endpoint = base
            .join(path)
            .map_err(|_| ModelError::InvalidRequest(format!("invalid {label} endpoint")))?;
        endpoint.set_query(query.as_deref());
        Ok(endpoint)
    }
}

pub(crate) fn set_query_parameter(endpoint: &mut Url, name: &str, value: &str) {
    let retained = endpoint
        .query_pairs()
        .filter(|(current, _)| current != name)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    endpoint.set_query(None);
    let mut query = endpoint.query_pairs_mut();
    for (name, value) in retained {
        query.append_pair(&name, &value);
    }
    query.append_pair(name, value);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelRetryPolicy {
    pub maximum_attempts: u32,
    pub initial_backoff: Duration,
    pub maximum_backoff: Duration,
}

impl ModelRetryPolicy {
    pub fn new(
        maximum_attempts: u32,
        initial_backoff: Duration,
        maximum_backoff: Duration,
    ) -> Result<Self, ModelError> {
        if maximum_attempts == 0
            || maximum_attempts > MAX_REQUEST_ATTEMPTS
            || initial_backoff.is_zero()
            || maximum_backoff < initial_backoff
            || maximum_backoff > MAX_RETRY_BACKOFF
        {
            return Err(ModelError::InvalidRequest(
                "invalid model request retry policy".into(),
            ));
        }
        Ok(Self {
            maximum_attempts,
            initial_backoff,
            maximum_backoff,
        })
    }
}

impl Default for ModelRetryPolicy {
    fn default() -> Self {
        Self {
            maximum_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            maximum_backoff: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelTimeouts {
    pub connect: Duration,
    pub request_establishment: Duration,
    pub stream_idle: Duration,
}

impl ModelTimeouts {
    pub fn new(
        connect: Duration,
        request_establishment: Duration,
        stream_idle: Duration,
    ) -> Result<Self, ModelError> {
        if connect.is_zero() || request_establishment.is_zero() || stream_idle.is_zero() {
            return Err(ModelError::InvalidRequest(
                "model transport timeouts must be positive".into(),
            ));
        }
        Ok(Self {
            connect,
            request_establishment,
            stream_idle,
        })
    }
}

impl Default for ModelTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request_establishment: Duration::from_secs(30),
            stream_idle: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelProvider {
    endpoint: ModelEndpoint,
    credential: Option<ModelCredential>,
    headers: Vec<ModelHeader>,
    retry_policy: ModelRetryPolicy,
    timeouts: ModelTimeouts,
    proxy: ModelHttpProxy,
}

impl ModelProvider {
    pub fn new(endpoint: ModelEndpoint, credential: Option<ModelCredential>) -> Self {
        Self {
            endpoint,
            credential,
            headers: Vec::new(),
            retry_policy: ModelRetryPolicy::default(),
            timeouts: ModelTimeouts::default(),
            proxy: ModelHttpProxy::default(),
        }
    }

    /// The proxy its requests go through (the system's by default).
    pub fn with_proxy(mut self, proxy: ModelHttpProxy) -> Self {
        self.proxy = proxy;
        self
    }

    pub fn proxy(&self) -> &ModelHttpProxy {
        &self.proxy
    }

    pub fn with_header(mut self, header: ModelHeader) -> Self {
        self.headers.push(header);
        self
    }

    pub fn with_retry_policy(mut self, retry_policy: ModelRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub fn with_timeouts(mut self, timeouts: ModelTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub fn endpoint(&self) -> &ModelEndpoint {
        &self.endpoint
    }

    pub fn retry_policy(&self) -> ModelRetryPolicy {
        self.retry_policy
    }

    pub fn timeouts(&self) -> ModelTimeouts {
        self.timeouts
    }

    pub(crate) fn credential(&self) -> Option<&ModelCredential> {
        self.credential.as_ref()
    }

    pub(crate) fn headers(&self) -> &[ModelHeader] {
        &self.headers
    }

    pub(crate) fn body_read_error(&self, body: &str, error: reqwest::Error) -> ModelError {
        // I/O kinds describe the failure without exposing URLs or peer-supplied text.
        let io_kind =
            std::iter::successors(std::error::Error::source(&error), |cause| cause.source())
                .find_map(|cause| {
                    cause
                        .downcast_ref::<std::io::Error>()
                        .map(std::io::Error::kind)
                });
        let detail = if error.is_timeout() {
            "read timed out".to_owned()
        } else if let Some(kind) = io_kind {
            kind.to_string()
        } else {
            error.without_url().to_string()
        };
        ModelError::StreamInterrupted(
            self.bound_diagnostic(format!("could not read {body}: {detail}")),
        )
    }

    pub(crate) fn bound_diagnostic(&self, value: impl Into<String>) -> String {
        let mut value = value.into();
        if let Some(credential) = &self.credential {
            value = value.replace(credential.expose(), "[redacted]");
        }
        for header in &self.headers {
            value = value.replace(header.raw_value.as_ref(), "[redacted]");
        }
        truncate_diagnostic(value)
    }
}

pub(crate) fn truncate_diagnostic(mut value: String) -> String {
    if value.len() <= MAX_DIAGNOSTIC_BYTES {
        return value;
    }
    let mut boundary = MAX_DIAGNOSTIC_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value.push('…');
    value
}
