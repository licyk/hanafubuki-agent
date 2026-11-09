// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/lib.rs` (the event names and their key labels) and
// `codex-rs/hooks/src/events/common.rs` (which events have matchers) at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: only the events the Agent raises are
// events; Codex's others are named so a configuration that has them is
// told why they never run.

use serde::Serialize;

/// An event the Agent raises hooks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum HookEventName {
    PreToolUse,
    PostToolUse,
    SessionStart,
    UserPromptSubmit,
    Stop,
}

impl HookEventName {
    pub const ALL: [Self; 5] = [
        Self::PreToolUse,
        Self::PostToolUse,
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::Stop,
    ];

    /// The name in `hooks.json` and `[hooks]`.
    pub fn name(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::Stop => "Stop",
        }
    }

    /// The label in a hook's key and in the identity its hash is over.
    pub fn label(self) -> &'static str {
        match self {
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::SessionStart => "session_start",
            Self::UserPromptSubmit => "user_prompt_submit",
            Self::Stop => "stop",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.name() == name)
    }

    /// Whether a group's `matcher` selects anything: the tool name, or the
    /// session start's source. The other events ignore it.
    pub(crate) fn has_matcher(self) -> bool {
        matches!(
            self,
            Self::PreToolUse | Self::PostToolUse | Self::SessionStart
        )
    }
}

/// Codex's events the Agent does not raise, with why: a hook under one is
/// reported and never runs.
pub(crate) fn unraised_event_reason(name: &str) -> Option<&'static str> {
    Some(match name {
        "PreModelRequest" | "PostModelRequest" => "the Agent has no hook around a model request",
        "PermissionRequest" => {
            "approval rules decide without running anything; use PreToolUse to deny a call"
        }
        "PreCompact" | "PostCompact" => "the Agent has no hook around a compaction",
        "SessionEnd" => "the Agent does not tell plugins that a session ended",
        "SubagentStart" | "SubagentStop" => "the Agent has no subagents",
        "Interrupt" => "the Agent does not tell plugins why a turn was cancelled",
        _ => return None,
    })
}
