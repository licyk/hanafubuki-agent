//! Where agents are listed, each names the Provider, model and reasoning
//! effort its session runs on now: `list_agents` for the model and the
//! facade's `subagents` for a host, for agents spawned with and without
//! `provider`, `model` and `reasoning_effort`, after a person changes an
//! agent's model or effort, and for agents of an earlier run, read without
//! a turn.

use hanafubuki_agent_kernel::ModelReasoningEffort;

use super::models::{spawned, with_two_providers};
use super::*;
use crate::test_support::reopen_service;
use crate::{AgentSessionModelSelectionRequest, AgentSessionReasoningEffortRequest};

/// `list_agents`' entries: name, role, status, Provider, model, effort.
async fn listed(
    subagents: &Arc<SubagentService>,
    parent: AgentSessionId,
) -> Vec<(String, String, String, String, String, Value)> {
    let listed = call(subagents, SubagentToolKind::ListAgents, parent, json!({}))
        .await
        .unwrap();
    listed["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|agent| {
            let text = |key: &str| agent[key].as_str().unwrap().to_string();
            (
                text("agent_name"),
                text("agent_type"),
                text("agent_status"),
                text("provider"),
                text("model"),
                agent["reasoning_effort"].clone(),
            )
        })
        .collect()
}

/// The facade's statuses: title, Provider, model, effort.
async fn statuses(
    service: &AgentApplicationService,
    parent: AgentSessionId,
) -> Vec<(String, String, String, Option<ModelReasoningEffort>)> {
    service
        .subagents(parent)
        .await
        .unwrap()
        .into_iter()
        .map(|status| {
            (
                status.title,
                status.provider_id,
                status.model,
                status.reasoning_effort,
            )
        })
        .collect()
}

fn entry(
    name: &str,
    provider: &str,
    model: &str,
    effort: Value,
) -> (String, String, String, String, String, Value) {
    (
        name.to_string(),
        "default".to_string(),
        "completed".to_string(),
        provider.to_string(),
        model.to_string(),
        effort,
    )
}

fn status(
    name: &str,
    provider: &str,
    model: &str,
    effort: Option<ModelReasoningEffort>,
) -> (String, String, String, Option<ModelReasoningEffort>) {
    (
        name.to_string(),
        provider.to_string(),
        model.to_string(),
        effort,
    )
}

/// Agents spawned on the parent's pair, on another model with an effort,
/// and on another Provider are listed with what each runs on; a model or
/// effort a person sets on an agent afterwards is what is listed next; and
/// after a restart the agents of the earlier run are listed the same,
/// without a request to either Provider.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn agents_are_listed_with_the_provider_model_and_effort_they_run_on() {
    let first = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 2]).await;
    let second = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let (service, paths, config) =
        with_two_providers("subagents-listing", &first, &second, |_| {}).await;
    let subagents_now = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let plain = spawned(&subagents_now, parent, "plain", json!({})).await;
    spawned(
        &subagents_now,
        parent,
        "quick",
        json!({ "model": "quick-model", "reasoning_effort": "high" }),
    )
    .await;
    spawned(
        &subagents_now,
        parent,
        "remote",
        json!({ "provider": "Second", "model": "remote-model" }),
    )
    .await;

    assert_eq!(
        listed(&subagents_now, parent).await,
        [
            entry("plain", "provider-1", "fixture-model", json!("medium")),
            entry("quick", "provider-1", "quick-model", json!("high")),
            entry("remote", "provider-2", "remote-model", json!("medium")),
        ]
    );
    assert_eq!(
        statuses(&service, parent).await,
        [
            status(
                "plain",
                "provider-1",
                "fixture-model",
                Some(ModelReasoningEffort::Medium)
            ),
            status(
                "quick",
                "provider-1",
                "quick-model",
                Some(ModelReasoningEffort::High)
            ),
            status(
                "remote",
                "provider-2",
                "remote-model",
                Some(ModelReasoningEffort::Medium)
            ),
        ]
    );

    // A person moves `plain` to another pair and `quick` to the Provider's
    // default effort.
    let agents = subagents_now.agents(parent).await.unwrap();
    assert_eq!(plain["agent_id"], agents[0].id.to_string());
    let moved = service.snapshot(agents[0].id).unwrap();
    service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: moved.id,
            generation: moved.generation,
            revision: moved.revision,
            provider_id: "provider-2".to_string(),
            model: "moved-model".to_string(),
        })
        .await
        .unwrap();
    let defaulted = service.snapshot(agents[1].id).unwrap();
    service
        .set_session_reasoning_effort(AgentSessionReasoningEffortRequest {
            session_id: defaulted.id,
            generation: defaulted.generation,
            revision: defaulted.revision,
            effort: None,
        })
        .await
        .unwrap();
    let now = [
        entry("plain", "provider-2", "moved-model", json!("medium")),
        entry("quick", "provider-1", "quick-model", Value::Null),
        entry("remote", "provider-2", "remote-model", json!("medium")),
    ];
    assert_eq!(listed(&subagents_now, parent).await, now);
    let statuses_now = [
        status(
            "plain",
            "provider-2",
            "moved-model",
            Some(ModelReasoningEffort::Medium),
        ),
        status("quick", "provider-1", "quick-model", None),
        status(
            "remote",
            "provider-2",
            "remote-model",
            Some(ModelReasoningEffort::Medium),
        ),
    ];
    assert_eq!(statuses(&service, parent).await, statuses_now);
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let requests = (first.requests().len(), second.requests().len());
    let reopened = reopen_service(config, test_host(&paths), &paths);
    reopened.start_plugins().await;
    let subagents_then = subagents(&reopened).await;
    assert_eq!(listed(&subagents_then, parent).await, now);
    assert_eq!(statuses(&reopened, parent).await, statuses_now);
    assert_eq!(
        (first.requests().len(), second.requests().len()),
        requests,
        "listing starts no turn"
    );
    finish(reopened, paths);
}
