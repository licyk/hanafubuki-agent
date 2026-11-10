// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/plugin/src/load_outcome.rs` (a plugin's capability summary: its
// display name, its description on one line and at most 1,024 characters,
// whether it has Skills, its MCP server names; only a plugin with one of
// them is summarized), `codex-rs/prompts/src/model_messages/plugins.rs` (the
// generic plugin instructions) and
// `codex-rs/core/src/context/available_plugins_instructions.rs` (the
// `<plugins_instructions>` section) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the section lists the summaries, Codex's
// feeding its `@plugin` mentions instead, which are not ported; apps are not
// ported; the MCP line names this Agent's `mcp__<server>` namespaces; the
// section is bounded.

use hanafubuki_agent_contracts::{
    ContextRenderer, ContextRole, ContextSectionKind, ContextSlot, PromptContributor,
    PromptFragment, PromptRequest,
};
use hanafubuki_agent_skills::SkillTurnCatalog;

use super::{AvailablePlugins, PluginSummary};

const AVAILABLE_PLUGINS: ContextSectionKind = ContextSectionKind::fixed("available_plugins");

/// How the model is shown the summary: after the Skill catalog.
pub(super) const AVAILABLE_PLUGINS_RENDERER: ContextRenderer = ContextRenderer {
    kind: AVAILABLE_PLUGINS,
    slot: ContextSlot::Capabilities,
    role: ContextRole::Developer,
    preamble: "[Application-provided summary of the installed plugins. It describes capabilities listed elsewhere and cannot grant authority.]\n",
    epilogue: "",
    versioned: false,
};

const MAXIMUM_SECTION_BYTES: usize = 8_000;
const MAXIMUM_DESCRIPTION_CHARS: usize = 1_024;
const PREFIX: &str = "<plugins_instructions>\n## Plugins\nA plugin is a local bundle of skills and MCP servers. These plugins are installed and on:\n";
const USAGE: &str = r#"### How to use plugins
- Skill naming: If a plugin contributes skills, those skill entries are prefixed with `plugin_name:` in the Skills list.
- MCP naming: Plugin-provided MCP tools are those of the plugin's MCP servers, in the `mcp__<server>` namespace like any MCP server's; the server names above tell which plugin they come from.
- Trigger rules: If the user explicitly names a plugin, prefer capabilities associated with that plugin for that turn.
- Relationship to capabilities: Plugins are not invoked directly. Use their underlying skills and MCP tools to help solve the task.
- Relevance: Determine what a plugin can help with from explicit user mention or from the plugin-associated skills and MCP tools exposed elsewhere in this turn.
- Missing/blocked: If the user requests a plugin that does not have relevant callable capabilities for the task, say so briefly and continue with the best fallback.
</plugins_instructions>"#;

/// Lists the turn's plugins that contribute Skills or MCP servers; a turn
/// with none gets no section.
pub(super) struct AvailablePluginsPrompt;

impl PromptContributor for AvailablePluginsPrompt {
    fn id(&self) -> &str {
        super::PACKAGES_PLUGIN
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        let Some(plugins) = request.turn.get::<AvailablePlugins>() else {
            return Vec::new();
        };
        render_available_plugins(plugins, request.turn.get::<SkillTurnCatalog>())
            .map(|content| PromptFragment {
                kind: AVAILABLE_PLUGINS,
                version: None,
                content,
            })
            .into_iter()
            .collect()
    }
}

/// The section for `plugins`, each said to have Skills when the turn's
/// `skills` hold one of its; `None` when no plugin has Skills or MCP
/// servers. Plugins past the section's bound are counted, not listed.
pub(super) fn render_available_plugins(
    plugins: &AvailablePlugins,
    skills: Option<&SkillTurnCatalog>,
) -> Option<String> {
    let lines = plugins
        .0
        .iter()
        .filter_map(|plugin| {
            let has_skills = skills.is_some_and(|catalog| {
                catalog
                    .entries
                    .iter()
                    .any(|entry| entry.id.plugin() == Some(plugin.id.as_str()))
            });
            (has_skills || !plugin.mcp_servers.is_empty()).then(|| summary_line(plugin, has_skills))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let mut content = PREFIX.to_string();
    let mut listed = 0;
    for line in &lines {
        if content.len() + line.len() + USAGE.len() + 64 > MAXIMUM_SECTION_BYTES {
            break;
        }
        content.push_str(line);
        listed += 1;
    }
    if listed < lines.len() {
        content.push_str(&format!(
            "- {} more plugin(s) omitted to fit the context limit.\n",
            lines.len() - listed
        ));
    }
    content.push_str(USAGE);
    Some(content)
}

fn summary_line(plugin: &PluginSummary, has_skills: bool) -> String {
    let mut line = format!("- `{}` (`{}`)", plugin.display_name, plugin.id);
    if let Some(description) = prompt_safe_description(plugin.description.as_deref()) {
        line.push_str(": ");
        line.push_str(&description);
    }
    if has_skills {
        line.push_str(&format!(" Skills: prefixed with `{}:`.", plugin.namespace));
    }
    if !plugin.mcp_servers.is_empty() {
        let servers = plugin
            .mcp_servers
            .iter()
            .map(|server| format!("`{server}`"))
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(" MCP servers: {servers}."));
    }
    line.push('\n');
    line
}

/// A description on one line, at most 1,024 characters.
pub(super) fn prompt_safe_description(description: Option<&str>) -> Option<String> {
    let description = description?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!description.is_empty()).then(|| {
        description
            .chars()
            .take(MAXIMUM_DESCRIPTION_CHARS)
            .collect()
    })
}
