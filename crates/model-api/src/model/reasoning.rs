use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use serde::{Deserialize, Serialize};

const MAX_OPAQUE_COMPACTION_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReasoningConfig {
    pub effort: ModelReasoningEffort,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ModelOpaqueCompactionPayload(String);

impl ModelOpaqueCompactionPayload {
    pub fn parse(value: String) -> Result<Self, String> {
        if value.is_empty() || value.len() > MAX_OPAQUE_COMPACTION_PAYLOAD_BYTES {
            return Err(format!(
                "opaque compaction payload must contain 1 to {MAX_OPAQUE_COMPACTION_PAYLOAD_BYTES} bytes"
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ModelOpaqueCompactionPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ModelOpaqueCompactionPayload([redacted])")
    }
}

impl<'de> Deserialize<'de> for ModelOpaqueCompactionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCompactionCapability {
    #[default]
    LocalSummary,
    ResponsesCompact,
    ResponsesCompactV2,
}
