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
    pub summary_tools: Vec<String>,
    pub excluded_tools: Vec<String>,
    pub executor_models: Vec<String>,
    pub share_images: bool,
    pub redact_secrets: bool,
    pub git_context: super::workspace::GitContext,
    pub project_rules: bool,
    pub tracked_files: bool,
    pub untracked_files: bool,
    pub git_chars: usize,
    pub simple: bool,
    pub review_stages: Vec<String>,
    pub consultation_limit: Option<usize>,
    pub record_outcomes: bool,
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
            summary_tools: Vec::new(),
            excluded_tools: Vec::new(),
            executor_models: Vec::new(),
            share_images: false,
            redact_secrets: false,
            git_context: Default::default(),
            project_rules: false,
            tracked_files: false,
            untracked_files: false,
            git_chars: 20_000,
            simple: false,
            review_stages: vec!["plan".into(), "failure".into(), "completion".into()],
            consultation_limit: None,
            record_outcomes: false,
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
        (
            "summaryTools",
            "Share status only for these tools",
            SettingKind::TextList {
                maximum_items: None,
                maximum_item_chars: None,
            },
        ),
        (
            "excludedTools",
            "Exclude these tools from Advisor history",
            SettingKind::TextList {
                maximum_items: None,
                maximum_item_chars: None,
            },
        ),
        (
            "executorModels",
            "Allowed executor provider/model IDs (empty allows all)",
            SettingKind::TextList {
                maximum_items: None,
                maximum_item_chars: None,
            },
        ),
        (
            "shareImages",
            "Share images in selected conversation evidence",
            SettingKind::Boolean,
        ),
        (
            "redactSecrets",
            "Redact common credential patterns before sending text",
            SettingKind::Boolean,
        ),
        (
            "gitContext",
            "Maximum Git evidence",
            SettingKind::Choice {
                values: ["none", "summary", "full"]
                    .map(|v| SettingChoice::new(v, v))
                    .into(),
            },
        ),
        (
            "projectRules",
            "Share trusted project rules and preferences",
            SettingKind::Boolean,
        ),
        (
            "trackedFiles",
            "Allow a named tracked file requested by the Advisor",
            SettingKind::Boolean,
        ),
        (
            "untrackedFiles",
            "Allow explicitly named untracked files",
            SettingKind::Boolean,
        ),
        (
            "gitChars",
            "Maximum Git evidence characters",
            SettingKind::Integer {
                minimum: Some(0),
                maximum: None,
            },
        ),
        (
            "simple",
            "Offer voluntary consultations only",
            SettingKind::Boolean,
        ),
        (
            "reviewStages",
            "Review guidance (plan, failure, completion or custom instructions)",
            SettingKind::TextList {
                maximum_items: None,
                maximum_item_chars: None,
            },
        ),
        (
            "consultationLimit",
            "Session consultation budget (reset for unlimited)",
            SettingKind::Integer {
                minimum: Some(0),
                maximum: None,
            },
        ),
        (
            "recordOutcomes",
            "Record adoption and verification outcomes locally",
            SettingKind::Boolean,
        ),
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
