// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/prompts.ts` and `buildCompressSystemPrompt` of
// `kernel/src/compress-tools.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the rule texts cannot be overridden and the
// prompt has no section packs; only the language-preservation rule is
// optional, and the time-aware `<acp>` tag section is chosen by a flag (the
// kernel's `FUNCTION_ACP_TAGS_TIMESTAMPED`, which billion-context's host
// swaps in when `compress.messageTimestamps` is on).

mod opt_in;
mod rules;
mod tools;

pub use opt_in::{
    absorb_tool, retrieve_tool, rule_tool, ABSORB_TOOL_DESCRIPTION, ABSORB_TOOL_NAME,
    RETRIEVE_TOOL_DESCRIPTION, RETRIEVE_TOOL_NAME, RULE_TOOL_DESCRIPTION, RULE_TOOL_NAME,
};
pub use rules::{
    COMPRESS_PHILOSOPHY, HOW_TO_COMPRESS_RULES, LANGUAGE_PRESERVATION_RULE, TIER2_DISTILL_RULES,
    TIER3_CONDENSE_RULES,
};
pub use tools::{
    acp_tools, compress_parameters, ToolSchema, ACP_CACHE_TOOL_DESCRIPTION, ACP_CACHE_TOOL_NAME,
    ACP_MUTATING_TOOLS, ACP_READONLY_TOOLS, ACP_STATUS_TOOL_DESCRIPTION, ACP_STATUS_TOOL_NAME,
    ACP_TOOL_NAMES, COMPRESS_TOOL_DESCRIPTION, DECOMPRESS_TOOL_DESCRIPTION, DECOMPRESS_TOOL_NAME,
    SEARCH_CONTEXT_TOOL_DESCRIPTION, SEARCH_CONTEXT_TOOL_NAME,
};

/// The rule texts the system prompt and every nudge carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompts {
    pub compress_philosophy: String,
    pub how_to_compress_rules: String,
    pub tier2_distill_rules: String,
    pub tier3_condense_rules: String,
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            compress_philosophy: COMPRESS_PHILOSOPHY.to_owned(),
            how_to_compress_rules: HOW_TO_COMPRESS_RULES.to_owned(),
            tier2_distill_rules: TIER2_DISTILL_RULES.to_owned(),
            tier3_condense_rules: TIER3_CONDENSE_RULES.to_owned(),
        }
    }
}

impl Prompts {
    /// The default rules with [`LANGUAGE_PRESERVATION_RULE`] appended to
    /// each; without it, the texts are byte-identical to the defaults.
    pub fn with_language_preservation() -> Self {
        Self {
            compress_philosophy: format!("{COMPRESS_PHILOSOPHY}\n- {LANGUAGE_PRESERVATION_RULE}"),
            how_to_compress_rules: format!(
                "{HOW_TO_COMPRESS_RULES}\n\n{LANGUAGE_PRESERVATION_RULE}"
            ),
            tier2_distill_rules: format!("{TIER2_DISTILL_RULES}\n\n{LANGUAGE_PRESERVATION_RULE}"),
            tier3_condense_rules: format!("{TIER3_CONDENSE_RULES}\n\n{LANGUAGE_PRESERVATION_RULE}"),
        }
    }
}

/// The static system prompt for the function tools: the philosophy, how to
/// compress, the `<acp>` tags (with their `time` attribute when
/// `message_timestamps`), the tools and how to read past summaries. It
/// never changes within a session, so it stays in the cached prefix.
pub fn build_compress_system_prompt(prompts: &Prompts, message_timestamps: bool) -> String {
    let tags = if message_timestamps {
        tools::ACP_TAGS_SECTION_TIMESTAMPED
    } else {
        tools::ACP_TAGS_SECTION
    };
    [
        prompts.compress_philosophy.as_str(),
        prompts.how_to_compress_rules.as_str(),
        tags,
        tools::TOOLS_SECTION,
        tools::SUMMARIES_IN_CONTEXT_SECTION,
    ]
    .join("\n\n")
}
