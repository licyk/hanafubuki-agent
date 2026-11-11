use hanafubuki_agent_compression::REF_FLOOR_ID;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentHistoryStore, AgentSessionForkRequest,
    AgentSessionOrigin, AgentSessionPersistence, AgentTurnId,
};

use super::*;

/// A session whose third turn folded both earlier answers (`b1` over
/// `m00002`, `b2` over `m00004`) and whose fourth went on; the ids of its
/// four turns.
async fn folded_session(
    store: Arc<EphemeralAgentHistoryStore>,
    more: impl IntoIterator<Item = ScriptedResponse>,
) -> (
    AgentService,
    ScriptedModel,
    AgentSessionId,
    Vec<AgentTurnId>,
) {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        compress("fold-2", "m00004–m00004", "Second exchange"),
        response("three", "Folded.", 3_000),
        response("four", "Going on.", 3_000),
    ]);
    responses.extend(more);
    let model = ScriptedModel::new(responses);
    let service = AgentService::with_history_store(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
        store,
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    let mut turns = Vec::new();
    for prompt in ["first", "second", "Fold both exchanges.", "Go on."] {
        service.start_turn(session.id, turn(prompt)).await.unwrap();
        turns.push(service.snapshot(session.id).unwrap().turn.unwrap().id);
    }
    assert_eq!(state(&service, session.id).active_blocks().count(), 2);
    (service, model, session.id, turns)
}

fn fork_before(
    service: &AgentService,
    session: AgentSessionId,
    turn: AgentTurnId,
) -> AgentSessionId {
    let source = service.snapshot(session).unwrap();
    service
        .fork_session_before_turn(AgentSessionForkRequest {
            source_session_id: session,
            source_generation: source.generation,
            source_revision: source.revision,
            before_turn_id: turn,
        })
        .unwrap()
        .session
        .id
}

fn refs(state: &AgentCompressionState) -> Vec<&str> {
    state
        .message_refs
        .by_ref
        .keys()
        .map(String::as_str)
        .collect()
}

/// The blocks as the model knows them: id, tier, summary, what they cover.
fn blocks(state: &AgentCompressionState) -> Vec<(String, u8, String, Vec<String>, bool)> {
    state
        .blocks
        .iter()
        .map(|block| {
            (
                block.block_id.clone(),
                block.tier,
                block.summary.clone(),
                block.effective_message_ids.clone(),
                block.active,
            )
        })
        .collect()
}

/// A fork before a turn keeps the blocks whose messages it holds whole and
/// drops the others; the refs of what it holds stay where the model saw
/// them, and its own messages number above the parent's.
#[tokio::test]
async fn a_fork_before_a_turn_adopts_the_blocks_it_holds_whole() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let (service, model, session, turns) =
        folded_session(store, [response("forked", "From the fork.", 3_000)]).await;
    let parent = state(&service, session);
    let highest = parent.message_refs.by_ref.keys().max().unwrap().clone();

    // Before the second turn: the first answer is there, the second is not.
    let early = fork_before(&service, session, turns[1]);
    let adopted = state(&service, early);
    assert_eq!(
        blocks(&adopted),
        blocks(&parent)[..1].to_vec(),
        "b1 is adopted, b2 dropped"
    );
    assert_eq!(refs(&adopted), ["m00001", "m00002"]);
    assert_eq!(adopted.message_refs.by_raw[REF_FLOOR_ID], highest);

    service
        .start_turn(early, turn("second, again"))
        .await
        .unwrap();
    let request = model.requests().last().unwrap().clone();
    assert!(!text(&request).contains("a long first answer"));
    assert!(text(&request).contains("First exchange"));
    let fresh = format!("m{:05}", highest[1..].parse::<u32>().unwrap() + 1);
    assert!(
        text(&request).contains(&format!(">{fresh}</acp>\\nsecond, again")),
        "{}",
        text(&request)
    );

    // Before the fold's own turn both answers are there: both blocks stay,
    // shown without the `compress` calls that carried them.
    let late = fork_before(&service, session, turns[2]);
    assert_eq!(blocks(&state(&service, late)), blocks(&parent));
}

/// A fork of a session whose state was lost replays the `compress` calls
/// its history holds; one from before the folds has none to replay.
#[tokio::test]
async fn a_fork_rebuilds_a_lost_state_from_the_compress_calls() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let (service, _model, session, turns) = folded_session(store, []).await;
    let parent = state(&service, session);
    service.test_set_compression_state(session, None).unwrap();

    let rebuilt = state(&service, fork_before(&service, session, turns[3]));
    assert_eq!(blocks(&rebuilt), blocks(&parent));
    // The replay numbers the messages as they were numbered; the parent's
    // last message (its fourth prompt) is not in the fork.
    let mut numbered = parent.message_refs.by_ref.clone();
    numbered.pop_last();
    assert_eq!(rebuilt.message_refs.by_ref, numbered);

    let before_folds = fork_before(&service, session, turns[2]);
    assert_eq!(service.test_compression_state(before_folds).unwrap(), None);
}

/// A child at its parent's head projects the parent's blocks; an empty
/// child has none.
#[tokio::test]
async fn a_child_with_the_parents_history_keeps_its_state() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let (service, _model, session, _turns) = folded_session(store, []).await;
    let child = |base| {
        service
            .create_child_session(AgentChildSessionRequest {
                parent_session_id: session,
                origin: AgentSessionOrigin::Side,
                persistence: AgentSessionPersistence::Durable,
                base,
                context: Vec::new(),
                scope: Default::default(),
                role_instructions: None,
            })
            .unwrap()
            .id
    };
    let at_head = child(AgentChildSessionBase::ParentHead);
    assert_eq!(state(&service, at_head), state(&service, session));
    let empty = child(AgentChildSessionBase::Empty);
    assert_eq!(service.test_compression_state(empty).unwrap(), None);
}

/// A session whose state was lost (an unreadable one is dropped as it is
/// read) gets it back at the next start from its `compress` calls, and goes
/// on folded.
#[tokio::test]
async fn a_lost_state_is_rebuilt_at_restart() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let (service, _model, session, _turns) = folded_session(store.clone(), []).await;
    let before = state(&service, session);
    service.test_set_compression_state(session, None).unwrap();
    let snapshot = service.snapshot(session).unwrap();
    service
        .set_session_mode(
            session,
            snapshot.generation,
            snapshot.revision,
            hanafubuki_agent_kernel::AgentSessionMode::Plan,
        )
        .unwrap();

    let mut persisted = serde_json::to_value(store.load_runtime_state(session).unwrap()).unwrap();
    assert!(persisted.get("compression").is_none());
    persisted["compression"] = json!({ "blocks": "not a list" });
    let decoded: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
        serde_json::from_value(persisted).unwrap();
    assert_eq!(decoded.compression, None);

    let restarted_model = ScriptedModel::new([response("five", "Back.", 3_000)]);
    let restarted = AgentService::without_default_model_with_history_store(
        FakeTools::new([]).await.registries(),
        store,
    );
    restarted
        .recover_sessions_with_test_model(Arc::new(restarted_model.clone()))
        .unwrap();
    assert_eq!(blocks(&state(&restarted, session)), blocks(&before));
    restarted
        .start_turn(session, turn("Are you there?"))
        .await
        .unwrap();
    let request = &restarted_model.requests()[0];
    assert!(!text(request).contains("a long first answer"));
    assert!(!text(request).contains("a long second answer"));
    assert!(text(request).contains("Second exchange"));
}
