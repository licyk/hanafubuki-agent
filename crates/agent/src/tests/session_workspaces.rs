use super::*;
use hanafubuki_agent_contracts::TurnWorkspace;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentSessionOrigin, AgentSessionPersistence,
};

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_sessions_keep_distinct_files_context_skills_and_execution_roots() {
    let arguments = json!({"path":"identity.txt", "offset":0, "maxBytes":128}).to_string();
    let server = FakeServer::start(vec![
        (tool_events("read_file", &arguments), Duration::ZERO),
        (tool_events("read_file", &arguments), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("session-workspace-isolation", &server.base_url);
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    std::fs::write(left.path().join("identity.txt"), "left-unique-content").unwrap();
    std::fs::write(right.path().join("identity.txt"), "right-unique-content").unwrap();
    for (root, name) in [(left.path(), "left-skill"), (right.path(), "right-skill")] {
        let directory = root.join(".agents/skills").join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: A workspace skill.\n---\nUse this workspace.\n"
            ),
        )
        .unwrap();
    }
    let a = service
        .create_session_with_workspace(
            AgentSessionMode::Default,
            None,
            left.path().to_string_lossy().into(),
        )
        .await
        .unwrap();
    let b = service
        .create_session_with_workspace(
            AgentSessionMode::Default,
            None,
            right.path().to_string_lossy().into(),
        )
        .await
        .unwrap();
    let ta = service
        .test_session_driver()
        .collect_turn_extensions(a.id, a.mode)
        .await
        .unwrap();
    let tb = service
        .test_session_driver()
        .collect_turn_extensions(b.id, b.mode)
        .await
        .unwrap();
    let ac = ta
        .get::<hanafubuki_agent_skills::SkillTurnCatalog>()
        .unwrap();
    let bc = tb
        .get::<hanafubuki_agent_skills::SkillTurnCatalog>()
        .unwrap();
    assert!(ac.entries.iter().any(|entry| entry.name == "left-skill"));
    assert!(!ac.entries.iter().any(|entry| entry.name == "right-skill"));
    assert!(bc.entries.iter().any(|entry| entry.name == "right-skill"));
    assert!(!bc.entries.iter().any(|entry| entry.name == "left-skill"));
    let ar = ta.get::<TurnWorkspace>().unwrap().roots.clone();
    let br = tb.get::<TurnWorkspace>().unwrap().roots.clone();
    assert_ne!(ar, br);
    let ae = service.test_executions().with_workspace_roots(ar.clone());
    let be = service.test_executions().with_workspace_roots(br.clone());
    assert_eq!(ae.working_directory(), ar.first().cloned());
    assert_eq!(be.working_directory(), br.first().cloned());
    let (a_result, b_result) = tokio::join!(
        service.start_turn(a.id, "Read identity.txt".to_string().into()),
        service.start_turn(b.id, "Read identity.txt".to_string().into())
    );
    a_result.unwrap();
    b_result.unwrap();
    for (session, expected, other) in [
        (a.id, "left-unique-content", "right-unique-content"),
        (b.id, "right-unique-content", "left-unique-content"),
    ] {
        let page = service
            .read_transcript_page(crate::AgentTranscriptPageRequest {
                session_id: session,
                page_size: 50,
                cursor: None,
                direction: crate::AgentHistorySortDirection::Ascending,
                through_revision: None,
            })
            .unwrap();
        let events = serde_json::to_string(&page).unwrap();
        assert!(events.contains(expected), "{events}");
        assert!(!events.contains(other), "{events}");
    }
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn workspace_binding_survives_fork_child_and_reopen_and_refuses_missing_paths() {
    let (service, paths, config) = service_with_config("session-workspace-reopen", |_, config| {
        configure_test_provider(config, "http://127.0.0.1:1/v1", None)
    });
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_string_lossy().to_string();
    let session = service
        .create_session_with_workspace(AgentSessionMode::Default, None, path.clone())
        .await
        .unwrap();
    let fork = service
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: session.generation,
            source_revision: session.revision,
        })
        .unwrap()
        .session;
    let child = service
        .test_session_driver()
        .create_child(AgentChildSessionRequest {
            parent_session_id: session.id,
            origin: AgentSessionOrigin::Side,
            persistence: AgentSessionPersistence::Durable,
            base: AgentChildSessionBase::Empty,
            context: Vec::new(),
            scope: Default::default(),
            role_instructions: None,
        })
        .unwrap();
    assert_eq!(
        service.session_summary(session.id).unwrap().cwd.as_deref(),
        Some(path.as_str())
    );
    for id in [session.id, fork.id, child.id] {
        assert_eq!(
            service
                .core
                .session_workspace(id)
                .unwrap()
                .execution_root
                .as_deref(),
            Some(path.as_str())
        );
    }
    let invalid = root.path().join("not-directory");
    std::fs::write(&invalid, "file").unwrap();
    assert!(service
        .create_session_with_workspace(
            AgentSessionMode::Default,
            None,
            invalid.to_string_lossy().into()
        )
        .await
        .is_err());
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);
    let reopened = AgentApplicationService::new_with_host(
        config,
        test_host(&paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        hanafubuki_agent_process::executor::ExecutorManager::default(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .session_workspace_roots(session.id)
            .unwrap()
            .primary()
            .to_string_lossy(),
        path
    );
    drop(root);
    assert!(reopened.session_workspace_roots(session.id).is_err());
    assert_eq!(
        reopened.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn clipboard_images_are_normalized_staged_and_readable_without_a_file_path() {
    use image::{DynamicImage, ImageFormat};
    let (service, paths) = configured_service("clipboard-bytes", "http://127.0.0.1:1/v1");
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgba8(4, 3)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let attachment = service
        .import_image_bytes(
            session.id,
            bytes.into_inner(),
            "clipboard.png".into(),
            hanafubuki_agent_kernel::AgentImageDetail::Auto,
        )
        .unwrap();
    let staged = crate::prepare_and_stage_images(
        &service.core,
        &service.files,
        session.id,
        vec![DesktopAgentImageInput {
            path: None,
            attachment: Some(attachment.clone()),
            display_name: "clipboard.png".into(),
            detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
        }],
    )
    .unwrap();
    assert_eq!(staged, vec![attachment.clone()]);
    let preview = service.read_image_preview(session.id, attachment).unwrap();
    assert!(preview.width <= 512 && preview.height <= 512);
    assert_eq!(preview.width * 3, preview.height * 4);
    assert!(preview.data_url.starts_with("data:image/png;base64,"));
    assert!(service
        .import_image_bytes(
            session.id,
            b"bad image".to_vec(),
            "bad.png".into(),
            hanafubuki_agent_kernel::AgentImageDetail::Auto
        )
        .is_err());
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
