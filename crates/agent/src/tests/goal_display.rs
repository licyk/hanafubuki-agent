use super::golden::{agent_history_dir, copy_tree, golden_path};
use super::*;
use crate::AgentSessionListRequest;

#[tokio::test]
async fn recorded_goal_calls_restore_readable_presentations() {
    let (service, paths) = service_with("goal-display-history", |paths, _config| {
        copy_tree(&golden_path("history-goal"), &agent_history_dir(paths));
    });
    service.start_plugins().await;
    let session_id = service
        .list_sessions_page(AgentSessionListRequest::default())
        .unwrap()
        .sessions[0]
        .session_id;
    let mut goal_displays = Vec::new();
    let mut pending_goal = None;
    for item in super::colour::transcript(&service, session_id) {
        match item {
            ConversationItem::ToolCall {
                namespace,
                name,
                arguments,
                ..
            } if matches!(name.as_str(), "create_goal" | "get_goal" | "update_goal") => {
                pending_goal = Some((namespace, name, arguments));
            }
            ConversationItem::ToolOutput {
                output,
                display_metadata,
                ..
            } => {
                let Some((namespace, name, arguments)) = pending_goal.take() else {
                    continue;
                };
                let shown = service.present_tool_call(
                    namespace.as_deref(),
                    &name,
                    &arguments,
                    Some((&output, &display_metadata)),
                );
                let Some(crate::AgentToolDisplay::Text { text }) = shown.display else {
                    panic!("goal presentation missing after reopening history");
                };
                assert!(text.contains("Goal: Write the changelog"), "{text}");
                goal_displays.push(text);
            }
            _ => {}
        }
    }
    assert!(goal_displays
        .iter()
        .any(|text| text.contains("Status: active")));
    assert!(goal_displays
        .iter()
        .any(|text| text.contains("Status: complete")));

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
