// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the `absorb` and `acp_retrieve` tools of `kernel/src/compress-tools.ts` of
// acp-kernel 0.0.189 and the `acp_rule` tool of `src/compress-tool.ts`, at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: one wire-neutral schema per tool; the
// kernel ships no `acp_rule` schema, so its host's is taken verbatim.

use serde_json::json;

use super::tools::ToolSchema;

pub const ABSORB_TOOL_NAME: &str = "absorb";
pub const RETRIEVE_TOOL_NAME: &str = "acp_retrieve";
pub const RULE_TOOL_NAME: &str = "acp_rule";

pub const ABSORB_TOOL_DESCRIPTION: &str = r##"Distill a tool result into a compact summary you write. REQUIRED immediately after a tool result ends with an [ACP absorb] instruction: pass its ref and the distilled essentials (outcome, key values, paths:lines, errors, decisions). The original output is then removed from context; your summary is the durable record. If nothing worth keeping survives, say so in the summary (e.g. '(nothing needed)') — that discards the output cleanly."##;

pub const RETRIEVE_TOOL_DESCRIPTION: &str = r##"Retrieve the full original text of a stored tool result. REQUIRED when you need details from a placeholder that starts with 📦 [acp-stored #mNNNNN: pass its ref. Returns the exact original text inline — very large originals are exported to a file instead, and the result then carries the exported file's path. The placeholder stays in context. Unknown refs return not-found."##;

pub const RULE_TOOL_DESCRIPTION: &str = r##"Record a short, principle-level reminder so it survives context compression — the call and its result are protected and stay in context. Record when: the user calls out or repeatedly emphasizes a lesson; the user asks you to remember or follow a behavior; you personally hit a major pitfall worth remembering long-term. Keep each rule to one short line. Omit the rule argument to list recorded rules. To remove a recorded rule pass delete with its id (e.g. "rule3"); to remove every recorded rule pass clear: true. delete and clear are mutually exclusive with each other and with rule — use one operation per call."##;

/// `absorb`, offered only while absorption is on.
pub fn absorb_tool() -> ToolSchema {
    ToolSchema {
        name: ABSORB_TOOL_NAME,
        description: ABSORB_TOOL_DESCRIPTION,
        parameters: json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "mNNNNN ref of the tool result to absorb (from the [ACP absorb] instruction)",
                },
                "summary": {
                    "type": "string",
                    "description": "Distilled essentials of the tool result — this replaces the original output in context",
                },
            },
            "required": ["ref", "summary"],
        }),
    }
}

/// `acp_retrieve`, offered only while the content store is on.
pub fn retrieve_tool() -> ToolSchema {
    ToolSchema {
        name: RETRIEVE_TOOL_NAME,
        description: RETRIEVE_TOOL_DESCRIPTION,
        parameters: json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "mNNNNN ref shown in the 📦 [acp-stored …] placeholder",
                },
            },
            "required": ["ref"],
        }),
    }
}

/// `acp_rule`, offered only while rules are on.
pub fn rule_tool() -> ToolSchema {
    ToolSchema {
        name: RULE_TOOL_NAME,
        description: RULE_TOOL_DESCRIPTION,
        parameters: json!({
            "type": "object",
            "properties": {
                "rule": {
                    "type": "string",
                    "description": "Short principle-level reminder to record. Omit to list recorded rules.",
                },
                "delete": {
                    "type": "string",
                    "description": "Id of a recorded rule to remove (e.g. \"rule3\"). Mutually exclusive with rule and clear.",
                },
                "clear": {
                    "type": "boolean",
                    "description": "Remove every recorded rule at once. Mutually exclusive with rule and delete.",
                },
            },
        }),
    }
}
