#[cfg(unix)]
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn workspace_instruction_changes_refresh_on_the_next_turn_without_a_new_session() {
    let server = FakeServer::start(vec![
        (assistant_events("first"), Duration::ZERO),
        (assistant_events("second"), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("world-state-refresh", &server.base_url);
    let instructions = paths.managed_instances_dir.join("AGENTS.md");
    std::fs::write(&instructions, "workspace rule one").unwrap();
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "first turn".into())
        .await
        .unwrap();

    std::fs::write(&instructions, "workspace rule two").unwrap();
    service
        .start_turn(session.id, "second turn".into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let first = String::from_utf8(requests[0].body.clone()).unwrap();
    let second = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(first.contains("workspace rule one"));
    assert!(!first.contains("workspace rule two"));
    assert!(second.contains("workspace rule two"));
    assert!(!second.contains("workspace rule one"));

    let history = service
        .open_session(session.id, 100)
        .unwrap()
        .transcript
        .items;
    let workspace_updates = history
        .iter()
        .filter(|item| {
            matches!(
                &item.content,
                hanafubuki_agent_kernel::ConversationItem::WorldStateUpdate { updates }
                    if updates.iter().any(|update| matches!(
                        update,
                        hanafubuki_agent_kernel::AgentWorldStateUpdate::Replace { kind, .. }
                            if *kind == hanafubuki_agent_kernel::ContextSectionKind::WORKSPACE_INSTRUCTIONS
                    ))
            )
        })
        .count();
    assert_eq!(workspace_updates, 2);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
