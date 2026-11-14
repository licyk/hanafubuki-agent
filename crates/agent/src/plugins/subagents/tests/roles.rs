//! An agent spawned as a role (`agent_type`): only a spawnable role is
//! accepted, by a field-named error that lists them; a child's tools are
//! its parent's narrowed by its role's, its instructions its role's (its
//! parent's without one); its model and reasoning effort its role's, else
//! its parent's; `list_agents` names each one's role; and the guidance
//! lists the spawnable roles, bounded and sorted, as they are at each step,
//! while `spawn_agent`'s spec names none.

use hanafubuki_agent_kernel::ModelReasoningEffort;

use super::*;

const LEAD: &str = r#"description = "Leads."
instructions = "Lead the work."

[tools]
allow = ["read_file", "list_directory", "apply_patch", "spawn_agent", "list_agents"]
"#;

fn write_role(paths: &TestPaths, name: &str, definition: &str) {
    let agents = paths.config_dir.join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join(format!("{name}.toml")), definition).unwrap();
}

async fn started_with_roles(
    name: &str,
    server: &FakeServer,
    roles: &[(&str, &str)],
) -> (AgentApplicationService, TestPaths) {
    let (service, paths, _) = service_with_host(name, test_host, |paths, config| {
        configure_test_provider(config, &server.base_url, Some(ModelReasoningEffort::Medium));
        for (role, definition) in roles {
            write_role(paths, role, definition);
        }
    });
    service.start_plugins().await;
    (service, paths)
}

fn tools(request: &Value) -> Vec<String> {
    let mut tools = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    tools.sort();
    tools
}

fn spawn_as(task_name: &str, message: &str, agent_type: Option<&str>) -> Value {
    let mut arguments = json!({ "task_name": task_name, "message": message, "fork_turns": "none" });
    if let Some(agent_type) = agent_type {
        arguments["agent_type"] = json!(agent_type);
    }
    arguments
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_spawnable_role_is_accepted_and_the_error_lists_them() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = started_with_roles(
        "subagents-role-refused",
        &server,
        &[
            (
                "quiet",
                "description = \"Never spawned.\"\nspawnable = false\n",
            ),
            ("lead", LEAD),
        ],
    )
    .await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    for agent_type in ["quiet", "nope"] {
        let refused = call(
            &subagents,
            SubagentToolKind::SpawnAgent,
            parent,
            spawn_as("worker", "Work.", Some(agent_type)),
        )
        .await
        .unwrap_err();
        assert_eq!(
            refused,
            AgentToolError::Failed(format!(
                "invalid tool argument agent_type: `{agent_type}` is not a role an agent can be spawned as; use one of default, explorer, lead"
            ))
        );
    }
    assert!(subagents.agents(parent).await.unwrap().is_empty());
    assert!(server.requests().is_empty());
    finish(service, paths);
}

/// A lead whose role allows a few tools spawns an explorer and a default
/// agent: the explorer's tools are the two roles' intersection and its
/// instructions are its own; the default one keeps the lead's tools,
/// instructions and role.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_is_its_parent_narrowed_by_its_role() {
    let server = FakeServer::start(vec![
        (assistant_events("Explored."), Duration::ZERO),
        (assistant_events("Helped."), Duration::ZERO),
    ])
    .await;
    let (service, paths) =
        started_with_roles("subagents-role-scope", &server, &[("lead", LEAD)]).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, Some("lead".to_string()))
        .await
        .unwrap()
        .id;

    call(
        &subagents,
        SubagentToolKind::SpawnAgent,
        parent,
        spawn_as("scout", "Explore the code.", Some("explorer")),
    )
    .await
    .unwrap();
    call(
        &subagents,
        SubagentToolKind::SpawnAgent,
        parent,
        spawn_as("helper", "Help out.", None),
    )
    .await
    .unwrap();

    let explorer = request_with(&server, "Explore the code.").await;
    assert_eq!(
        tools(&explorer),
        [
            "list_directory",
            "new_context",
            "read_file",
            "request_user_input",
            "update_plan"
        ]
    );
    let instructions = explorer["instructions"].as_str().unwrap();
    assert!(!instructions.contains("Lead the work."), "{instructions}");
    let helper = request_with(&server, "Help out.").await;
    assert_eq!(
        tools(&helper),
        [
            "apply_patch",
            "list_agents",
            "list_directory",
            "new_context",
            "read_file",
            "request_user_input",
            "spawn_agent",
            "update_plan"
        ]
    );
    let instructions = helper["instructions"].as_str().unwrap();
    assert!(
        instructions.ends_with("\n\nLead the work.\n"),
        "{instructions}"
    );

    let listed = call(&subagents, SubagentToolKind::ListAgents, parent, json!({}))
        .await
        .unwrap();
    let roles = listed["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|agent| {
            (
                agent["agent_name"].as_str().unwrap(),
                agent["agent_type"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(roles, [("scout", "explorer"), ("helper", "lead")]);
    subagents
        .wait(parent, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    finish(service, paths);
}

/// The guidance is shown only where the step offers `spawn_agent`: to a
/// lead whose role allows it, not to an explorer whose role leaves it out.
#[tokio::test(flavor = "multi_thread")]
async fn the_guidance_follows_whether_the_step_offers_spawn_agent() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 2]).await;
    let (service, paths) =
        started_with_roles("subagents-role-guidance-scope", &server, &[("lead", LEAD)]).await;
    for (role, text) in [("lead", "As a lead."), ("explorer", "As an explorer.")] {
        let session = service
            .create_session(AgentSessionMode::Default, Some(role.to_string()))
            .await
            .unwrap()
            .id;
        service
            .start_turn(session, text.to_string().into())
            .await
            .unwrap();
        wait_until("the turn ends", || !turn_running(&service, session)).await;
    }

    let lead = request_with(&server, "As a lead.").await;
    assert!(tools(&lead).contains(&"spawn_agent".to_string()));
    assert!(lead["input"].to_string().contains("<subagents>"));
    let explorer = request_with(&server, "As an explorer.").await;
    assert!(!tools(&explorer).contains(&"spawn_agent".to_string()));
    assert!(!explorer["input"].to_string().contains("<subagents>"));
    finish(service, paths);
}

/// A role's model and reasoning effort are the child's; what the role does
/// not set is the parent's, the effort even when the role changes the
/// model (whose Provider policy has another).
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_takes_its_roles_model_and_effort_else_its_parents() {
    let server = FakeServer::start(vec![
        (assistant_events("Fast."), Duration::ZERO),
        (assistant_events("Slow."), Duration::ZERO),
        (assistant_events("Read."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started_with_roles(
        "subagents-role-model",
        &server,
        &[
            (
                "fast",
                "description = \"Fast.\"\nmodel = \"fast-model\"\ninstructions = \"Be quick.\"\n",
            ),
            (
                "slow",
                "description = \"Slow.\"\nreasoningEffort = \"low\"\n",
            ),
        ],
    )
    .await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let parent = service
        .set_session_reasoning_effort(crate::AgentSessionReasoningEffortRequest {
            session_id: parent.id,
            generation: parent.generation,
            revision: parent.revision,
            effort: Some(ModelReasoningEffort::High),
        })
        .await
        .unwrap();
    assert_eq!(parent.model, "fixture-model");

    for (task_name, agent_type) in [("fast", "fast"), ("slow", "slow"), ("reader", "explorer")] {
        call(
            &subagents,
            SubagentToolKind::SpawnAgent,
            parent.id,
            spawn_as(task_name, &format!("Task {task_name}."), Some(agent_type)),
        )
        .await
        .unwrap();
    }
    let children = subagents
        .agents(parent.id)
        .await
        .unwrap()
        .into_iter()
        .map(|agent| service.snapshot(agent.id).unwrap())
        .collect::<Vec<_>>();

    let chosen = children
        .iter()
        .map(|child| (child.model.as_str(), child.reasoning_effort.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        chosen,
        [
            ("fast-model", Some(ModelReasoningEffort::High)),
            ("fixture-model", Some(ModelReasoningEffort::Low)),
            ("fixture-model", Some(ModelReasoningEffort::High)),
        ]
    );
    // The role's instructions follow the base ones, past the model change.
    let fast = request_with(&server, "Task fast.").await;
    assert_eq!(fast["model"], "fast-model");
    let instructions = fast["instructions"].as_str().unwrap();
    assert!(instructions.ends_with("\n\nBe quick.\n"), "{instructions}");
    subagents
        .wait(parent.id, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    finish(service, paths);
}

/// The guidance lists at most 16 spawnable roles, by name, whatever order
/// their files were written in, each description on one line and cut at
/// 200 characters (never inside a character); `spawn_agent`'s spec lists
/// none.
#[tokio::test(flavor = "multi_thread")]
async fn the_guidance_lists_the_spawnable_roles_bounded_and_sorted() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let long = format!("{}\nsecond line", "é".repeat(250));
    let definitions = (0..20)
        .rev()
        .map(|index| {
            (
                format!("r{index:02}"),
                format!("description = \"\"\"{long}\"\"\"\n"),
            )
        })
        .collect::<Vec<_>>();
    let mut roles = definitions
        .iter()
        .map(|(name, definition)| (name.as_str(), definition.as_str()))
        .collect::<Vec<_>>();
    roles.push(("hidden", "description = \"Hidden.\"\nspawnable = false\n"));
    let (service, paths) = started_with_roles("subagents-role-guidance", &server, &roles).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(parent, "Hello.".to_string().into())
        .await
        .unwrap();

    let request = body(&server, 0);
    let request: Value = serde_json::from_str(&request).unwrap();
    let guidance = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"][0]["text"].as_str())
        .find(|text| text.contains("<subagents>"))
        .unwrap();
    // The roles' paragraph; the Models part after it lists Providers.
    let roles_part = guidance
        .split("\n\n")
        .find(|part| part.starts_with("`spawn_agent`'s `agent_type`"))
        .unwrap();
    let listed = roles_part
        .lines()
        .filter_map(|line| line.strip_prefix("- `"))
        .collect::<Vec<_>>();
    let names = listed
        .iter()
        .map(|line| line.split('`').next().unwrap())
        .collect::<Vec<_>>();
    let mut expected = vec!["default".to_string(), "explorer".to_string()];
    expected.extend((0..14).map(|index| format!("r{index:02}")));
    assert_eq!(names, expected);
    let cut = format!("- `r00`: {}…", "é".repeat(200));
    assert!(guidance.lines().any(|line| line == cut), "{guidance}");
    assert!(!guidance.contains("second line") && !guidance.contains("hidden"));
    let spawn = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "spawn_agent")
        .unwrap();
    let agent_type = spawn["parameters"]["properties"]["agent_type"]["description"]
        .as_str()
        .unwrap();
    assert!(agent_type.contains("sub-agent guidance"), "{agent_type}");
    assert!(!agent_type.contains("- `") && !agent_type.contains("r00"));
    finish(service, paths);
}

/// A role written after the Agent started is spawnable once the roles are
/// read again: a spawn reads the catalog at its call, and the guidance at
/// its step; `spawn_agent`'s spec, which names no role, is the same before
/// and after.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_and_the_guidance_read_the_roles_as_they_are_now() {
    let server = FakeServer::start(vec![
        (assistant_events("Early."), Duration::ZERO),
        (assistant_events("Late."), Duration::ZERO),
        (assistant_events("Hello."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started_with_roles("subagents-role-reload", &server, &[]).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(parent, "Before.".to_string().into())
        .await
        .unwrap();
    wait_until("the first turn ends", || !turn_running(&service, parent)).await;
    let before: Value = serde_json::from_str(&body(&server, 0)).unwrap();
    write_role(&paths, "late", "description = \"Came later.\"\n");
    service.reload_packages().await.unwrap();

    call(
        &subagents,
        SubagentToolKind::SpawnAgent,
        parent,
        spawn_as("latecomer", "Task late.", Some("late")),
    )
    .await
    .unwrap();
    request_with(&server, "Task late.").await;
    subagents
        .wait(parent, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    service
        .start_turn(parent, "Hello.".to_string().into())
        .await
        .unwrap();
    let after = body(&server, 2);
    assert!(after.contains("- `late`: Came later."), "{after}");
    assert!(!body(&server, 0).contains("`late`"));
    let after: Value = serde_json::from_str(&after).unwrap();
    assert_eq!(spawn_agent_spec(&after), spawn_agent_spec(&before));
    finish(service, paths);
}

fn spawn_agent_spec(request: &Value) -> &Value {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "spawn_agent")
        .unwrap()
}
