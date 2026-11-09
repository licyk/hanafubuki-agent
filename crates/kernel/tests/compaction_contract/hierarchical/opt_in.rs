//! The opt-in features (`[compaction.hierarchical]` `absorb`,
//! `contentStore`, `crush`, `rules`): each changes the request only while
//! it is on, its tool is planned only then, and what its tool records is
//! committed with the tool's output.

use hanafubuki_agent_kernel::test_support::FakeRegisteredExchange;
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};

use super::*;

/// A task log of about 5 000 tokens: over the features' size gates.
static LOG: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| "the nightly build step finished without a warning; ".repeat(400));

const ABSORB_PROMPT: &str = "INSTANT TOOL-RESULT ABSORPTION (absorb)";
const RULES_PROMPT: &str = "Use the acp_rule tool to record short, principle-level reminders";

#[derive(Default)]
struct Features {
    absorb: bool,
    content_store: bool,
    crush: bool,
    rules: bool,
}

/// A turn as the facade admits one with `features` on.
fn turn_with(features: &Features, text: &str) -> AgentTurnInput {
    let mut input = AgentTurnInput::from(text);
    input.turn = TurnExtensions::default().with(Arc::new(AgentHierarchicalCompression {
        absorb: features.absorb,
        content_store: features.content_store,
        crush: features.crush,
        rules: features.rules,
        ..tuning()
    }));
    input
}

/// The model reads the task log once (refs: the prompt m00001, the call
/// m00002, the log m00003), then makes `then`.
async fn read_log(
    log: &str,
    then: impl IntoIterator<Item = ScriptedResponse>,
) -> (AgentService, ScriptedModel, AgentSessionId) {
    let model =
        ScriptedModel::new(std::iter::once(call("read_task_log", "log", json!({}))).chain(then));
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "read_task_log",
        json!({}),
        ToolOutput {
            content: vec![ToolOutputContent::Text {
                text: log.to_string(),
            }],
            structured_content: None,
            is_error: false,
        },
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(hierarchical_config()).unwrap();
    (service, model, session.id)
}

fn content_store(service: &AgentService, session: AgentSessionId) -> AgentContentStore {
    service
        .test_content_store(session)
        .unwrap()
        .expect("the session has a content store")
}

/// With every feature off the tool result reaches the model as it is, and
/// neither the tools nor the instructions change: what `hierarchical` did
/// before them.
#[tokio::test]
async fn the_features_off_change_nothing() {
    let (service, model, session) = read_log(&LOG, [response("done", "Read.", 3_000)]).await;
    service
        .start_turn(session, turn_with(&Features::default(), "Read the log."))
        .await
        .unwrap();

    let after = &model.requests()[1];
    assert!(text(after).contains(LOG.as_str()));
    assert!(!text(after).contains("[ACP absorb]"));
    assert!(!text(after).contains("[acp-stored"));
    let names = tool_names(after);
    for tool in ["absorb", "acp_retrieve", "acp_rule"] {
        assert!(!names.contains(&tool.to_string()), "{tool}");
    }
    assert!(!after.instructions.contains(ABSORB_PROMPT));
    assert!(!after.instructions.contains(RULES_PROMPT));
    assert_eq!(service.test_content_store(session).unwrap(), None);
}

/// `absorb`: a large result asks the model to distil it; once it has, the
/// call and the result are gone from the next request and the record is in
/// the state committed with the `absorb` call's output.
#[tokio::test]
async fn absorb_hides_a_large_result_the_model_distilled() {
    let summary = "The nightly build log: one line repeated, no warning and no failure.";
    let (service, model, session) = read_log(
        &LOG,
        [
            call(
                "absorb",
                "absorb",
                json!({ "ref": "m00003", "summary": summary }),
            ),
            response("done", "Read.", 3_000),
        ],
    )
    .await;
    let features = Features {
        absorb: true,
        ..Features::default()
    };
    service
        .start_turn(session, turn_with(&features, "Read the log."))
        .await
        .unwrap();

    let requests = model.requests();
    let asked = &requests[1];
    assert!(text(asked).contains("[ACP absorb]"));
    assert!(text(asked).contains("m00003"));
    assert!(tool_names(asked).contains(&"absorb".to_string()));
    assert!(asked.instructions.contains(ABSORB_PROMPT));
    let absorbed = text(&requests[2]);
    assert!(!absorbed.contains(LOG.as_str()));
    assert!(!absorbed.contains("[ACP absorb]"));
    assert!(absorbed.contains(summary));
    assert!(tool_output(&service, session, "absorb").contains("absorbed m00003"));
    let state = state(&service, session);
    assert_eq!(state.absorbed.len(), 1);
    assert_eq!(state.absorbed[0].tool_call_id, "log");
    assert_eq!(state.absorbed[0].absorb_call_id.as_deref(), Some("absorb"));
    // The history keeps the original.
    assert!(tool_output(&service, session, "log").contains("nightly build step"));
}

/// `crush` (with `absorb`): a large JSON result folds deterministically,
/// and once it is under absorption's gate the model is not asked for it.
#[tokio::test]
async fn crush_folds_a_large_json_result_before_absorb_asks() {
    let rows =
        serde_json::to_string(&vec![json!({ "task": "build", "status": "ok" }); 800]).unwrap();
    let (service, model, session) = read_log(&rows, [response("done", "Read.", 3_000)]).await;
    let features = Features {
        absorb: true,
        crush: true,
        ..Features::default()
    };
    service
        .start_turn(session, turn_with(&features, "Read the rows."))
        .await
        .unwrap();

    let after = text(&model.requests()[1]);
    assert!(after.contains("__acp_crush"), "{after}");
    assert!(!after.contains("[ACP absorb]"));
    assert!(after.len() < rows.len() / 4);
    assert!(tool_output(&service, session, "log").contains("\\\"status\\\":\\\"ok\\\""));
}

/// The content store: a large result is replaced on arrival by a
/// placeholder, the same bytes on every request; `acp_retrieve` returns the
/// original (inline here: the ephemeral store keeps no export files), counts
/// it, and a restarted kernel reads the store back.
#[tokio::test]
async fn the_content_store_replaces_a_large_result_and_retrieves_it() {
    let model = ScriptedModel::new([
        call("read_task_log", "log", json!({})),
        call("acp_retrieve", "retrieve", json!({ "ref": "m00003" })),
        call("acp_retrieve", "made-up", json!({ "ref": "m00099" })),
        response("done", "Read.", 3_000),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "read_task_log",
        json!({}),
        ToolOutput {
            content: vec![ToolOutputContent::Text {
                text: LOG.to_string(),
            }],
            structured_content: None,
            is_error: false,
        },
    )])
    .await;
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(model.clone()),
        tools.registries(),
        store.clone(),
    );
    let session = service.create_session(hierarchical_config()).unwrap().id;
    let features = Features {
        content_store: true,
        ..Features::default()
    };
    service
        .start_turn(session, turn_with(&features, "Read the log."))
        .await
        .unwrap();

    let requests = model.requests();
    let arrived = text(&requests[1]);
    assert!(!arrived.contains(LOG.as_str()));
    let start = arrived
        .find("[acp-stored #m00003 · ")
        .expect("the placeholder");
    let end = start + arrived[start..].find("returns the full text").unwrap();
    let placeholder = &arrived[start..end];
    for request in &requests[1..] {
        assert!(text(request).contains(placeholder));
        assert!(tool_names(request).contains(&"acp_retrieve".to_string()));
    }
    let retrieved = tool_output(&service, session, "retrieve");
    assert!(retrieved.contains("[acp-retrieved #m00003"), "{retrieved}");
    assert!(retrieved.contains("could not be exported"));
    assert!(retrieved.contains("nightly build step"));
    assert!(tool_output(&service, session, "made-up").contains("not found"));
    let stats = state(&service, session).stats;
    assert_eq!((stats.stored_count, stats.retrieval_count), (1, 1));
    let before = content_store(&service, session);
    assert_eq!(before.by_ref["m00003"].raw_id.split(':').count(), 2);
    assert_eq!(before.retrieve("m00003").unwrap().1, LOG.as_str());

    let restarted_model = ScriptedModel::new([response("again", "Still here.", 3_000)]);
    let restarted = AgentService::without_default_model_with_history_store(
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    restarted
        .recover_sessions_with_test_model(Arc::new(restarted_model.clone()))
        .unwrap();
    assert_eq!(content_store(&restarted, session), before);
    restarted
        .start_turn(session, turn_with(&features, "Are you there?"))
        .await
        .unwrap();
    assert!(text(&restarted_model.requests()[0]).contains(placeholder));
}

/// A fold stores the originals of the messages it covered, so a folded ref
/// can be retrieved (inline: it is small); a store that lost it would say
/// the ref has no stored original.
#[tokio::test]
async fn a_fold_stores_the_originals_it_covered() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold", "m00002–m00004", "Both answers"),
        call("acp_retrieve", "folded", json!({ "ref": "m00003" })),
        response("three", "Retrieved.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap().id;
    let features = Features {
        content_store: true,
        ..Features::default()
    };
    service
        .start_turn(session, turn_with(&features, "first"))
        .await
        .unwrap();
    service
        .start_turn(session, turn_with(&features, "second"))
        .await
        .unwrap();
    service
        .start_turn(session, turn_with(&features, "Fold and look again."))
        .await
        .unwrap();

    let store = content_store(&service, session);
    for r#ref in ["m00002", "m00003", "m00004"] {
        assert!(store.has_ref(r#ref), "{ref}");
    }
    let folded = tool_output(&service, session, "folded");
    assert!(folded.contains("[acp-retrieved #m00003"), "{folded}");
    assert!(folded.contains("<acp-retrieved ref=\\\"m00003\\\">\\nsecond\\n</acp-retrieved>"));
    assert!(!folded.contains("could not be exported"));
}

/// `acp_rule`: a recorded rule is in the instructions of every request
/// after it, below the rules' usage, with the state committed with its
/// output.
#[tokio::test]
async fn a_recorded_rule_is_in_the_instructions() {
    let model = ScriptedModel::new([
        call(
            "acp_rule",
            "rule",
            json!({ "rule": "Run the tests before every commit." }),
        ),
        response("done", "Recorded.", 3_000),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap().id;
    let features = Features {
        rules: true,
        ..Features::default()
    };
    service
        .start_turn(session, turn_with(&features, "Remember to test."))
        .await
        .unwrap();

    let requests = model.requests();
    let listed = "# Persistent rules (recorded via acp_rule — kept across compression)\n- [rule1] Run the tests before every commit.";
    assert!(requests[0].instructions.contains(RULES_PROMPT));
    assert!(!requests[0].instructions.contains(listed));
    assert!(tool_names(&requests[0]).contains(&"acp_rule".to_string()));
    assert!(requests[1].instructions.ends_with(listed));
    assert!(tool_output(&service, session, "rule").contains("Recorded rule1"));
    let rules = state(&service, session).rules;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].text, "Run the tests before every commit.");
}

/// An interrupted turn is booked into the nudge once the context grows past
/// what it was sent with (`absorb_aborted_turn_growth`), not before.
#[tokio::test]
async fn an_interrupted_turn_is_booked_once_the_context_grows() {
    let model = ScriptedModel::new([
        response("one", &FIRST_ANSWER, 1_000),
        ScriptedResponse::WaitForCancellation,
        response("three", "Back.", 3_000),
        response("four", "Again.", 4_000),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap().id;
    service.start_turn(session, turn("first")).await.unwrap();
    let running = service.clone();
    let task = tokio::spawn(async move { running.start_turn(session, turn("wait")).await });
    while model.requests().len() < 2 {
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let interrupted = test_session_view(&service, session)
        .unwrap()
        .runtime
        .turn
        .expect("the turn runs");
    service
        .cancel_turn_confirmed(session, interrupted.id, interrupted.generation)
        .await
        .unwrap();
    let _ = task.await;

    // Sent with the same count as the interrupted turn: nothing yet.
    service.start_turn(session, turn("third")).await.unwrap();
    assert_eq!(state(&service, session).nudge.absorb_count, 0);
    // The count moved past it.
    service.start_turn(session, turn("fourth")).await.unwrap();
    assert_eq!(state(&service, session).nudge.absorb_count, 1);
}

/// A session that left `hierarchical` keeps projecting its blocks, but no
/// feature acts without its tool: a large result arrives as it is, with no
/// placeholder and no absorb demand.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_features_stop_with_the_strategy() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
        call("read_task_log", "log", json!({})),
        response("four", "Read.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "read_task_log",
        json!({}),
        ToolOutput {
            content: vec![ToolOutputContent::Text {
                text: LOG.to_string(),
            }],
            structured_content: None,
            is_error: false,
        },
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(hierarchical_config()).unwrap().id;
    let features = Features {
        absorb: true,
        content_store: true,
        crush: true,
        rules: true,
    };
    for prompt in ["first", "second", "Fold the first exchange."] {
        service
            .start_turn(session, turn_with(&features, prompt))
            .await
            .unwrap();
    }

    assert!(
        super::switching::set_strategy(&service, session, AgentCompactionStrategy::Standard).await
    );
    service
        .start_turn(session, turn_with(&features, "Read the log."))
        .await
        .unwrap();
    let after = model.requests().last().unwrap().clone();
    assert!(!text(&after).contains("a long first answer"));
    assert!(text(&after).contains(LOG.as_str()));
    assert!(!text(&after).contains("[acp-stored"));
    assert!(!text(&after).contains("[ACP absorb]"));
    assert!(!after.instructions.contains(RULES_PROMPT));
}
