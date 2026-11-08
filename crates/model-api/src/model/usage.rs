use serde::{Deserialize, Serialize};

/// Bounded response headers that are useful for diagnostics and scheduling.
/// Response metadata and remote errors retain their original values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitMetadata {
    pub limit_requests: Option<String>,
    pub remaining_requests: Option<String>,
    pub reset_requests: Option<String>,
    pub limit_tokens: Option<String>,
    pub remaining_tokens: Option<String>,
    pub reset_tokens: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponseMetadata {
    pub request_id: Option<String>,
    pub rate_limit: RateLimitMetadata,
}
