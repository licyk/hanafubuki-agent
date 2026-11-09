//! What a host sees of the hierarchical compression: `ContextCompressed`
//! for each fold, in the commit of the `compress` call's output, and the
//! status `/compression` reads, which changes nothing.

use hanafubuki_agent_kernel::{AgentCompressionSummary, AgentEvent};

use super::*;

fn events(service: &AgentService, session: AgentSessionId) -> Vec<AgentEvent> {
    service.events_since(session, None).unwrap().events
}

/// A fold is told once, right after the commit of its output and before
/// the call completes; a `compress` that folds nothing is not.
#[tokio::test]
async fn a_fold_is_told_in_the_commit_of_its_output() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        compress("again", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold the first exchange."))
        .await
        .unwrap();

    let events = events(&service, session.id);
    let compressed: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event.kind, AgentEventKind::ContextCompressed { .. }))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(compressed.len(), 1, "{events:#?}");
    let at = compressed[0];
    let AgentEventKind::ContextCompressed {
        call_id,
        blocks_created,
        tokens_compressed,
        tier,
    } = &events[at].kind
    else {
        unreachable!()
    };
    assert_eq!(call_id, "fold-1");
    assert_eq!(*blocks_created, 1);
    assert!(*tokens_compressed > 0);
    assert_eq!(*tier, 1);
    assert!(matches!(
        events[at - 1].kind,
        AgentEventKind::HistoryCommitted { item_count: 1, .. }
    ));
    assert!(matches!(
        &events[at + 1].kind,
        AgentEventKind::ToolCompleted { call_id, is_error: false } if call_id == "fold-1"
    ));
    assert!(tool_output(&service, session.id, "again").contains("[Compression no-op"));
    assert_eq!(
        service.snapshot(session.id).unwrap().compression,
        Some(AgentCompressionSummary {
            active_blocks: 1,
            tokens_compressed: *tokens_compressed,
        })
    );
}

/// The status shows the block and the counters, and reading it changes
/// neither the state nor the revision. A session that never ran
/// `hierarchical` has none, and its snapshot carries no summary.
#[tokio::test]
async fn the_status_reads_the_blocks_without_changing_the_session() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold the first exchange."))
        .await
        .unwrap();
    let before = state(&service, session.id);
    let revision = service.snapshot(session.id).unwrap().revision;

    let status = service
        .compression_status(session.id, &tuning())
        .unwrap()
        .expect("a hierarchical session has a status");
    assert!(
        status.panel.contains("ACP Context Analysis"),
        "{}",
        status.panel
    );
    assert!(
        status.panel.contains("Blocks: 1 active / 1 total"),
        "{}",
        status.panel
    );
    assert!(status.panel.contains("First exchange"), "{}", status.panel);
    assert!(!status.cache_report.is_empty());
    assert_eq!(status.stats.compression_count, 1);
    assert_eq!(status.content_store, None);
    assert_eq!(state(&service, session.id), before);
    assert_eq!(service.snapshot(session.id).unwrap().revision, revision);

    let standard = service.create_session(config()).unwrap();
    assert_eq!(
        service.compression_status(standard.id, &tuning()).unwrap(),
        None
    );
    assert_eq!(service.snapshot(standard.id).unwrap().compression, None);
}
