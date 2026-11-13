use hanafubuki_agent_kernel::{
    AgentAttachmentId, AgentImageAttachmentRef, AgentImageDetail, AgentMessageId, AgentPlanBlock,
    AgentToolCatalogRevision, AgentToolGeneration, AgentToolId, AgentToolProviderId,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::json;

use super::*;

fn user(text: &str) -> ConversationItem {
    ConversationItem::UserMessage {
        text: text.to_string(),
        images: Vec::new(),
    }
}

fn assistant(text: &str, plan: Option<&str>) -> ConversationItem {
    ConversationItem::AssistantMessage {
        message_id: AgentMessageId::new(1),
        phase: None,
        text: text.to_string(),
        model_text: None,
        proposed_plan: plan.map(|text| AgentPlanBlock {
            text: text.to_string(),
        }),
        sampling_step: 0,
        memory_citations: Vec::new(),
    }
}

fn call(call_id: &str, name: &str, arguments: Value) -> ConversationItem {
    ConversationItem::ToolCall {
        call_id: call_id.to_string(),
        namespace: None,
        name: name.to_string(),
        origin: Default::default(),
        catalog_revision: AgentToolCatalogRevision([1; 32]),
        provider_id: AgentToolProviderId::parse("builtin").unwrap(),
        tool_id: AgentToolId::parse(name).unwrap(),
        tool_generation: AgentToolGeneration([2; 32]),
        arguments,
    }
}

fn output(call_id: &str, text: &str, is_error: bool) -> ConversationItem {
    ConversationItem::ToolOutput {
        call_id: call_id.to_string(),
        origin: Default::default(),
        output: ToolOutput {
            content: vec![ToolOutputContent::Text {
                text: text.to_string(),
            }],
            structured_content: None,
            is_error,
        },
        truncation: None,
        images: Vec::new(),
        tool_search_results: Vec::new(),
    }
}

#[test]
fn messages_plans_and_calls_become_sections_in_order() {
    let items = [
        user("List the files."),
        ConversationItem::Reasoning {
            text: "I should run ls.".to_string(),
            duration_millis: 10,
            sampling_step: 0,
            truncated: false,
            replay: None,
        },
        call("call-1", "exec_command", json!({"cmd": "ls"})),
        output("call-1", "a.txt\nb.txt", false),
        assistant("Two files:\n\n- `a.txt`\n- `b.txt`", Some("1. Read them")),
    ];

    assert_eq!(
        render_markdown(&items).unwrap(),
        concat!(
            "# Hanafubuki Agent conversation\n",
            "\n## User\n\nList the files.\n",
            "\n## Activity\n\n",
            "    exec_command({\"cmd\":\"ls\"})\n",
            "    a.txt\n",
            "    b.txt\n",
            "\n## Assistant\n\nTwo files:\n\n- `a.txt`\n- `b.txt`\n",
            "\n## Plan\n\n1. Read them\n",
        )
    );
}

#[test]
fn a_failed_call_says_so_and_long_output_is_cut() {
    let long: Vec<String> = (1..=13).map(|line| format!("line {line}")).collect();
    let items = [
        call("call-1", "exec_command", json!({"cmd": "make"})),
        output("call-1", &long.join("\n"), true),
    ];

    let markdown = render_markdown(&items).unwrap();

    assert!(
        markdown.contains("    exec_command({\"cmd\":\"make\"}) · failed\n"),
        "{markdown}"
    );
    assert!(markdown.contains("    line 10\n"), "{markdown}");
    assert!(!markdown.contains("line 11"), "{markdown}");
    assert!(markdown.contains("    … 3 more lines\n"), "{markdown}");
}

#[test]
fn images_are_labelled_and_a_conversation_without_content_has_none() {
    let image = AgentImageAttachmentRef {
        attachment_id: AgentAttachmentId::parse("a".repeat(64)).unwrap(),
        source_session_id: AgentSessionId::new(1),
        mime_type: "image/png".to_string(),
        byte_length: 1,
        width: 1,
        height: 1,
        display_name: "shot.png".to_string(),
        detail: AgentImageDetail::default(),
    };
    let items = [ConversationItem::UserMessage {
        text: "Look.".to_string(),
        images: vec![image],
    }];

    assert!(render_markdown(&items)
        .unwrap()
        .contains("\n## User\n\nLook.\n\n[Image #1]\n"));
    assert_eq!(render_markdown(&[]), None);
    assert_eq!(render_markdown(&[user("   ")]), None);
}

#[test]
fn a_user_shell_command_is_an_activity_with_its_exit() {
    let shell =
        |command: &str, exit_code: Option<i32>, output: &str| ConversationItem::UserShellCommand {
            call_id: "user-shell-1".to_string(),
            command: command.to_string(),
            cwd: None,
            exit_code,
            duration_millis: 12,
            output: output.to_string(),
            truncated: false,
        };
    let items = [
        shell("ls", Some(0), "a.txt\nb.txt"),
        shell("false", Some(1), ""),
        shell("sleep 30", None, "the command was stopped"),
    ];

    assert_eq!(
        render_markdown(&items).unwrap(),
        concat!(
            "# Hanafubuki Agent conversation\n",
            "\n## Activity\n\n",
            "    !ls\n",
            "    a.txt\n",
            "    b.txt\n",
            "\n## Activity\n\n",
            "    !false · exit 1\n",
            "\n## Activity\n\n",
            "    !sleep 30 · failed\n",
            "    the command was stopped\n",
        )
    );
}
