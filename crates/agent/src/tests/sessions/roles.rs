//! Sessions with a role (site/content/docs/roles.en.mdx, "Roles in
//! sessions"): creation takes the role's instructions, model and scope,
//! an unknown role is refused with the usable ones, a model selection keeps
//! the role's instructions, `set_session_role` switches between turns and
//! `default` clears the scope, recovery keeps a scope whose role file is
//! gone, and the `skills` and `memory` plugins turned off take their values
//! out of the session.

use super::*;
use serde_json::Value;

const READER: &str = r#"description = "Reads."
instructions = "Only read; never edit."
model = "role-model"
permissionModeCap = "manual"

[tools]
allow = ["read_file", "list_directory"]

[plugins]
off = ["titles"]
"#;

fn write_role(paths: &TestPaths, name: &str, definition: &str) {
    let agents = paths.config_dir.join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join(format!("{name}.toml")), definition).unwrap();
}

fn body(server: &FakeServer, index: usize) -> Value {
    serde_json::from_slice(&server.requests()[index].body).unwrap()
}

fn tools(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn a_session_takes_its_role_and_an_unknown_role_is_refused() {
    let server = FakeServer::start(vec![
        (assistant_events("Read."), Duration::ZERO),
        (assistant_events("Still read."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = service_with("role-session", |paths, config| {
        configure_test_provider(config, &server.base_url, None);
        write_role(paths, "reader", READER);
        // Never the built-in's: `default` is reserved.
        write_role(
            paths,
            "default",
            "description = \"Mine.\"\ntools.allow = []\n",
        );
    });
    let catalog = service.roles();
    assert!(catalog.issues().iter().any(
        |issue| issue.role.as_deref() == Some("default") && issue.message.contains("reserved")
    ));

    let refused = service
        .create_session(AgentSessionMode::Default, Some("nope".to_string()))
        .await
        .unwrap_err();
    assert!(
        refused.contains("no role is named `nope`; the usable roles are default, explorer, reader"),
        "{refused}"
    );
    let plain = service
        .create_session(AgentSessionMode::Default, Some("default".to_string()))
        .await
        .unwrap();
    assert_eq!((plain.role, plain.scope), (None, None));

    let session = service
        .create_session(AgentSessionMode::Default, Some("reader".to_string()))
        .await
        .unwrap();
    assert_eq!(session.role.as_deref(), Some("reader"));
    assert_eq!(session.model, "role-model");
    let scope = session.scope.clone().unwrap();
    assert_eq!((scope.allow_lists, scope.allow_patterns), (1, 2));
    assert_eq!(scope.plugins_off, ["titles"]);
    let summary = service
        .list_sessions_page(crate::AgentSessionListRequest::default())
        .unwrap()
        .sessions
        .into_iter()
        .find(|summary| summary.session_id == session.id)
        .unwrap();
    assert_eq!(summary.role.as_deref(), Some("reader"));
    service
        .start_turn(
            session.id,
            crate::AgentTurnInput {
                permission_mode: Some(AgentPermissionMode::FullAccess),
                .."Look.".into()
            },
        )
        .await
        .unwrap();
    let request = body(&server, 0);
    assert_eq!(request["model"], "role-model");
    let instructions = request["instructions"].as_str().unwrap();
    assert!(
        instructions.ends_with("\n\nOnly read; never edit.\n"),
        "{instructions}"
    );
    let mut offered = tools(&request);
    offered.sort();
    assert_eq!(
        offered,
        [
            "list_directory",
            "new_context",
            "read_file",
            "request_user_input",
            "update_plan"
        ]
    );
    // The cap: the turn ran in manual, and its permission section says so.
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.permission_mode, AgentPermissionMode::Manual);
    let section = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"][0]["text"].as_str())
        .find(|text| text.contains("kind=\"permission_mode\""))
        .unwrap();
    assert!(section.contains("{\"mode\":\"manual\"}"), "{section}");

    // A model selection keeps the role's instructions.
    let session = service.snapshot(session.id).unwrap();
    service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: session.id,
            generation: session.generation,
            revision: session.revision,
            provider_id: session.provider_id.clone(),
            model: "fixture-model".to_string(),
        })
        .await
        .unwrap();
    service
        .start_turn(session.id, "Again.".to_string().into())
        .await
        .unwrap();
    let request = body(&server, 1);
    assert_eq!(request["model"], "fixture-model");
    assert!(request["instructions"]
        .as_str()
        .unwrap()
        .ends_with("\n\nOnly read; never edit.\n"));

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_role_switches_between_turns_and_default_clears_it() {
    let server = FakeServer::start(vec![
        (assistant_events("Explored."), Duration::ZERO),
        (assistant_events("Back."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("role-switch", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let refused = service
        .set_session_role(session.id, "nope")
        .await
        .unwrap_err();
    assert!(
        refused.contains("the usable roles are default, explorer"),
        "{refused}"
    );

    let switched = service
        .set_session_role(session.id, crate::EXPLORER_ROLE)
        .await
        .unwrap();
    assert_eq!(switched.role.as_deref(), Some("explorer"));
    service
        .start_turn(session.id, "Explore.".to_string().into())
        .await
        .unwrap();
    let explored = tools(&body(&server, 0));
    assert!(explored.contains(&"read_file".to_string()), "{explored:?}");
    assert!(
        !explored.contains(&"exec_command".to_string()),
        "{explored:?}"
    );
    assert!(
        !explored.contains(&"apply_patch".to_string()),
        "{explored:?}"
    );

    let cleared = service
        .set_session_role(session.id, crate::DEFAULT_ROLE)
        .await
        .unwrap();
    assert_eq!((cleared.role, cleared.scope), (None, None));
    service
        .start_turn(session.id, "Edit.".to_string().into())
        .await
        .unwrap();
    assert!(tools(&body(&server, 1)).contains(&"exec_command".to_string()));

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn recovery_keeps_the_scope_after_the_role_file_is_deleted() {
    let server = FakeServer::start(vec![(assistant_events("Recovered."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_config("role-recovery", |paths, config| {
        configure_test_provider(config, &server.base_url, None);
        write_role(paths, "reader", READER);
    });
    let session = service
        .create_session(AgentSessionMode::Default, Some("reader".to_string()))
        .await
        .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);
    std::fs::remove_file(paths.config_dir.join("agents").join("reader.toml")).unwrap();

    let service = AgentApplicationService::new_with_host(
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
    assert!(service.roles().role("reader").is_none());
    let reopened = service.open_session(session.id, 10).unwrap().runtime;
    assert_eq!(reopened.role.as_deref(), Some("reader"));
    assert_eq!(reopened.scope, session.scope);
    service
        .start_turn(session.id, "Read.".to_string().into())
        .await
        .unwrap();
    let request = body(&server, 0);
    assert!(request["instructions"]
        .as_str()
        .unwrap()
        .ends_with("\n\nOnly read; never edit.\n"));
    assert!(!tools(&request).contains(&"exec_command".to_string()));

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// `skills` off: no Skill catalog, so an explicit reference is refused, a
/// `$mention` selects nothing, and the Skill tools are absent.
#[tokio::test]
async fn a_role_with_skills_off_has_no_skill_catalog() {
    let server = FakeServer::start(vec![(assistant_events("No Skills."), Duration::ZERO)]).await;
    let (service, paths) = service_with("role-skills-off", |paths, config| {
        configure_test_provider(config, &server.base_url, None);
        write_role(
            paths,
            "plain",
            "description = \"Plain.\"\nplugins.off = [\"skills\"]\n",
        );
    });
    enable_two_skills(&service, &paths);
    let skill = service.list_skills().unwrap().entries.remove(0).id;
    let session = service
        .create_session(AgentSessionMode::Default, Some("plain".to_string()))
        .await
        .unwrap();

    let refused = service
        .start_turn(
            session.id,
            crate::AgentTurnInput {
                skills: vec![hanafubuki_agent_kernel::AgentSkillRef { skill }],
                .."Use it.".into()
            },
        )
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("the Skill catalog is unavailable"),
        "{refused}"
    );
    service
        .start_turn(session.id, "Use $diagnostics.".to_string().into())
        .await
        .unwrap();
    let request = body(&server, 0);
    let offered = tools(&request);
    assert!(
        !offered.iter().any(|name| name.starts_with("skills_")),
        "{offered:?}"
    );
    assert!(!request.to_string().contains("release-notes"));
    // A `$mention` names no Skill without the catalog: it stays text.
    assert!(!request
        .to_string()
        .contains("Follow the diagnostics workflow."));

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// `memory` off: no memory summary and no memory tools, while the
/// session still contributes as its contribution mode says (observers are
/// never filtered).
#[tokio::test]
async fn a_role_with_memory_off_reads_no_memory_but_still_contributes() {
    let server = FakeServer::start(vec![(assistant_events("No memory."), Duration::ZERO)]).await;
    let (service, paths) = service_with("role-memory-off", |paths, config| {
        configure_test_provider(config, &server.base_url, None);
        enable_memory(config, false);
        write_role(
            paths,
            "forgetful",
            "description = \"Forgetful.\"\nplugins.off = [\"memory\"]\n",
        );
    });
    std::fs::write(
        service
            .test_memory()
            .repository()
            .paths()
            .artifacts
            .join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, Some("forgetful".to_string()))
        .await
        .unwrap();
    service
        .start_turn(session.id, "How should I run validation?".into())
        .await
        .unwrap();
    let request = String::from_utf8(server.requests()[0].body.clone()).unwrap();
    assert!(!request.contains("MEMORY_SUMMARY BEGINS"));
    assert!(!request.contains("list_agent_memories"));

    let (mode, _) = service
        .set_memory_session_contribution(session.id, session.memory_contribution.revision, true)
        .unwrap();
    assert_eq!(
        service
            .test_memory()
            .repository()
            .contribution_mode(&session.id.to_string())
            .unwrap()
            .0,
        mode
    );

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
