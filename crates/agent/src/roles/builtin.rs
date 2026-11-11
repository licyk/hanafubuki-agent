//! The roles compiled into the Agent (plan decision D4): `default`, which
//! narrows nothing, and `explorer`, which reads. Codex's `explorer`
//! description (`codex-rs/core/src/agent/role.rs`, `mod built_in`, at
//! commit `57a808a392`, Apache-2.0) is adapted for its spawn guidance.

use hanafubuki_agent_config::InstructionsMode;
use hanafubuki_agent_kernel::{AgentCapabilityScope, AgentToolScope, ToolPattern};

use super::resolve::Candidate;
use super::AgentRoleSource;

/// The role a session has without one.
pub const DEFAULT_ROLE: &str = "default";
/// The built-in read-only role.
pub const EXPLORER_ROLE: &str = "explorer";

const DEFAULT_DESCRIPTION: &str =
    "The Agent as it is configured: every tool and plugin the instance has, nothing narrowed.";

const EXPLORER_DESCRIPTION: &str = "Use `explorer` for specific, well-scoped questions about \
the codebase. Explorers are fast and authoritative: they list, search and read files and \
images and search and read the web, and never edit files or run commands.
Rules:
- Do not explore again what an explorer already covered: trust its findings without checking \
them again. You may still read the code yourself for the context you need.
- Ask several explorers in parallel when you have distinct questions that can be answered \
independently, and keep working on what does not depend on their answers meanwhile.
- Reuse an explorer for related questions.";

/// What `explorer` may use: reading tools alone.
const EXPLORER_TOOLS: [&str; 6] = [
    "list_directory",
    "search_text",
    "read_file",
    "view_image",
    "web_search",
    "fetch_web_page",
];

pub(super) fn built_in_roles() -> Vec<Candidate> {
    let explorer_tools = EXPLORER_TOOLS
        .iter()
        .map(|name| ToolPattern::Name((*name).to_string()))
        .collect();
    vec![
        built_in(DEFAULT_ROLE, DEFAULT_DESCRIPTION, AgentToolScope::default()),
        built_in(
            EXPLORER_ROLE,
            EXPLORER_DESCRIPTION,
            AgentToolScope {
                allow: vec![explorer_tools],
                deny: Default::default(),
            },
        ),
    ]
}

fn built_in(name: &str, description: &str, tools: AgentToolScope) -> Candidate {
    Candidate {
        name: name.to_string(),
        source: AgentRoleSource::BuiltIn,
        description: Some(description.to_string()),
        extends: None,
        instructions: None,
        instructions_mode: InstructionsMode::Append,
        model: None,
        reasoning_effort: None,
        spawnable: Some(true),
        scope: AgentCapabilityScope {
            role: Some(name.to_string()),
            tools,
            ..AgentCapabilityScope::default()
        },
        unusable: None,
    }
}
