//! Session goals end to end, over the scripted model: the model creates and
//! completes a goal, an active goal keeps its turn going once per final
//! answer and stops at its budget, the user pauses, resumes, replaces and
//! clears it, a fork keeps it and a child does not, and with the `goal` row
//! off the tools and the calls are gone.

use hanafubuki_agent_kernel::{
    AgentContinuationHookOutcome, AgentError, AgentGoal, AgentGoalOrigin, AgentGoalStatus,
    AgentSessionHeadForkRequest, AgentSessionMode,
};

use super::host::AgentPluginState;
use super::*;
use crate::AgentGoalBudgets;

async fn goal_service(
    name: &str,
    responses: Vec<(Vec<u8>, Duration)>,
) -> (
    AgentApplicationService,
    TestPaths,
    FakeServer,
    AgentSessionId,
) {
    let server = FakeServer::start(responses).await;
    let (service, paths) = configured_service(name, &server.base_url);
    service.start_plugins().await;
    let session_id = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap()
        .id;
    (service, paths, server, session_id)
}

fn answers(responses: Vec<Vec<u8>>) -> Vec<(Vec<u8>, Duration)> {
    responses
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect()
}

/// The outcomes of the goal's continuation hook, in order.
fn goal_continuations(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> Vec<AgentContinuationHookOutcome> {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ContinuationHookCompleted {
                provider_id,
                outcome,
                ..
            } if provider_id == "goal" => Some(outcome),
            _ => None,
        })
        .collect()
}

fn goal_updates(service: &AgentApplicationService, session_id: AgentSessionId) -> usize {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::SessionGoalUpdated { .. }))
        .count()
}

fn request_text(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

async fn wait_for_goal_row(service: &AgentApplicationService, state: AgentPluginState) {
    let deadline = Instant::now() + TEST_WAIT;
    while service
        .plugin_status()
        .into_iter()
        .any(|status| status.name == "goal" && status.state != state)
    {
        assert!(Instant::now() < deadline, "the goal row is not {state:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The model creates a goal when asked; its first final answer continues
/// the turn with the continuation, and once the model marks the goal
/// complete the next final answer ends it.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_goal_the_model_creates_keeps_its_turn_going_until_complete() {
    let (service, paths, server, session_id) = goal_service(
        "goal-model",
        answers(vec![
            tool_events("create_goal", r#"{"objective":"Ship the <release>"}"#),
            assistant_events("Goal created."),
            tool_events("get_goal", "{}"),
            tool_events("update_goal", r#"{"status":"complete"}"#),
            assistant_events("Shipped."),
        ]),
    )
    .await;

    let outcome = service
        .start_turn(
            session_id,
            "Make shipping the release a goal.".to_string().into(),
        )
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Shipped.");
    assert_eq!(server.requests().len(), 5);
    let continued = request_text(&server, 2);
    assert!(continued.contains("Continue working toward the active session goal."));
    assert!(
        continued.contains("Ship the &lt;release&gt;"),
        "{continued}"
    );
    let read = request_text(&server, 3);
    assert!(read.contains(r#"\"status\":\"active\""#), "{read}");
    let goal = service.goal(session_id).await.unwrap().unwrap();
    assert_eq!(goal.objective, "Ship the <release>");
    assert_eq!(goal.status, AgentGoalStatus::Complete);
    assert_eq!(goal.origin, AgentGoalOrigin::Model);
    assert!(goal.tokens_used > 0, "{goal:?}");
    assert_eq!(
        goal_continuations(&service, session_id),
        vec![AgentContinuationHookOutcome::Continued]
    );
    assert_eq!(goal_updates(&service, session_id), 3);
    finish(service, paths);
}

/// The model may not create a goal over an unfinished one, nor resume one.
#[tokio::test(flavor = "multi_thread")]
async fn the_model_cannot_replace_an_unfinished_goal() {
    let (service, paths, server, session_id) = goal_service(
        "goal-model-refused",
        answers(vec![
            tool_events("create_goal", r#"{"objective":"Another"}"#),
            tool_events("update_goal", r#"{"status":"active"}"#),
            tool_events("update_goal", r#"{"status":"paused"}"#),
            assistant_events("Paused."),
        ]),
    )
    .await;
    service
        .set_goal(session_id, "First".to_string(), AgentGoalBudgets::default())
        .await
        .unwrap();

    service
        .start_turn(session_id, "Try.".to_string().into())
        .await
        .unwrap();

    assert!(request_text(&server, 1).contains("this session has an unfinished goal"));
    assert!(request_text(&server, 2).contains("can only mark the existing goal complete"));
    let goal = service.goal(session_id).await.unwrap().unwrap();
    assert_eq!(
        (goal.objective.as_str(), goal.status, goal.origin),
        ("First", AgentGoalStatus::Paused, AgentGoalOrigin::User)
    );
    assert!(goal_continuations(&service, session_id).is_empty());
    finish(service, paths);
}

/// A goal the user sets is in the turn's context; once the turn's tokens
/// spend its budget, the goal is budget-limited and the turn gets one more
/// answer to wrap up.
#[tokio::test(flavor = "multi_thread")]
async fn a_spent_budget_limits_the_goal_and_wraps_up_the_turn() {
    let (service, paths, server, session_id) = goal_service(
        "goal-budget",
        answers(vec![
            assistant_events("Working."),
            assistant_events("Wrapped up."),
        ]),
    )
    .await;
    let set = service
        .set_goal(
            session_id,
            "  Port the parser  ".to_string(),
            AgentGoalBudgets {
                token_budget: Some(1),
                time_budget_seconds: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(set.objective, "Port the parser");

    let outcome = service
        .start_turn(session_id, "Go.".to_string().into())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Wrapped up.");
    assert_eq!(server.requests().len(), 2);
    let first = request_text(&server, 0);
    assert!(
        first.contains(r#"User set the goal: \"Port the parser\""#),
        "{first}"
    );
    assert!(first.contains("Token budget: 1."));
    assert!(request_text(&server, 1).contains("has reached its budget"));
    let goal = service.goal(session_id).await.unwrap().unwrap();
    assert_eq!(goal.status, AgentGoalStatus::BudgetLimited);
    assert!(goal.tokens_used >= 1);
    // A spent budget outranks resuming.
    assert_eq!(
        service.resume_goal(session_id).await.unwrap().status,
        AgentGoalStatus::BudgetLimited
    );
    assert_eq!(
        goal_continuations(&service, session_id),
        vec![AgentContinuationHookOutcome::Continued]
    );
    finish(service, paths);
}

/// The user replaces the objective while a turn works on the old one: the
/// turn's next final answer is told so.
#[tokio::test(flavor = "multi_thread")]
async fn an_objective_replaced_mid_turn_is_announced() {
    let (service, paths, server, session_id) = goal_service(
        "goal-replaced",
        vec![
            // The objective is replaced while this answer is on its way.
            (assistant_events("Working on A."), Duration::from_secs(5)),
            (
                tool_events("update_goal", r#"{"status":"complete"}"#),
                Duration::ZERO,
            ),
            (assistant_events("Done with B."), Duration::ZERO),
        ],
    )
    .await;
    service
        .set_goal(
            session_id,
            "Objective A".to_string(),
            AgentGoalBudgets::default(),
        )
        .await
        .unwrap();

    let (outcome, replaced) = tokio::join!(
        service.start_turn(session_id, "Go.".to_string().into()),
        async {
            while server.requests().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            service
                .set_goal(
                    session_id,
                    "Objective B".to_string(),
                    AgentGoalBudgets::default(),
                )
                .await
        }
    );

    replaced.unwrap();
    assert_eq!(outcome.unwrap().final_answer, "Done with B.");
    let updated = request_text(&server, 1);
    assert!(
        updated.contains("objective was edited by the user") && updated.contains("Objective B"),
        "{updated}"
    );
    assert_eq!(
        service.goal(session_id).await.unwrap().unwrap().status,
        AgentGoalStatus::Complete
    );
    finish(service, paths);
}

/// A paused goal does not continue; resumed it is active again; cleared it
/// is gone, and a second clear or a pause without a goal says so.
#[tokio::test(flavor = "multi_thread")]
async fn pause_resume_and_clear() {
    let (service, paths, server, session_id) =
        goal_service("goal-pause", answers(vec![assistant_events("Answered.")])).await;
    service
        .set_goal(
            session_id,
            "Refactor".to_string(),
            AgentGoalBudgets::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        service.pause_goal(session_id).await.unwrap().status,
        AgentGoalStatus::Paused
    );
    service
        .start_turn(session_id, "Unrelated question.".to_string().into())
        .await
        .unwrap();
    assert_eq!(server.requests().len(), 1);
    assert!(request_text(&server, 0).contains("Goal status: paused."));
    assert_eq!(
        service.resume_goal(session_id).await.unwrap().status,
        AgentGoalStatus::Active
    );
    assert!(service.clear_goal(session_id).await.unwrap());
    assert_eq!(service.goal(session_id).await.unwrap(), None);
    assert!(!service.clear_goal(session_id).await.unwrap());
    assert!(matches!(
        service.pause_goal(session_id).await,
        Err(AgentError::InvalidInput(_))
    ));
    assert!(goal_continuations(&service, session_id).is_empty());
    finish(service, paths);
}

/// An objective is trimmed, not blank and bounded; budgets are positive.
#[tokio::test(flavor = "multi_thread")]
async fn a_goal_is_validated_at_the_facade() {
    let (service, paths, _server, session_id) = goal_service("goal-validated", Vec::new()).await;
    for (objective, budgets) in [
        ("   ".to_string(), AgentGoalBudgets::default()),
        (
            "x".repeat(crate::MAXIMUM_GOAL_OBJECTIVE_CHARS + 1),
            AgentGoalBudgets::default(),
        ),
        (
            "Fine".to_string(),
            AgentGoalBudgets {
                token_budget: None,
                time_budget_seconds: Some(0),
            },
        ),
    ] {
        assert!(matches!(
            service.set_goal(session_id, objective, budgets).await,
            Err(AgentError::InvalidInput(_))
        ));
    }
    assert_eq!(service.goal(session_id).await.unwrap(), None);
    finish(service, paths);
}

/// A fork keeps its source's goal; a side conversation, a child, has none
/// and is refused one.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_keeps_the_goal_and_a_child_does_not() {
    let (service, paths, _server, session_id) =
        goal_service("goal-fork", answers(vec![assistant_events("Hello.")])).await;
    service
        .start_turn(session_id, "Hi.".to_string().into())
        .await
        .unwrap();
    service
        .set_goal(
            session_id,
            "Keep going".to_string(),
            AgentGoalBudgets {
                token_budget: Some(500),
                time_budget_seconds: Some(60),
            },
        )
        .await
        .unwrap();
    let goal: AgentGoal = service.pause_goal(session_id).await.unwrap();
    let source = service.snapshot(session_id).unwrap();

    let fork = service
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session_id,
            source_generation: source.generation,
            source_revision: source.revision,
        })
        .unwrap();

    assert_eq!(service.goal(fork.session.id).await.unwrap(), Some(goal));
    let side = service.open_side_conversation(session_id).unwrap();
    assert_eq!(service.goal(side.id).await.unwrap(), None);
    assert!(matches!(
        service
            .set_goal(
                side.id,
                "Side goal".to_string(),
                AgentGoalBudgets::default()
            )
            .await,
        Err(AgentError::InvalidInput(_))
    ));
    finish(service, paths);
}

/// With the `goal` row off the tools leave the catalog and every goal call
/// is unavailable; the goal stays stored and returns with the row.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_goal_plugin_off_goals_are_unavailable() {
    let (service, paths, server, session_id) =
        goal_service("goal-off", answers(vec![assistant_events("Plain.")])).await;
    service
        .set_goal(
            session_id,
            "Stored".to_string(),
            AgentGoalBudgets::default(),
        )
        .await
        .unwrap();
    service
        .set_plugin_enabled("goal".into(), false)
        .await
        .unwrap();
    wait_for_goal_row(&service, AgentPluginState::Disabled).await;

    let unavailable = AgentError::FeatureUnavailable {
        feature: "goal".to_string(),
        reason: "the goal Agent plugin is unavailable: it is turned off".to_string(),
    };
    assert_eq!(service.goal(session_id).await.unwrap_err(), unavailable);
    assert_eq!(
        service.clear_goal(session_id).await.unwrap_err(),
        unavailable
    );
    service
        .start_turn(session_id, "Hello.".to_string().into())
        .await
        .unwrap();
    let request = request_text(&server, 0);
    assert!(!request.contains("get_goal") && !request.contains("User set the goal"));
    assert!(goal_continuations(&service, session_id).is_empty());

    service
        .set_plugin_enabled("goal".into(), true)
        .await
        .unwrap();
    wait_for_goal_row(&service, AgentPluginState::Active).await;
    assert_eq!(
        service.goal(session_id).await.unwrap().unwrap().objective,
        "Stored"
    );
    finish(service, paths);
}
