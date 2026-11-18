// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::style::Color;

use super::*;

const CATEGORIES: [(&str, &str, &str, &str); 11] = [
    (
        "write_stdin",
        "session 3",
        "Interacting with terminals",
        "Interacted with terminals",
    ),
    (
        "read_agent_memory",
        "MEMORY.md",
        "Using memory",
        "Used memory",
    ),
    (
        "read_thread",
        "session 70",
        "Reading conversations",
        "Read conversations",
    ),
    (
        "search_context",
        "build",
        "Managing context",
        "Managed context",
    ),
    ("view_image", "diagram.png", "Viewing image", "Viewed image"),
    ("mcp__docs__lookup", "MCP docs", "Calling MCP", "Called MCP"),
    ("skills_read", "user:rust", "Using skills", "Used skills"),
    (
        "search_tools",
        "mcp__docs__lookup",
        "Discovering tools",
        "Discovered tools",
    ),
    (
        "web_search",
        "ratatui",
        "Searching the web",
        "Searched the web",
    ),
    (
        "create_goal",
        "Fix the bug",
        "Managing goal",
        "Managed goal",
    ),
    (
        "request_user_input",
        "Choose a target",
        "Asking user",
        "Asked user",
    ),
];

#[test]
fn categorized_calls_keep_their_header_and_disclosure_through_completion() {
    let mut gallery = Vec::new();
    for (name, subject, running, completed) in CATEGORIES {
        let mut cell = new_tool_call("call".into(), name.into(), Some(subject.into()));
        assert!(render(&cell, 80).contains(&format!("{running}\n  └ ")));
        assert!(cell.transcript_animation_tick().is_some());
        assert_eq!(
            cell.activity_disclosure(80),
            Some(ActivityDisclosure::Generic)
        );
        assert!(text(&cell.transcript_lines(80)).contains(running));
        cell.complete("call", false);
        assert_eq!(cell.transcript_animation_tick(), None);
        assert!(render(&cell, 80).starts_with(&format!("• {completed}\n  └ ")));
        cell.show(
            "call",
            AgentToolCallDisplay {
                subject: None,
                display: Some(AgentToolDisplay::Text {
                    text: "Full result kept in details".into(),
                }),
            },
        );
        assert_eq!(
            render(&cell, 80).contains("Full result"),
            matches!(name, "create_goal" | "request_user_input")
        );
        assert!(text(&cell.transcript_lines(80)).starts_with(&format!("• {completed}\n  └ ")));
        assert!(text(&cell.transcript_lines(80)).contains("Full result kept in details"));
        assert!(text(&cell.raw_lines()).contains("Full result kept in details"));
        assert_eq!(
            cell.display_lines(80)[0].spans[0].style.fg,
            Some(Color::Green)
        );
        gallery.push(both(&cell, 32));
    }
    insta::assert_snapshot!(gallery.join("\n\n"));
}

#[test]
fn failed_and_interrupted_categories_keep_errors_and_stop_animating() {
    for (name, subject, _, _) in CATEGORIES {
        let mut cell = new_tool_call("first".into(), name.into(), Some(subject.into()));
        cell.complete("first", true);
        cell.show(
            "first",
            AgentToolCallDisplay {
                subject: None,
                display: Some(AgentToolDisplay::Text {
                    text: "Access denied".into(),
                }),
            },
        );
        cell.join(new_tool_call(
            "second".into(),
            name.into(),
            Some(subject.into()),
        ));
        assert!(render(&cell, 40).contains("1 failed"));
        assert!(render(&cell, 40).contains("Access denied"));
        assert!(cell.transcript_animation_tick().is_some());
        cell.end_running();
        let rendered = render(&cell, 40);
        assert!(rendered.contains("2 failed"), "{rendered}");
        assert_eq!(rendered.matches("(failed)").count(), 2, "{rendered}");
        assert!(!rendered.contains("(running)"));
        assert_eq!(cell.activity_ids(), ["tool:first", "tool:second"]);
        assert_eq!(
            cell.display_lines(40)[0].spans[0].style.fg,
            Some(Color::Red)
        );
        assert_eq!(cell.transcript_animation_tick(), None);
    }
}

#[test]
fn image_details_show_media_metadata_and_plural_groups_keep_each_path() {
    let mut cell = new_tool_call("one".into(), "view_image".into(), Some("first.png".into()));
    cell.finish(
        "one",
        AgentToolDisplay::Content {
            blocks: vec![hanafubuki_agent::AgentOutputBlock::Image {
                mime_type: "image/png".into(),
                bytes: 2048,
            }],
        },
    );
    assert_eq!(render(&cell, 80), "• Viewed image\n  └ View first.png");
    assert!(text(&cell.transcript_lines(80)).contains("image/png · 2.0 KiB"));
    cell.join(new_tool_call(
        "two".into(),
        "view_image".into(),
        Some("second.png".into()),
    ));
    assert!(render(&cell, 80).contains("Viewing images"));
    cell.complete("two", false);
    assert_eq!(
        render(&cell, 80),
        "• Viewed images\n  └ View first.png\n    View second.png"
    );
}

#[test]
fn discovery_aliases_and_web_goal_operations_stay_in_their_categories() {
    for (names, group) in [
        (
            &["write_stdin", "resize_execution"][..],
            ToolGroup::Terminals,
        ),
        (
            &[
                "list_agent_memories",
                "search_agent_memories",
                "read_agent_memory",
                "append_agent_memory_note",
            ][..],
            ToolGroup::Memory,
        ),
        (&["read_thread"][..], ToolGroup::Conversations),
        (
            &[
                "decompress",
                "search_context",
                "new_context",
                "acp_status",
                "acp_cache",
                "absorb",
                "acp_retrieve",
                "acp_rule",
            ][..],
            ToolGroup::Context,
        ),
        (
            &[
                "tool_list",
                "search_tools",
                "__hanafubuki_native_tool_search",
            ][..],
            ToolGroup::Discovery,
        ),
        (
            &[
                "web_search",
                "advanced_search",
                "multi_search",
                "fetch_web_page",
                "platform_search",
                "video_search",
                "domain_search",
                "weather",
                "research",
                "cache_search",
                "free_search_test",
            ][..],
            ToolGroup::Web,
        ),
        (
            &["create_goal", "get_goal", "update_goal"][..],
            ToolGroup::Goals,
        ),
    ] {
        for name in names {
            assert_eq!(ToolGroup::for_name(name), Some(group));
        }
    }
    for name in [
        "plugin__view_image",
        "plugin__web_search",
        "plugin__update_plan",
        "plugin__write_stdin",
        "plugin__read_agent_memory",
        "goal_status",
        "exec_command",
    ] {
        assert_eq!(ToolGroup::for_name(name), None);
    }
    assert_eq!(
        ToolGroup::for_name("mcp__docs__web_search"),
        Some(ToolGroup::Mcp)
    );
}

#[test]
fn terminal_interactions_group_polls_input_and_resize_without_hiding_failures() {
    let mut cell = new_tool_call(
        "poll".into(),
        "write_stdin".into(),
        Some("session 3".into()),
    );
    cell.finish("poll", command("", "building\n", None));
    cell.join(new_tool_call(
        "input".into(),
        "write_stdin".into(),
        Some("session 3 ← exit 7\\n".into()),
    ));
    cell.finish("input", command("exit 7\\n", "failed build\n", Some(7)));
    cell.join(new_tool_call(
        "resize".into(),
        "resize_execution".into(),
        Some("session 4 to 80×24".into()),
    ));
    assert!(render(&cell, 80).contains("Interacting with terminals · 1 failed"));
    cell.finish(
        "resize",
        AgentToolDisplay::Text {
            text: "resized".into(),
        },
    );
    let compact = render(&cell, 80);
    assert!(compact.contains("Wait for session 3"), "{compact}");
    assert!(compact.contains("Send input to session 3"), "{compact}");
    assert!(compact.contains("Resize session 4 to 80×24"), "{compact}");
    assert!(
        compact.contains("failed build") && compact.contains("exit 7"),
        "{compact}"
    );
    let expanded = text(&cell.transcript_lines(80));
    assert!(
        expanded.contains("building") && expanded.contains("resized"),
        "{expanded}"
    );
    assert_eq!(
        cell.activity_ids(),
        ["tool:poll", "tool:input", "tool:resize"]
    );
    assert_eq!(cell.transcript_animation_tick(), None);
    assert_eq!(
        cell.display_lines(80)[0].spans[0].style.fg,
        Some(Color::Red)
    );
}

#[test]
fn completed_questions_and_goals_show_wrapped_summaries_without_expanding() {
    for (tool, summary) in [
        ("request_user_input", "请选择需要修改的项目范围以继续下一步 → 仅此功能\n运行测试？ → 运行"),
        ("update_goal", "Status: complete\nGoal: 修复终端工具状态显示并验证恢复会话\nTokens: 250 / 1000 · 750 remaining\nTime: 42s used"),
    ] {
        let cell = shown(tool, None, AgentToolDisplay::Text { text: summary.into() });
        let collapsed = render(&cell, 36);
        for expected in if tool == "request_user_input" { vec!["仅此功能", "运行测试？", "运行"] } else { vec!["Status: complete", "Tokens: 250", "Time: 42s used"] } {
            assert!(collapsed.contains(expected), "{collapsed}");
        }
        assert_eq!(cell.transcript_animation_tick(), None);
        assert_eq!(cell.activity_disclosure(36), Some(ActivityDisclosure::Generic));
        let expanded = text(&cell.transcript_lines(120));
        for line in summary.lines() {
            assert!(expanded.contains(line), "{expanded}");
        }
    }
}

#[test]
fn long_goal_summaries_keep_the_remaining_text_in_details() {
    let cell = shown(
        "get_goal",
        None,
        AgentToolDisplay::Text { text: numbered(8) },
    );
    let collapsed = render(&cell, 80);
    assert!(collapsed.contains("line 5"));
    assert!(!collapsed.contains("line 6"));
    assert!(text(&cell.transcript_lines(80)).contains("line 8"));
}
