use hanafubuki_agent_config::{PluginOptionSchema, SettingChoice, SettingKind, SettingSpec};
use hanafubuki_agent_kernel::ModelReasoningEffort;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) enum ActivationScope {
    #[default]
    Persistent,
    Session,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct AdvisorOptions {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub fallback_provider: Option<String>,
    pub fallback_model: Option<String>,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    pub skip_same_model: bool,
    pub history_chars: usize,
    pub activation_scope: ActivationScope,
    pub active: bool,
}

impl Default for AdvisorOptions {
    fn default() -> Self {
        Self {
            provider: None,
            model: None,
            fallback_provider: None,
            fallback_model: None,
            reasoning_effort: None,
            skip_same_model: true,
            history_chars: 15_000,
            activation_scope: ActivationScope::Persistent,
            active: false,
        }
    }
}

impl AdvisorOptions {
    pub(super) fn primary(&self) -> Option<(&str, &str)> {
        self.provider
            .as_deref()
            .filter(|id| !id.is_empty())
            .zip(self.model.as_deref().filter(|id| !id.is_empty()))
    }
}

pub(crate) fn advisor_schema() -> PluginOptionSchema {
    let text = || SettingKind::Text {
        maximum_chars: None,
        secret: false,
        nullable: true,
    };
    let defaults = serde_json::json!(AdvisorOptions::default());
    let options = [
        ("provider", "Advisor provider ID", text()),
        ("model", "Advisor model ID", text()),
        ("fallbackProvider", "Fallback provider ID", text()),
        ("fallbackModel", "Fallback model ID", text()),
        ("reasoningEffort", "Advisor reasoning effort", text()),
        (
            "skipSameModel",
            "Skip consultations on the executor's model",
            SettingKind::Boolean,
        ),
        (
            "historyChars",
            "Conversation characters to share",
            SettingKind::Integer {
                minimum: Some(0),
                maximum: None,
            },
        ),
        (
            "activationScope",
            "Activation scope",
            SettingKind::Choice {
                values: vec![
                    SettingChoice::new("persistent", "Persistent"),
                    SettingChoice::new("session", "Session"),
                ],
            },
        ),
        (
            "active",
            "Activate Advisor in persistent mode",
            SettingKind::Boolean,
        ),
    ]
    .into_iter()
    .map(|(key, label, kind)| {
        let mut spec = SettingSpec::new(key, label, kind);
        spec.default = spec.check(&defaults[key]).ok();
        spec
    })
    .collect();
    PluginOptionSchema { options }
}
