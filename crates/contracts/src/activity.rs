use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// What an activity is doing, named by the tool or kernel step that runs it.
///
/// An open identifier rather than a closed list: a tool names its own action,
/// and Vue labels the names it knows and shows a generic tool label for the
/// rest. Retained sessions keep the names they recorded; a recorded value that
/// is not an identifier reads back as [`AgentActivityAction::UNKNOWN`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct AgentActivityAction(Cow<'static, str>);

impl AgentActivityAction {
    pub const GENERATING_RESPONSE: Self = Self::fixed("generating_response");
    pub const RETRYING_MODEL_REQUEST: Self = Self::fixed("retrying_model_request");
    pub const WAITING_FOR_APPROVAL: Self = Self::fixed("waiting_for_approval");
    pub const CHECKING_CONTINUATION: Self = Self::fixed("checking_continuation");
    pub const UPDATE_PLAN: Self = Self::fixed("update_plan");
    pub const REQUEST_USER_INPUT: Self = Self::fixed("request_user_input");
    /// A tool that names no action of its own.
    pub const USE_TOOL: Self = Self::fixed("use_tool");
    pub const UNKNOWN: Self = Self::fixed("unknown");

    pub const MAX_BYTES: usize = 64;

    /// A compile-time action name; it must satisfy [`Self::is_valid`]
    /// (checked by the tool registry when a tool registers).
    pub const fn fixed(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::is_valid(name).then(|| Self(Cow::Owned(name.to_owned())))
    }

    /// Lower-case ASCII letters, digits and `_`, starting with a letter.
    pub fn is_valid(name: &str) -> bool {
        name.len() <= Self::MAX_BYTES
            && name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentActivityAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AgentActivityAction {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Self::parse(&name).unwrap_or(Self::UNKNOWN))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivityPresentation {
    pub action: AgentActivityAction,
    pub subject: Option<String>,
}

pub type AgentToolPresentation = AgentActivityPresentation;
