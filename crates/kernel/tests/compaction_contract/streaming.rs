use super::*;
use hanafubuki_agent_kernel::{AgentEvent, AgentEventSink};

#[derive(Default)]
struct CompactionEvents(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for CompactionEvents {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn streamed_summary(cancelled: bool) -> ScriptedResponse {
    let ScriptedResponse::Events(mut events) = response("compact", "corrected summary", 10) else {
        unreachable!()
    };
    let deltas = ["partial", " summary"].map(|delta| {
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "item-compact".into(),
            delta: delta.into(),
        })
    });
    events.splice(2..2, deltas);
    if cancelled {
        events.truncate(4);
        events.push(Err(ModelError::Cancelled));
    }
    ScriptedResponse::Events(events)
}

#[tokio::test]
async fn local_summary_streams_real_cumulative_text_then_installs_completed_text() {
    let service = AgentService::new(
        Arc::new(ScriptedModel::new([
            response("one", "answer", 10),
            streamed_summary(false),
        ])),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "question").await.unwrap();
    let before = service.snapshot(session.id).unwrap();
    let sink = Arc::new(CompactionEvents::default());
    let result = service
        .compact_session_with_sink(session.id, before.generation, before.revision, sink.clone())
        .await
        .unwrap();
    let snapshots = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            AgentEventKind::CompactionUpdated { compaction } => Some(compaction.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(snapshots[0].status, AgentCompactionStatus::Running);
    assert_eq!(snapshots[0].summary, None);
    assert_eq!(snapshots[1].summary.as_deref(), Some("partial"));
    assert_eq!(snapshots[2].summary.as_deref(), Some("partial summary"));
    assert_eq!(snapshots[3].summary.as_deref(), Some("corrected summary"));
    assert_eq!(snapshots[4].status, AgentCompactionStatus::ReadyToInstall);
    assert_eq!(snapshots[5].status, AgentCompactionStatus::Completed);
    assert_eq!(snapshots[4].summary, result.compaction.summary);
    assert!(result
        .compaction
        .summary
        .unwrap()
        .ends_with("corrected summary"));
    assert!(snapshots
        .windows(2)
        .all(|pair| pair[0].updated_at <= pair[1].updated_at));
}

#[tokio::test]
async fn cancelled_compaction_keeps_partial_snapshot_for_reconnection_without_installing_it() {
    let service = AgentService::new(
        Arc::new(ScriptedModel::new([
            response("one", "answer", 10),
            streamed_summary(true),
        ])),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "question").await.unwrap();
    let before = test_session_view(&service, session.id).unwrap();
    let sink = Arc::new(CompactionEvents::default());
    assert_eq!(
        service
            .compact_session_with_sink(session.id, before.generation, before.revision, sink.clone())
            .await,
        Err(AgentError::Cancelled)
    );
    let after = test_session_view(&service, session.id).unwrap();
    assert_eq!(before.model_context, after.model_context);
    let snapshot = after.compaction.as_ref().unwrap();
    assert_eq!(snapshot.status, AgentCompactionStatus::Cancelled);
    assert_eq!(snapshot.summary.as_deref(), Some("partial summary"));
    assert!(matches!(&sink.0.lock().unwrap().last().unwrap().kind,
        AgentEventKind::CompactionUpdated { compaction } if compaction == snapshot));
}

#[tokio::test]
async fn opaque_compaction_emits_status_without_inventing_readable_text() {
    let service = AgentService::new(
        Arc::new(ScriptedModel::remote(
            ModelCompactionCapability::ResponsesCompactV2,
            [
                response("one", "answer", 10),
                v2_compaction_response(&["opaque"], None),
            ],
            [],
        )),
        FakeTools::new([]).await.registries(),
    );
    let mut settings = config();
    settings.compaction_capability = ModelCompactionCapability::ResponsesCompactV2;
    let session = service.create_session(settings).unwrap();
    service.start_turn(session.id, "question").await.unwrap();
    let before = service.snapshot(session.id).unwrap();
    let sink = Arc::new(CompactionEvents::default());
    service
        .compact_session_with_sink(session.id, before.generation, before.revision, sink.clone())
        .await
        .unwrap();
    let statuses = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            AgentEventKind::CompactionUpdated { compaction } => {
                assert_eq!(compaction.summary, None);
                Some(compaction.status)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        [
            AgentCompactionStatus::Running,
            AgentCompactionStatus::ReadyToInstall,
            AgentCompactionStatus::Completed
        ]
    );
}
