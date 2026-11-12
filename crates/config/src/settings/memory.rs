//! `[memory]`: whether the Agent remembers across sessions, and how it
//! reads them.

use hanafubuki_agent_foundation::settings::{SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

use crate::sections::{described, SettingsSection};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMemorySettings {
    pub enabled: bool,
    pub use_memories: bool,
    pub generate_memories: bool,
    pub exclude_external_context: bool,
    pub extraction_model: Option<String>,
    pub consolidation_model: Option<String>,
    pub max_source_age_days: u32,
    pub minimum_idle_hours: u32,
    pub max_sessions_per_pass: usize,
    pub max_stage1_outputs: usize,
    pub max_unused_days: u32,
}

impl Default for AgentMemorySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            use_memories: true,
            generate_memories: true,
            exclude_external_context: false,
            extraction_model: None,
            consolidation_model: None,
            max_source_age_days: 10,
            minimum_idle_hours: 6,
            max_sessions_per_pass: 2,
            max_stage1_outputs: 256,
            max_unused_days: 30,
        }
    }
}

/// The longest model name, in UTF-8 bytes.
const MAX_MODEL_NAME_BYTES: usize = 256;

pub(crate) fn section() -> SettingsSection {
    let model = || SettingKind::Text {
        maximum_chars: Some(MAX_MODEL_NAME_BYTES as u32),
        secret: false,
        nullable: true,
    };
    let count = |minimum, maximum| SettingKind::Integer {
        minimum: Some(minimum),
        maximum: Some(maximum),
    };
    let advanced = |spec: SettingSpec| SettingSpec {
        advanced: true,
        ..spec
    };
    let boolean = || SettingKind::Boolean;
    SettingsSection::new(
        "memory",
        "Memory",
        vec![
            SettingSpec {
                keywords: vec!["memories".into(), "remember".into()],
                ..described(
                    SettingSpec::new("memory.enabled", "Memory", boolean()),
                    "Whether memory is on",
                )
            },
            described(
                SettingSpec::new("memory.useMemories", "Use memories", boolean()),
                "Whether the model may use the memories",
            ),
            described(
                SettingSpec::new("memory.generateMemories", "Generate memories", boolean()),
                "Whether new memories are extracted from sessions",
            ),
            described(
                SettingSpec::new(
                    "memory.excludeExternalContext",
                    "Exclude external context",
                    boolean(),
                ),
                "A session stops contributing once it has used a web or MCP tool's output",
            ),
            advanced(described(
                SettingSpec::new("memory.extractionModel", "Extraction model", model()),
                "The model that extracts memories; unset, the model policy's",
            )),
            advanced(described(
                SettingSpec::new("memory.consolidationModel", "Consolidation model", model()),
                "The model that consolidates them; unset, the model policy's",
            )),
            advanced(described(
                SettingSpec::new(
                    "memory.maxSourceAgeDays",
                    "Maximum source age (days)",
                    count(1, 3650),
                ),
                "Only sessions of the last that many days are read",
            )),
            advanced(described(
                SettingSpec::new(
                    "memory.minimumIdleHours",
                    "Minimum idle time (hours)",
                    count(0, 8760),
                ),
                "A session is read once it has been idle that many hours",
            )),
            advanced(described(
                SettingSpec::new(
                    "memory.maxSessionsPerPass",
                    "Sessions per pass",
                    count(1, 64),
                ),
                "At most that many sessions are read per pass",
            )),
            advanced(described(
                SettingSpec::new(
                    "memory.maxStage1Outputs",
                    "Extractions consolidated",
                    count(1, 4096),
                ),
                "At most that many extractions are consolidated",
            )),
            advanced(described(
                SettingSpec::new(
                    "memory.maxUnusedDays",
                    "Maximum unused age (days)",
                    count(1, 3650),
                ),
                "An extraction unused for that many days expires",
            )),
        ],
    )
}

/// What the specs do not say of the memory settings: a model name is not
/// blank and fits [`MAX_MODEL_NAME_BYTES`].
pub(crate) fn validate_memory(settings: &AgentMemorySettings) -> Result<(), String> {
    for (field, value) in [
        ("extractionModel", settings.extraction_model.as_deref()),
        (
            "consolidationModel",
            settings.consolidation_model.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty() || value.len() > MAX_MODEL_NAME_BYTES)
        {
            return Err(format!(
                "memory.{field} must contain 1 to {MAX_MODEL_NAME_BYTES} bytes or be null"
            ));
        }
    }
    Ok(())
}
