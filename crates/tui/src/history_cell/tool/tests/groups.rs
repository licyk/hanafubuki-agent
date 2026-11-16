// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::{both, new_tool_call, render, text, AgentToolDisplay, HistoryCell, ToolGroup};

#[test]
fn skill_and_mcp_groups_are_distinct_from_exploration_and_other_plugins() {
    for name in [
        "skills_list",
        "skills_read",
        "enable_skill",
        "disable_skill",
    ] {
        assert_eq!(ToolGroup::for_name(name), Some(ToolGroup::Skills));
    }
    for name in [
        "mcp__docs__lookup",
        "list_mcp_resources",
        "list_mcp_resource_templates",
        "read_mcp_resource",
    ] {
        assert_eq!(ToolGroup::for_name(name), Some(ToolGroup::Mcp));
    }
    for name in ["plugin__skills_read", "mcp_status"] {
        assert_eq!(ToolGroup::for_name(name), None);
    }
}

#[test]
fn a_single_skill_read_is_compact_and_keeps_its_instructions_in_details() {
    let mut cell = new_tool_call(
        "read".into(),
        "skills_read".into(),
        Some("user:rust".into()),
    );
    assert!(render(&cell, 80).contains("Using skills\n  └ Read skill user:rust (running)"));
    assert!(cell.transcript_animation_tick().is_some());
    cell.finish(
        "read",
        AgentToolDisplay::Text {
            text: "Full skill instructions".into(),
        },
    );
    assert_eq!(render(&cell, 80), "• Used skills\n  └ Read skill user:rust");
    assert!(text(&cell.transcript_lines(80)).contains("Full skill instructions"));
    assert!(cell.activity_disclosure(80).is_some());
    assert_eq!(cell.transcript_animation_tick(), None);
}

#[test]
fn mcp_groups_keep_namespaces_errors_and_results_when_expanded() {
    let mut cell = new_tool_call(
        "one".into(),
        "mcp__one__lookup".into(),
        Some("MCP one".into()),
    );
    cell.finish(
        "one",
        AgentToolDisplay::Text {
            text: "first result".into(),
        },
    );
    cell.join(new_tool_call(
        "two".into(),
        "mcp__two__lookup".into(),
        Some("MCP two".into()),
    ));
    assert!(render(&cell, 80).contains("Calling MCP"));
    assert!(render(&cell, 80).contains("mcp__two__lookup: MCP two (running)"));
    cell.complete("two", true);
    cell.show(
        "two",
        hanafubuki_agent::AgentToolCallDisplay {
            subject: None,
            display: Some(AgentToolDisplay::Text {
                text: "Access denied".into(),
            }),
        },
    );
    insta::assert_snapshot!(both(&cell, 80));
    assert_eq!(cell.activity_ids(), ["tool:one", "tool:two"]);
    assert_eq!(cell.transcript_animation_tick(), None);
}

#[test]
fn an_interrupted_skill_group_stops_animating_and_marks_unfinished_calls() {
    let mut cell = new_tool_call("list".into(), "skills_list".into(), None);
    cell.join(new_tool_call(
        "enable".into(),
        "enable_skill".into(),
        Some("user:rust".into()),
    ));
    cell.end_running();
    let shown = render(&cell, 80);
    assert!(shown.contains("Used skills · 2 failed"));
    assert!(shown.contains("List skills (failed)"));
    assert!(shown.contains("Enable skill user:rust (failed)"));
    assert_eq!(cell.transcript_animation_tick(), None);
}
