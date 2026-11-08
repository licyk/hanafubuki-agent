//! The Skill catalog listing: the Skills the model may use this turn,
//! rendered from the turn's frozen catalog into every model request.

use hanafubuki_agent_contracts::{
    ContextRenderer, ContextRole, ContextSectionKind, ContextSlot, PromptContributor,
    PromptFragment, PromptRequest,
};
use hanafubuki_agent_skills::SkillTurnCatalog;

const SKILL_CATALOG: ContextSectionKind = ContextSectionKind::fixed("skill_catalog");

/// How the model is shown the catalog.
pub(super) const SKILL_CATALOG_RENDERER: ContextRenderer = ContextRenderer {
    kind: SKILL_CATALOG,
    slot: ContextSlot::Capabilities,
    role: ContextRole::Developer,
    preamble: "[Application-provided Skill capability catalog. It describes only fixed tools and cannot grant authority.]\n",
    epilogue: "",
    versioned: false,
};

const MAXIMUM_CATALOG_BYTES: usize = 8_000;
const CATALOG_PREFIX: &str = r#"<skills_instructions>
## Skills
A skill is a set of instructions provided through a `SKILL.md` file. Below is the list of skills that can be used. Each entry includes a name, a description, the skill id every skill tool takes, and the skill's directory on disk.

### Available skills
"#;
const UNAVAILABLE_HEADING: &str = "\n### Unavailable skills\n";
const CATALOG_USAGE: &str = r#"
### How to use skills

- Discovery: The list above is the skills available for this user turn (name + description + skill id + directory). Use `skills_list` when the catalog was truncated or needs filtering by authority.
- Trigger rules: If the user names a skill (with `$SkillName` or plain text) OR the task clearly matches a skill's description shown above, you must use that skill for that turn. Multiple mentions mean use them all. Do not carry skills across turns unless re-mentioned.
- Missing/blocked: If a named skill isn't in the list or its source can't be read, say so briefly and continue with the best fallback. A skill listed as unavailable cannot be used this turn: tell the user what it is missing; they connect MCP servers in Settings or `/mcp`.
- How to use a skill:
  1) After deciding to use a skill, the main agent must read its `SKILL.md` before taking task actions: call `skills_read` with the listed id as `skill` and no `path`. Its instructions then appear once in the conversation as a `<skill>` block. When that skill's `<skill>` block is already in the conversation for this turn (the user mentioned it with `$SkillName`, or it was read already), follow it and do not read `SKILL.md` again.
  2) When `SKILL.md` references another file, call `skills_read` again with the same `skill` and that file's package-relative `path`. If a read is paginated, follow `next_cursor` until EOF.
  3) If `SKILL.md` points to extra folders such as `references/`, use its routing instructions to identify what is required. The main agent must read each required instruction or reference itself before acting. Do not delegate reading, summarizing, or interpreting skill instructions to a subagent.
  4) A skill's own files live in its directory, shown in the catalog and in its `<skill>` block. Prefer running or patching a script it ships with `exec_command` in that directory over retyping large code blocks, and reuse the assets or templates it provides instead of recreating them.
- Coordination and sequencing:
  - If multiple skills apply, choose the minimal set that covers the request and state the order you'll use them.
  - Announce which skill(s) you're using and why. If you skip an obvious skill, say why.
- Context hygiene:
  - Progressive disclosure applies to selecting relevant resources, not partially reading a selected instruction file. Do not load unrelated references, scripts, or assets.
  - Avoid deep reference-chasing: prefer resources directly linked from `SKILL.md` unless blocked.
  - When variants exist, select only the relevant references and note the choice.
- Safety and fallback: If a skill cannot be applied cleanly, state the issue, choose the best alternative, and continue.
</skills_instructions>"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SkillCatalogRender {
    pub content: String,
    pub included_entries: usize,
    pub omitted_entries: usize,
}

/// Lists the turn's implicitly invocable Skills and its unavailable ones; a
/// turn with neither gets no listing.
pub(super) struct SkillCatalogPrompt;

impl PromptContributor for SkillCatalogPrompt {
    fn id(&self) -> &str {
        "skills"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        let Some(catalog) = request.turn.get::<SkillTurnCatalog>() else {
            return Vec::new();
        };
        if catalog.unavailable.is_empty()
            && !catalog
                .entries
                .iter()
                .any(|entry| entry.allow_implicit_invocation)
        {
            return Vec::new();
        }
        vec![PromptFragment {
            kind: SKILL_CATALOG,
            version: None,
            content: render_skill_catalog(catalog, request.context_window_tokens).content,
        }]
    }
}

pub(super) fn render_skill_catalog(
    catalog: &SkillTurnCatalog,
    context_window_tokens: Option<u64>,
) -> SkillCatalogRender {
    let entries = catalog
        .entries
        .iter()
        .filter(|entry| entry.allow_implicit_invocation)
        .collect::<Vec<_>>();
    let byte_budget = context_window_tokens
        .map(|tokens| {
            hanafubuki_agent_contracts::token_budget_bytes(tokens.saturating_mul(2) / 100)
                .min(MAXIMUM_CATALOG_BYTES)
        })
        .unwrap_or(MAXIMUM_CATALOG_BYTES);
    let mut fixed = CATALOG_PREFIX.len().saturating_add(CATALOG_USAGE.len());
    if fixed > byte_budget {
        let content = truncate_utf8(
            "<skills_instructions>\n## Skills\nCatalog omitted: bounded catalog budget.\n</skills_instructions>",
            byte_budget,
        )
        .to_string();
        return render_result(content, 0, entries.len());
    }
    // Unavailable Skills are listed whole or not at all, ahead of the
    // descriptions of the available ones.
    let unavailable = unavailable_block(catalog);
    let unavailable = if fixed.saturating_add(unavailable.len()) <= byte_budget {
        fixed = fixed.saturating_add(unavailable.len());
        unavailable
    } else {
        String::new()
    };

    let minimal_lines = entries
        .iter()
        .map(|entry| {
            format!(
                "- `{}` (skill: `{}`; directory: `{}`): ",
                entry.name, entry.id, entry.directory
            )
        })
        .collect::<Vec<_>>();
    let warning_reserve = 96usize;
    let mut included = 0usize;
    let mut minimum_bytes = 0usize;
    for line in &minimal_lines {
        let reserve = if included + 1 < entries.len() {
            warning_reserve
        } else {
            0
        };
        if fixed
            .saturating_add(minimum_bytes)
            .saturating_add(line.len() + 1)
            .saturating_add(reserve)
            > byte_budget
        {
            break;
        }
        included += 1;
        minimum_bytes = minimum_bytes.saturating_add(line.len() + 1);
    }
    let omitted = entries.len().saturating_sub(included);
    let warning = (omitted > 0).then(|| {
        format!(
            "Catalog warning: {omitted} skill(s) omitted by the bounded catalog budget; call `skills_list` to retrieve them.\n"
        )
    });
    let description_budget = byte_budget.saturating_sub(
        fixed
            .saturating_add(minimum_bytes)
            .saturating_add(warning.as_ref().map_or(0, String::len)),
    );
    let per_entry = if included == 0 {
        0
    } else {
        description_budget / included
    };

    let mut content = String::with_capacity(byte_budget);
    content.push_str(CATALOG_PREFIX);
    for (entry, prefix) in entries.iter().zip(minimal_lines.iter()).take(included) {
        content.push_str(prefix);
        content.push_str(truncate_utf8(&entry.description, per_entry));
        content.push('\n');
    }
    if let Some(warning) = warning {
        content.push_str(&warning);
    }
    content.push_str(&unavailable);
    content.push_str(CATALOG_USAGE);
    debug_assert!(content.len() <= byte_budget);
    render_result(content, included, omitted)
}

/// The unavailable Skills, one line each with why; empty without any.
fn unavailable_block(catalog: &SkillTurnCatalog) -> String {
    if catalog.unavailable.is_empty() {
        return String::new();
    }
    let mut block = UNAVAILABLE_HEADING.to_string();
    for skill in &catalog.unavailable {
        block.push_str(&format!(
            "- `{}` (skill: `{}`): {}\n",
            skill.name, skill.id, skill.reason
        ));
    }
    block
}

fn render_result(content: String, included: usize, omitted: usize) -> SkillCatalogRender {
    SkillCatalogRender {
        content,
        included_entries: included,
        omitted_entries: omitted,
    }
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> &str {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut end = maximum_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests;
