use super::*;

fn kinds() -> BTreeSet<String> {
    ["agent_memory", "skill_catalog", "application_ui_context"]
        .map(str::to_owned)
        .into()
}

#[test]
fn kinds_are_collected_where_they_are_declared() {
    let constants = r#"
impl ContextSectionKind {
    pub const WORKSPACE_ROOTS: Self = Self::fixed("workspace_roots");
    pub const ENVIRONMENT: Self = Self::fixed("environment");
}
"#;
    assert_eq!(
        declared_kinds(constants, "Self::fixed(\""),
        ["environment", "workspace_roots"].map(str::to_owned).into()
    );
    let plugin = r#"
const AGENT_MEMORY: ContextSectionKind = ContextSectionKind::fixed("agent_memory");
action: AgentActivityAction::fixed("view_image"),
"#;
    assert_eq!(
        declared_kinds(plugin, "ContextSectionKind::fixed(\""),
        ["agent_memory"].map(str::to_owned).into()
    );
}

#[test]
fn a_kind_enum_and_its_wrapper_match_are_found() {
    let source = r#"
pub enum ModelContextKind {
    AgentMemory,
    SkillCatalog,
}
match kind {
    ModelContextKind::AgentMemory => format!("{AGENT_MEMORY_INSTRUCTIONS}\n{content}"),
    ModelContextKind::ApplicationUiContext => content.to_owned(),
}
"#;
    assert_eq!(
        kind_mentions(source, &kinds()),
        [
            (3, "AgentMemory".to_owned()),
            (4, "SkillCatalog".to_owned()),
            (7, "AgentMemory".to_owned()),
            (8, "ApplicationUiContext".to_owned()),
        ]
    );
}

#[test]
fn a_match_on_the_kind_label_is_found() {
    let source =
        "match kind.as_str() {\n    \"skill_catalog\" => wrap(content),\n    _ => content,\n}\n";
    assert_eq!(
        kind_mentions(source, &kinds()),
        [(2, "\"skill_catalog\"".to_owned())]
    );
}

#[test]
fn model_api_framing_and_longer_names_are_left_alone() {
    let source = r#"
pub(crate) fn continuation_context_text(provider_id: &str, content: &str) -> String {}
pub(crate) fn compaction_summary_text(summary: &str) -> String {}
kind: "remote_compaction".to_string(),
"- `read_agent_memory` reads a line range from one id."
let label = "agent_memory_index";
struct AgentMemoryStep;
"#;
    assert!(kind_mentions(source, &kinds()).is_empty());
}

#[test]
fn a_one_word_kind_counts_only_as_a_literal() {
    let kinds = ["environment"].map(str::to_owned).into();
    let source = "struct Environment;\nlet kind = \"environment\";\n";
    assert_eq!(
        kind_mentions(source, &kinds),
        [(2, "\"environment\"".to_owned())]
    );
}
