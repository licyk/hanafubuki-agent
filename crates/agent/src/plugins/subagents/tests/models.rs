//! An agent's Provider, model and reasoning effort, chosen by `spawn_agent`'s
//! `provider`, `model` and `reasoning_effort`: each the argument's, else
//! the role's, else the parent's, the pair compared as a whole; what the
//! model passed is checked before the agent exists, each refusal naming
//! its argument with what it can pass instead; a new pair and its effort
//! are one commit; and an agent on another Provider samples there, with
//! its parent's history, that model's window and image capability, after
//! a restart too.

use std::io::Cursor;

use hanafubuki_agent_config::{AgentConfig, AgentModelOverrideSettings};
use hanafubuki_agent_kernel::{
    AgentImageAttachmentRef, AgentImageDetail, AgentSessionRuntimeSnapshot, ModelMetadataSource,
    ModelReasoningEffort, PreparedAgentImage,
};
use image::{DynamicImage, ImageFormat};

use super::*;
use crate::test_support::{add_test_provider, http_error, reopen_service, FakeListing};
use crate::AgentProviderModelCatalogRequest;

const FAST: &str = "description = \"Fast.\"\nmodel = \"fast-model\"\nreasoningEffort = \"low\"\n";

/// An Agent whose default Provider `provider-1` (Fixture Provider, policy
/// effort `medium`) is `first`, beside `provider-2` (Second) at `second`,
/// with the role `fast`, after `configure`.
pub(super) async fn with_two_providers(
    name: &str,
    first: &FakeServer,
    second: &FakeServer,
    configure: impl FnOnce(&AgentConfig),
) -> (AgentApplicationService, TestPaths, AgentConfig) {
    let (service, paths, config) = service_with_host(name, test_host, |paths, config| {
        configure_test_provider(config, &first.base_url, Some(ModelReasoningEffort::Medium));
        add_test_provider(config, "provider-2", "Second", &second.base_url);
        configure(config);
        let agents = paths.config_dir.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("fast.toml"), FAST).unwrap();
    });
    service.start_plugins().await;
    (service, paths, config)
}

/// Records the listing of `provider_id` from its server.
pub(super) async fn list(service: &AgentApplicationService, provider_id: &str) {
    let listed = service
        .list_provider_models(AgentProviderModelCatalogRequest {
            provider_ids: Some(vec![provider_id.to_string()]),
        })
        .await
        .unwrap();
    assert_eq!(listed[0].error, None);
}

/// Waits until none of `parent`'s agents runs.
async fn settle(subagents: &Arc<SubagentService>, parent: AgentSessionId) {
    loop {
        match subagents
            .wait(parent, NO_WAIT, &CancellationToken::default())
            .await
            .unwrap()
        {
            WaitOutcome::Idle => return,
            WaitOutcome::Updates(_) => {}
            WaitOutcome::TimedOut => panic!("an agent still runs"),
        }
    }
}

/// Spawns `task_name` with `arguments` beside its task and no history,
/// and waits for its turn; returns spawn's result.
pub(super) async fn spawned(
    subagents: &Arc<SubagentService>,
    parent: AgentSessionId,
    task_name: &str,
    arguments: Value,
) -> Value {
    let mut call_arguments = json!({
        "task_name": task_name,
        "message": format!("Task {task_name}."),
        "fork_turns": "none",
    });
    for (key, value) in arguments.as_object().unwrap() {
        call_arguments[key] = value.clone();
    }
    let result = call(
        subagents,
        SubagentToolKind::SpawnAgent,
        parent,
        call_arguments,
    )
    .await
    .unwrap();
    settle(subagents, parent).await;
    result
}

fn child(service: &AgentApplicationService, result: &Value) -> AgentSessionRuntimeSnapshot {
    service
        .snapshot(AgentSessionId::new(
            result["agent_id"].as_str().unwrap().parse().unwrap(),
        ))
        .unwrap()
}

/// What the result says the agent runs on.
pub(super) fn runs_on(result: &Value) -> (&str, &str, Option<&str>) {
    (
        result["provider"].as_str().unwrap(),
        result["model"].as_str().unwrap(),
        result["reasoning_effort"].as_str(),
    )
}

fn effort(request: &Value) -> Option<&str> {
    request["reasoning"]["effort"].as_str()
}

/// A model passed without a role runs on the parent's Provider with the
/// parent's effort, and the result says so.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_passed_model_runs_on_the_parents_provider_with_the_parents_effort() {
    let first = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let second = FakeServer::start(Vec::new()).await;
    let (service, paths, _) =
        with_two_providers("subagents-model-explicit", &first, &second, |_| {}).await;
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

    let result = spawned(
        &subagents,
        parent.id,
        "other",
        json!({ "model": "other-model" }),
    )
    .await;

    let agent = child(&service, &result);
    assert_eq!(
        result,
        json!({
            "task_name": "other",
            "agent_id": agent.id.to_string(),
            "provider": "provider-1",
            "model": "other-model",
            "reasoning_effort": "high",
        })
    );
    let request = request_with(&first, "Task other.").await;
    assert_eq!(request["model"], "other-model");
    assert_eq!(effort(&request), Some("high"));
    finish(service, paths);
}

/// Field by field, the argument wins over the role and the role over the
/// parent; a passed Provider (by id or by name) moves the agent there with
/// the parent's model id, the pair compared as a whole.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn each_field_is_the_arguments_else_the_roles_else_the_parents() {
    let first = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 3]).await;
    let second = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 2]).await;
    let (service, paths, _) =
        with_two_providers("subagents-model-precedence", &first, &second, |_| {}).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let cases = [
        (
            "role_only",
            json!({ "agent_type": "fast" }),
            ("provider-1", "fast-model", Some("low")),
        ),
        (
            "model_over_role",
            json!({ "agent_type": "fast", "model": "arg-model" }),
            ("provider-1", "arg-model", Some("low")),
        ),
        (
            "effort_over_role",
            json!({ "agent_type": "fast", "reasoning_effort": "high" }),
            ("provider-1", "fast-model", Some("high")),
        ),
        (
            "role_on_provider",
            json!({ "agent_type": "fast", "provider": "provider-2" }),
            ("provider-2", "fast-model", Some("low")),
        ),
        (
            "parents_id_elsewhere",
            json!({ "provider": "second" }),
            ("provider-2", "fixture-model", Some("medium")),
        ),
    ];

    for (task_name, arguments, expected) in cases {
        let result = spawned(&subagents, parent, task_name, arguments).await;
        assert_eq!(runs_on(&result), expected, "{task_name}");
        let agent = child(&service, &result);
        assert_eq!(
            (
                agent.provider_id.as_str(),
                agent.model.as_str(),
                agent
                    .reasoning_effort
                    .as_ref()
                    .map(|effort| effort.as_str())
            ),
            expected,
            "{task_name}"
        );
        let server = if expected.0 == "provider-1" {
            &first
        } else {
            &second
        };
        let request = request_with(server, &format!("Task {task_name}.")).await;
        assert_eq!(request["model"], expected.1, "{task_name}");
        assert_eq!(effort(&request), expected.2, "{task_name}");
    }
    let parent = service.snapshot(parent).unwrap();
    assert_eq!(
        (parent.provider_id.as_str(), parent.model.as_str()),
        ("provider-1", "fixture-model")
    );
    finish(service, paths);
}

/// A Provider passed alone keeps the inherited model id when its listing
/// names it, and is refused when it does not; a model configured for the
/// Provider (`models."<id>"`) is known though its listing leaves it out.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_provider_alone_takes_the_inherited_model_only_where_it_is_known() {
    let first = FakeServer::start(Vec::new()).await;
    let second = FakeServer::with_listings(
        vec![(assistant_events("Done."), Duration::ZERO)],
        vec![FakeListing::ok(
            r#"{"data":[{"id":"fixture-model"},{"id":"other"}]}"#,
        )],
    )
    .await;
    let third = FakeServer::with_listings(
        vec![(assistant_events("Done."), Duration::ZERO)],
        vec![FakeListing::ok(r#"{"data":[{"id":"c-model"}]}"#)],
    )
    .await;
    let (service, paths, _) =
        with_two_providers("subagents-model-provider", &first, &second, |config| {
            add_test_provider(config, "provider-3", "Third", &third.base_url);
            let revision = config.providers().unwrap().revision;
            config
                .mutate_providers(revision, |registry| {
                    registry.providers[2].models.insert(
                        "configured-model".into(),
                        AgentModelOverrideSettings::default(),
                    );
                    Ok(())
                })
                .unwrap();
        })
        .await;
    list(&service, "provider-2").await;
    list(&service, "provider-3").await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let moved = spawned(
        &subagents,
        parent,
        "moved",
        json!({ "provider": "provider-2" }),
    )
    .await;
    assert_eq!(
        runs_on(&moved),
        ("provider-2", "fixture-model", Some("medium"))
    );
    let refused = call(
        &subagents,
        SubagentToolKind::SpawnAgent,
        parent,
        json!({ "task_name": "third", "message": "Task third.", "provider": "provider-3" }),
    )
    .await
    .unwrap_err();
    assert_eq!(
        refused,
        AgentToolError::Failed(
            "invalid tool argument model: the model this agent would inherit, `fixture-model`, is not one Provider `provider-3` (Third) lists; pass a `model`; closest: `c-model`, `configured-model`".to_string()
        )
    );
    let configured = spawned(
        &subagents,
        parent,
        "third",
        json!({ "provider": "provider-3", "model": "configured-model" }),
    )
    .await;
    assert_eq!(
        runs_on(&configured),
        ("provider-3", "configured-model", Some("medium"))
    );
    assert_eq!(
        request_with(&third, "Task third.").await["model"],
        "configured-model"
    );
    finish(service, paths);
}

/// Every refusal names its argument and says what to pass instead, and
/// leaves no agent behind, the task name free; the same name then spawns
/// with a level the model takes, and with an effort for a model whose
/// levels are not known, which the request carries as given.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_argument_names_its_field_and_creates_no_agent() {
    let mut listed = vec![
        json!({"id": "levels-model",
               "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}],
               "default_reasoning_level": "medium"}),
        json!({"id": "plain-model", "reasoning": false}),
        json!({"id": "open-model"}),
        json!({"id": "gpt-5.4-mini"}),
        json!({"id": "gpt-5.4"}),
        json!({"id": "gpt-4o"}),
    ];
    listed.extend((0..20).map(|index| json!({ "id": format!("z-{index:02}") })));
    let listing = json!({ "data": listed }).to_string();
    let first = FakeServer::start(Vec::new()).await;
    let second = FakeServer::with_listings(
        vec![(assistant_events("Done."), Duration::ZERO); 3],
        vec![FakeListing::ok(&listing)],
    )
    .await;
    let (service, paths, _) =
        with_two_providers("subagents-model-refused", &first, &second, |config| {
            add_test_provider(config, "provider-3", "Third", "http://127.0.0.1:9/v1");
            let revision = config.providers().unwrap().revision;
            config
                .mutate_providers(revision, |registry| {
                    registry.providers[2].enabled = false;
                    registry.providers[1].models.insert(
                        "no-reasoning-model".into(),
                        AgentModelOverrideSettings {
                            reasoning_efforts: Some(Vec::new()),
                            ..AgentModelOverrideSettings::default()
                        },
                    );
                    Ok(())
                })
                .unwrap();
        })
        .await;
    list(&service, "provider-2").await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let not_enabled = |requested: &str| {
        format!("invalid tool argument provider: `{requested}` is not an enabled Provider; use one of `provider-1` (Fixture Provider), `provider-2` (Second)")
    };
    let on_second = |model: &str, effort: Option<&str>| {
        let mut arguments = json!({ "provider": "provider-2", "model": model });
        if let Some(effort) = effort {
            arguments["reasoning_effort"] = json!(effort);
        }
        arguments
    };
    let cases = [
        (json!({ "provider": "nope" }), not_enabled("nope")),
        (json!({ "provider": "provider-3" }), not_enabled("provider-3")),
        (json!({ "provider": "Third" }), not_enabled("Third")),
        (
            on_second("gpt-5-mini", None),
            "invalid tool argument model: `gpt-5-mini` is not a model Provider `provider-2` (Second) lists; closest: `gpt-5.4`, `gpt-5.4-mini`, `gpt-4o`, `levels-model`, `no-reasoning-model`, `open-model`, `plain-model`, `z-00`, `z-01`, `z-02`, `z-03`, `z-04`, `z-05`, `z-06`, `z-07`, `z-08`, `z-09`, `z-10`, `z-11`, `z-12` (and 7 more)".to_string(),
        ),
        (
            on_second("levels-model", Some("xhigh")),
            "invalid tool argument reasoning_effort: `xhigh` is not a reasoning effort `levels-model` takes; use one of low, medium (default), high".to_string(),
        ),
        (
            on_second("plain-model", Some("low")),
            "invalid tool argument reasoning_effort: `plain-model` takes no reasoning effort; omit `reasoning_effort`".to_string(),
        ),
        (
            on_second("no-reasoning-model", Some("low")),
            "invalid tool argument reasoning_effort: `no-reasoning-model` takes no reasoning effort; omit `reasoning_effort`".to_string(),
        ),
        (
            json!({ "reasoning_effort": "none" }),
            "invalid tool argument reasoning_effort: reasoning effort 'none' is not supported; omit the setting for Provider default".to_string(),
        ),
        (
            json!({ "model": 5 }),
            "invalid tool argument model: must be a string of one line, at most 256 bytes"
                .to_string(),
        ),
        (
            json!({ "model": "é".repeat(129) }),
            "invalid tool argument model: must be a string of one line, at most 256 bytes"
                .to_string(),
        ),
        (
            json!({ "provider": ["provider-2"] }),
            "invalid tool argument provider: must be a string of one line, at most 128 bytes"
                .to_string(),
        ),
    ];

    for (arguments, expected) in cases {
        let mut call_arguments = json!({ "task_name": "worker", "message": "Task worker." });
        for (key, value) in arguments.as_object().unwrap() {
            call_arguments[key] = value.clone();
        }
        let refused = call(
            &subagents,
            SubagentToolKind::SpawnAgent,
            parent,
            call_arguments,
        )
        .await
        .unwrap_err();
        assert_eq!(refused, AgentToolError::Failed(expected));
        assert!(subagents.agents(parent).await.unwrap().is_empty());
    }
    assert!(first.requests().is_empty() && second.requests().is_empty());

    let worker = spawned(
        &subagents,
        parent,
        "worker",
        on_second("levels-model", Some("high")),
    )
    .await;
    assert_eq!(
        runs_on(&worker),
        ("provider-2", "levels-model", Some("high"))
    );
    let open = spawned(
        &subagents,
        parent,
        "open",
        on_second("open-model", Some("turbo")),
    )
    .await;
    assert_eq!(runs_on(&open), ("provider-2", "open-model", Some("turbo")));
    assert_eq!(
        effort(&request_with(&second, "Task open.").await),
        Some("turbo")
    );
    // A role's effort is not the model's choice: never checked, and the
    // request drops what the model does not take.
    let role = spawned(
        &subagents,
        parent,
        "role",
        json!({ "agent_type": "fast", "provider": "provider-2", "model": "plain-model" }),
    )
    .await;
    assert_eq!(runs_on(&role), ("provider-2", "plain-model", Some("low")));
    assert_eq!(effort(&request_with(&second, "Task role.").await), None);
    finish(service, paths);
}

/// Arguments naming the parent's own pair, blank or `null`, select no
/// model: the agent is as one spawned without them, its request the same
/// but for its task; a new effort is one commit more, and so is a new
/// pair with its effort.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn an_equal_pair_selects_no_model_and_a_new_pair_takes_its_effort_in_one_commit() {
    let first = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 5]).await;
    let second = FakeServer::start(Vec::new()).await;
    let (service, paths, _) =
        with_two_providers("subagents-model-equal", &first, &second, |_| {}).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let plain = spawned(&subagents, parent, "plain", json!({})).await;
    let same = spawned(
        &subagents,
        parent,
        "same",
        json!({ "provider": "provider-1", "model": "fixture-model", "reasoning_effort": "medium" }),
    )
    .await;
    let blank = spawned(
        &subagents,
        parent,
        "blank",
        json!({ "provider": "  ", "model": "", "reasoning_effort": null, "agent_type": null }),
    )
    .await;
    let effort_only = spawned(
        &subagents,
        parent,
        "effort_only",
        json!({ "reasoning_effort": "low" }),
    )
    .await;
    let moved = spawned(
        &subagents,
        parent,
        "moved",
        json!({ "model": "other-model", "reasoning_effort": "low" }),
    )
    .await;

    let revision = |result: &Value| child(&service, result).revision.value();
    assert_eq!(
        runs_on(&plain),
        ("provider-1", "fixture-model", Some("medium"))
    );
    for result in [&same, &blank] {
        assert_eq!(runs_on(result), runs_on(&plain));
        assert_eq!(revision(result), revision(&plain));
    }
    assert_eq!(
        runs_on(&effort_only),
        ("provider-1", "fixture-model", Some("low"))
    );
    assert_eq!(revision(&effort_only), revision(&plain) + 1);
    assert_eq!(runs_on(&moved), ("provider-1", "other-model", Some("low")));
    assert_eq!(revision(&moved), revision(&plain) + 1);
    let comparable = |task_name: &str, request: Value| {
        let text = request
            .to_string()
            .replace(&format!("Task {task_name}."), "TASK");
        let mut request: Value = serde_json::from_str(&text).unwrap();
        request.as_object_mut().unwrap().remove("prompt_cache_key");
        request
    };
    let expected = comparable("plain", request_with(&first, "Task plain.").await);
    for task_name in ["same", "blank"] {
        let request = request_with(&first, &format!("Task {task_name}.")).await;
        assert_eq!(comparable(task_name, request), expected, "{task_name}");
    }
    finish(service, paths);
}

/// A model no listing is known for is passed to the Provider as given:
/// the spawn succeeds and the wait reports the Provider's refusal.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_model_without_a_listing_fails_at_its_first_request() {
    let first = FakeServer::start(vec![(
        http_error(
            404,
            r#"{"error":{"message":"The model `missing-model` does not exist"}}"#,
        ),
        Duration::ZERO,
    )])
    .await;
    let second = FakeServer::start(Vec::new()).await;
    let (service, paths, _) =
        with_two_providers("subagents-model-unknown", &first, &second, |_| {}).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let result = call(
        &subagents,
        SubagentToolKind::SpawnAgent,
        parent,
        json!({ "task_name": "missing", "message": "Task missing.", "model": "missing-model" }),
    )
    .await
    .unwrap();

    assert_eq!(result["model"], "missing-model");
    let WaitOutcome::Updates(updates) = subagents
        .wait(parent, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap()
    else {
        panic!("the agent's turn is not reported");
    };
    assert!(
        matches!(&updates[0].status, AgentRunStatus::Errored(error) if error.contains("missing-model")),
        "{:?}",
        updates[0].status
    );
    finish(service, paths);
}

fn png() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::new_rgba8(1, 1)
        .write_to(&mut output, ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

fn staged_image(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> AgentImageAttachmentRef {
    service
        .core
        .stage_image_attachment(
            session_id,
            PreparedAgentImage {
                bytes: png(),
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "fixture.png".to_string(),
                detail: AgentImageDetail::High,
            },
        )
        .unwrap()
}

/// An agent spawned on another Provider's model, forking a history that
/// holds an image: its request reaches that Provider with the parent's
/// history, under the window its listing reports and without the image
/// the model does not read, while the parent stays on its own pair; after
/// a restart, a follow-up task still runs there.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_on_another_provider_forks_the_history_there_and_keeps_its_pair() {
    let first = FakeServer::start(vec![
        (assistant_events("Planned."), Duration::ZERO),
        (
            tool_calls_events(&[(
                "call-followup",
                "followup_task",
                r#"{"target":"remote","message":"Again."}"#,
            )]),
            Duration::ZERO,
        ),
        (assistant_events("Sent."), Duration::ZERO),
    ])
    .await;
    let second = FakeServer::with_listings(
        vec![
            (assistant_events("Remote done."), Duration::ZERO),
            (assistant_events("Again done."), Duration::ZERO),
        ],
        vec![FakeListing::ok(
            r#"{"data":[{"id":"remote-model","context_length":32768,"input_modalities":["text"]}]}"#,
        )],
    )
    .await;
    let (service, paths, config) =
        with_two_providers("subagents-model-elsewhere", &first, &second, |_| {}).await;
    let subagents_now = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let image = staged_image(&service, parent);
    service
        .start_turn(
            parent,
            crate::AgentTurnInput {
                images: vec![image],
                ..crate::AgentTurnInput::from("Plan the work.")
            },
        )
        .await
        .unwrap();
    list(&service, "provider-2").await;

    let result = call(
        &subagents_now,
        SubagentToolKind::SpawnAgent,
        parent,
        json!({
            "task_name": "remote",
            "message": "Task remote.",
            "provider": "provider-2",
            "model": "remote-model",
        }),
    )
    .await
    .unwrap();
    settle(&subagents_now, parent).await;

    assert_eq!(
        runs_on(&result),
        ("provider-2", "remote-model", Some("medium"))
    );
    let parent_request = body(&first, 0);
    assert!(parent_request.contains("\"input_image\""));
    let request = request_with(&second, "Task remote.").await;
    assert_eq!(request["model"], "remote-model");
    let text = request.to_string();
    assert!(text.contains("Plan the work.") && text.contains("Planned."));
    assert!(!text.contains("\"input_image\""), "{text}");
    let agent = child(&service, &result);
    assert!(!agent.input_capabilities.image_input);
    let window = service
        .session_model_metadata(agent.id)
        .unwrap()
        .context_window;
    assert_eq!(
        window.map(|window| (window.value, window.source)),
        Some((32768, ModelMetadataSource::Reported))
    );
    let parent_now = service.snapshot(parent).unwrap();
    assert_eq!(
        (parent_now.provider_id.as_str(), parent_now.model.as_str()),
        ("provider-1", "fixture-model")
    );
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let reopened = reopen_service(config, test_host(&paths), &paths);
    reopened.start_plugins().await;
    let subagents_then = subagents(&reopened).await;
    let outcome = reopened
        .start_turn(parent, "Follow up.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Sent.");
    assert!(body(&first, 2).contains("started a new turn"));
    settle(&subagents_then, parent).await;
    let again = request_with(&second, "Again.").await;
    assert_eq!(again["model"], "remote-model");
    let agent = reopened.snapshot(agent.id).unwrap();
    assert_eq!(
        (agent.provider_id.as_str(), agent.model.as_str()),
        ("provider-2", "remote-model")
    );
    finish(reopened, paths);
}
