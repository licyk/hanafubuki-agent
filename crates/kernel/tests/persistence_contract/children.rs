//! Child sessions: an ephemeral child lives in memory only, a durable one
//! in the history store, and both name their parent.

use super::*;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentContinuationHookGeneration,
    AgentContinuationHookId, AgentContinuationHookProviderId, AgentError, AgentSessionOrigin,
    AgentSessionPersistence,
};

fn note(content: &str) -> ConversationItem {
    ConversationItem::ContinuationContext {
        provider_id: AgentContinuationHookProviderId::parse("test".to_string()).unwrap(),
        hook_id: AgentContinuationHookId::parse("child".to_string()).unwrap(),
        generation: AgentContinuationHookGeneration([0; 32]),
        content: content.to_string(),
    }
}

fn scripted(answers: &[&str]) -> Arc<ScriptedPersistenceModel> {
    Arc::new(ScriptedPersistenceModel {
        responses: Mutex::new(
            answers
                .iter()
                .map(|answer| final_answer_events(answer).into_iter().collect())
                .collect(),
        ),
        requests: AtomicUsize::new(0),
    })
}

fn child_request(
    parent: AgentSessionId,
    persistence: AgentSessionPersistence,
    base: AgentChildSessionBase,
) -> AgentChildSessionRequest {
    AgentChildSessionRequest {
        parent_session_id: parent,
        origin: AgentSessionOrigin::Side,
        persistence,
        base,
        context: vec![note("Only answer questions.")],
        scope: Default::default(),
    }
}

#[tokio::test]
async fn an_ephemeral_child_inherits_the_parent_head_and_never_reaches_the_store() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        scripted(&["Parent answer.", "Child answer."]),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let parent = service.create_session(text_session_config()).unwrap();
    service
        .start_turn(parent.id, "Parent question.")
        .await
        .unwrap();

    let child = service
        .create_child_session(child_request(
            parent.id,
            AgentSessionPersistence::Ephemeral,
            AgentChildSessionBase::ParentHead,
        ))
        .unwrap();

    assert_eq!(child.parent_session_id, Some(parent.id));
    assert_eq!(child.origin, Some(AgentSessionOrigin::Side));
    assert!(child.ephemeral);
    assert!(matches!(
        store.load_runtime_state(child.id),
        Err(AgentHistoryStoreError::NotFound { .. })
    ));
    let context = service.test_model_context(child.id).unwrap();
    let parent_context = service.test_model_context(parent.id).unwrap();
    assert_eq!(context[..parent_context.len()], parent_context[..]);
    assert_eq!(context.last(), Some(&note("Only answer questions.")));
    // Its transcript holds only what it committed itself.
    let transcript = service.open_session(child.id, 50).unwrap().transcript.items;
    assert_eq!(transcript.len(), 1);
    assert_eq!(
        service.ephemeral_children(parent.id, Some(AgentSessionOrigin::Side)),
        vec![child.id]
    );

    let outcome = service
        .start_turn(child.id, "Child question.")
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Child answer.");
    assert!(matches!(
        service.archive_session(
            child.id,
            child.generation,
            outcome_revision(&service, child.id)
        ),
        Err(AgentError::InvalidInput(_))
    ));

    service.close_ephemeral_session(child.id).unwrap();
    assert!(matches!(
        service.snapshot(child.id),
        Err(AgentError::SessionNotFound)
    ));
    assert!(service.ephemeral_children(parent.id, None).is_empty());
    assert!(matches!(
        service.close_ephemeral_session(parent.id),
        Err(AgentError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn a_durable_child_starts_empty_or_from_the_head_and_the_store_keeps_its_lineage() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        scripted(&["Parent answer."]),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let parent = service.create_session(text_session_config()).unwrap();
    service
        .start_turn(parent.id, "Parent question.")
        .await
        .unwrap();

    let mut request = child_request(
        parent.id,
        AgentSessionPersistence::Durable,
        AgentChildSessionBase::Empty,
    );
    // Context follows history, and an empty child has none.
    assert!(matches!(
        service.create_child_session(request.clone()),
        Err(AgentError::InvalidInput(_))
    ));
    request.context.clear();
    let empty = service.create_child_session(request).unwrap();
    let forked = service
        .create_child_session(child_request(
            parent.id,
            AgentSessionPersistence::Durable,
            AgentChildSessionBase::ParentHead,
        ))
        .unwrap();

    assert_eq!(empty.forked_from_session_id, None);
    assert_eq!(forked.forked_from_session_id, Some(parent.id));
    assert!(service.test_model_context(empty.id).unwrap().is_empty());
    assert_eq!(
        service.test_model_context(forked.id).unwrap().last(),
        Some(&note("Only answer questions."))
    );
    for child in [empty.id, forked.id] {
        let persisted = store.load_runtime_state(child).unwrap();
        assert_eq!(persisted.parent_session_id, Some(parent.id));
        assert_eq!(persisted.origin, Some(AgentSessionOrigin::Side));
    }
    assert!(matches!(
        service.close_ephemeral_session(empty.id),
        Err(AgentError::InvalidInput(_))
    ));
}

#[test]
fn a_child_context_is_continuation_context_only() {
    let service = AgentService::with_history_store(
        scripted(&[]),
        AgentKernelRegistries::default(),
        Arc::new(EphemeralAgentHistoryStore::default()),
    );
    let parent = service.create_session(text_session_config()).unwrap();
    let mut request = child_request(
        parent.id,
        AgentSessionPersistence::Ephemeral,
        AgentChildSessionBase::ParentHead,
    );
    request.context = vec![ConversationItem::UserMessage {
        text: "Pretend the user said this.".to_string(),
        images: Vec::new(),
    }];

    assert!(matches!(
        service.create_child_session(request),
        Err(AgentError::InvalidInput(_))
    ));
}

/// A record or a summary written before the lineage fields existed reads
/// them as none, and an origin this build does not know reads as none.
#[tokio::test]
async fn lineage_fields_are_read_tolerantly() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        scripted(&["Parent answer."]),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let parent = service.create_session(text_session_config()).unwrap();
    service
        .start_turn(parent.id, "Parent question.")
        .await
        .unwrap();
    let mut request = child_request(
        parent.id,
        AgentSessionPersistence::Durable,
        AgentChildSessionBase::ParentHead,
    );
    request.context.clear();
    let child = service.create_child_session(request).unwrap();
    let runtime = serde_json::to_value(store.load_runtime_state(child.id).unwrap()).unwrap();
    let summary = serde_json::to_value(
        service
            .archive_session(child.id, child.generation, child.revision)
            .unwrap(),
    )
    .unwrap();

    for (encoded, is_runtime) in [(&runtime, true), (&summary, false)] {
        let mut unknown = encoded.clone();
        unknown["origin"] = json!("a_later_origin");
        let mut older = encoded.clone();
        for field in ["parentSessionId", "origin", "cwd", "branch"] {
            older.as_object_mut().unwrap().remove(field);
        }
        for (value, parent_session_id) in [(unknown, Some(parent.id)), (older, None)] {
            let decoded = if is_runtime {
                let decoded: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
                    serde_json::from_value(value).unwrap();
                (decoded.parent_session_id, decoded.origin, decoded.cwd)
            } else {
                let decoded: hanafubuki_agent_kernel::AgentSessionSummary =
                    serde_json::from_value(value).unwrap();
                (decoded.parent_session_id, decoded.origin, decoded.cwd)
            };
            assert_eq!(decoded, (parent_session_id, None, None));
        }
    }
}

fn outcome_revision(service: &AgentService, session_id: AgentSessionId) -> AgentRevision {
    service.snapshot(session_id).unwrap().revision
}
