// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/types.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: a message carries the host's own fields
// (image handles, a host protection flag) instead of wire sidecars, and no
// ledger sequence or timestamp: a message's time is the state's first-sight
// stamp, a [`LocalTime`] with the offset it was read in.

mod config;
mod opt_in;
mod state;

use serde::{Deserialize, Serialize};

pub use config::{
    CompressLimits, Config, NudgeConfig, TierConfig, TierGrowthTokens, TruncateConfig,
};
pub use opt_in::{
    AbsorbConfig, AbsorbRecord, ContentStoreConfig, CrushConfig, CrushStrategyOverride,
    MessageFiltersConfig, RuleRecord, RulesConfig,
};
pub use state::{
    Block, Generation, NudgeState, RefMap, State, Stats, TruncateLatchEntry, ViewLatch,
};

/// A reading of the host's clock: milliseconds since the Unix epoch and the
/// local time's offset from UTC when it was read. A message's stamp keeps
/// the offset it was made in, so its rendered time never changes when the
/// host's offset does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalTime {
    pub at: u64,
    pub utc_offset_seconds: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

impl Role {
    /// The kernel's name for the role, as its texts print it.
    pub fn name(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
        }
    }
}

/// What a message holds; one message per content part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// Its text is the call's JSON argument text.
    ToolCall,
    ToolResult,
    Reasoning,
}

/// One part of the host's history, in history order.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// The host's stable id: refs and blocks record it, so it must not change
    /// across requests or restarts. Ids beginning `acp_summary_` are the
    /// crate's own rendered summaries.
    pub id: String,
    pub role: Role,
    pub kind: Kind,
    pub text: String,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    /// Handles the host can reopen the message's images by.
    pub images: Vec<String>,
    /// The host never lets this message fold.
    pub protected: bool,
}

impl Message {
    pub fn text(id: impl Into<String>, role: Role, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role,
            kind: Kind::Text,
            text: text.into(),
            tool_name: None,
            tool_call_id: None,
            images: Vec::new(),
            protected: false,
        }
    }

    pub fn tool_call(
        id: impl Into<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            kind: Kind::ToolCall,
            tool_name: Some(tool_name.into()),
            tool_call_id: Some(call_id.into()),
            ..Self::text(id, Role::Assistant, arguments)
        }
    }

    pub fn tool_result(
        id: impl Into<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            kind: Kind::ToolResult,
            tool_name: Some(tool_name.into()),
            tool_call_id: Some(call_id.into()),
            ..Self::text(id, Role::Tool, output)
        }
    }

    pub fn reasoning(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            kind: Kind::Reasoning,
            ..Self::text(id, Role::Assistant, text)
        }
    }

    pub fn is_tool(&self) -> bool {
        matches!(self.kind, Kind::ToolCall | Kind::ToolResult)
    }

    pub(crate) fn tool_name_is(&self, name: &str) -> bool {
        self.tool_name.as_deref() == Some(name)
    }

    /// An assistant text or tool call: the act a reasoning run belongs to.
    pub(crate) fn is_assistant_act(&self) -> bool {
        self.role == Role::Assistant && matches!(self.kind, Kind::Text | Kind::ToolCall)
    }
}
