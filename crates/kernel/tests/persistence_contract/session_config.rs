//! A session's compaction settings: the strategy, and the remote
//! compaction its transport carries, persisted between operations.

use super::*;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentCompactionStrategy, AgentError,
    AgentSessionOrigin, AgentSessionPersistence, ModelRetryPolicy,
};
use std::path::Path;

/// The corpora shared with Hanafubuki were recorded before sessions had a
/// compaction strategy: their configs still decode, as `standard`.
#[test]
fn history_written_before_the_compaction_strategy_decodes_as_standard() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("../agent/src/tests/golden");
    let mut configs = 0;
    for corpus in [
        "history",
        "history-reasoning",
        "history-goal",
        "history-lineage",
    ] {
        for rollout in rollouts(&golden.join(corpus)) {
            for line in std::fs::read_to_string(&rollout).unwrap().lines() {
                let record: serde_json::Value = serde_json::from_str(line).unwrap();
                let Some(config) = record["payload"]["runtimeCheckpoint"].get("config") else {
                    continue;
                };
                assert!(config.get("compactionStrategy").is_none());
                let config: AgentSessionConfig = serde_json::from_value(config.clone())
                    .unwrap_or_else(|error| panic!("{}: {error}", rollout.display()));
                assert_eq!(
                    config.compaction_strategy,
                    AgentCompactionStrategy::Standard
                );
                configs += 1;
            }
        }
    }
    assert!(configs > 0, "the corpora hold runtime checkpoints");

    // One a later build wrote reads as standard too.
    let mut config = serde_json::to_value(text_session_config()).unwrap();
    config["compactionStrategy"] = json!("eventual");
    let config: AgentSessionConfig = serde_json::from_value(config).unwrap();
    assert_eq!(
        config.compaction_strategy,
        AgentCompactionStrategy::Standard
    );
}

fn rollouts(directory: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(rollouts(&path));
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            found.push(path);
        }
    }
    found
}

#[test]
fn the_strategy_is_persisted_between_operations_without_a_revision() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service.create_session(text_session_config()).unwrap();
    assert!(service
        .set_session_compaction_strategy(session.id, AgentCompactionStrategy::Hierarchical)
        .unwrap());
    assert!(!service
        .set_session_compaction_strategy(session.id, AgentCompactionStrategy::Hierarchical)
        .unwrap());
    // A caller that read the session before still holds its revision.
    assert_eq!(
        service.snapshot(session.id).unwrap().revision,
        session.revision
    );
    let persisted = store.load_runtime_state(session.id).unwrap();
    assert_eq!(
        persisted.config.compaction_strategy,
        AgentCompactionStrategy::Hierarchical
    );

    // A child takes its parent's strategy, an ephemeral one standard only.
    let child = |persistence| {
        service
            .create_child_session(AgentChildSessionRequest {
                parent_session_id: session.id,
                origin: AgentSessionOrigin::Subagent,
                persistence,
                base: AgentChildSessionBase::Empty,
                context: Vec::new(),
                scope: Default::default(),
                role_instructions: None,
            })
            .unwrap()
            .id
    };
    let durable = child(AgentSessionPersistence::Durable);
    assert_eq!(
        service.session_compaction_strategy(durable).unwrap(),
        AgentCompactionStrategy::Hierarchical
    );
    let ephemeral = child(AgentSessionPersistence::Ephemeral);
    assert_eq!(
        service.session_compaction_strategy(ephemeral).unwrap(),
        AgentCompactionStrategy::Standard
    );
    assert!(matches!(
        service.set_session_compaction_strategy(ephemeral, AgentCompactionStrategy::Background),
        Err(AgentError::InvalidInput(_))
    ));
    service.close_ephemeral_session(ephemeral).unwrap();
}

/// The configured remote compaction reaches a session with the transport
/// that carries it: compaction checks one against the other.
#[test]
fn activation_takes_a_changed_remote_compaction_into_the_session() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service.create_session(text_session_config()).unwrap();
    service
        .activate_model_session(
            hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            "https://api.example.test/v1",
            None,
            Vec::new(),
            ModelRetryPolicy::default(),
            ModelCompactionCapability::ResponsesCompactV2,
            AgentConfiguredModelFacts::default(),
            session.id,
        )
        .unwrap();
    assert_eq!(
        service.snapshot(session.id).unwrap().revision,
        session.revision
    );
    let persisted = store.load_runtime_state(session.id).unwrap();
    assert_eq!(
        persisted.config.compaction_capability,
        ModelCompactionCapability::ResponsesCompactV2
    );
}
