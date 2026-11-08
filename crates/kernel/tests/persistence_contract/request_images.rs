//! A model request's images are read with no session lock held: the history
//! store loads them on its own worker while the request's builder awaits,
//! and the builder needs the session again only to re-check and assemble.

use hanafubuki_agent_kernel::{AgentImageDetail, PreparedAgentImage};
use hanafubuki_agent_model_api::{ModelImageDetail, ModelInputItem, ModelUserContent};

use super::*;

/// Records each request and answers it with a final answer.
#[derive(Default)]
struct RecordingAnswerModel {
    requests: Mutex<Vec<ModelRequest>>,
}

impl ModelPort for RecordingAnswerModel {
    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().unwrap().push(request);
        let events = final_answer_events("Seen.");
        Box::pin(async move { Ok(Box::new(ScriptedPersistenceStream { events }) as ModelStream) })
    }
}

struct NoopSink;

impl AgentEventSink for NoopSink {
    fn emit(&self, _event: AgentEvent) {}
}

/// While the first request's image is loading, another writer takes the
/// session's lock and stages a commit (steering, whose append then parks).
/// The load completes meanwhile; the request is assembled only once that
/// commit settles, from the loaded bytes, though the session's revision
/// moved.
#[test]
fn a_requests_images_load_while_another_writer_holds_the_session() {
    let store = Arc::new(RecordingHistoryStore::default());
    let model = Arc::new(RecordingAnswerModel::default());
    let service = AgentService::with_history_store(
        model.clone(),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let mut config = text_session_config();
    config.input_capabilities.image_input = hanafubuki_agent_model_api::ModelImageInputPolicy::Auto;
    let session_id = service.create_session(config).unwrap().id;
    let image = service
        .stage_image_attachment(
            session_id,
            PreparedAgentImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "pixel.png".to_string(),
                detail: AgentImageDetail::High,
            },
        )
        .unwrap();
    let load_gate = Arc::new(AppendGate::new());
    *store.attachment_gate.lock().unwrap() = Some(load_gate.clone());

    let (outcome, assembled_while_held, first_request) = on_a_current_thread_runtime({
        let (service, store, model, image) =
            (service.clone(), store.clone(), model.clone(), image.clone());
        move || async move {
            let turn = tokio::spawn({
                let service = service.clone();
                let mut input = AgentTurnInput::from("Describe it");
                input.images = vec![image];
                async move { service.start_turn(session_id, input).await }
            });
            meet(|gate| &gate.started, &load_gate).await;
            // The session is free while its images load.
            let running = service.snapshot(session_id).unwrap().turn.unwrap();
            let append_gate = Arc::new(AppendGate::new());
            *store.append_gate.lock().unwrap() = Some(append_gate.clone());
            let steering = tokio::task::spawn_blocking({
                let service = service.clone();
                move || {
                    service.steer_turn_with_sink(
                        session_id,
                        running.id,
                        running.generation,
                        "Also this",
                        &NoopSink,
                    )
                }
            });
            meet(|gate| &gate.started, &append_gate).await;
            // The steering commit is staged and holds the session; the load
            // completes regardless.
            meet(|gate| &gate.release, &load_gate).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let assembled_while_held = !model.requests.lock().unwrap().is_empty();
            meet(|gate| &gate.release, &append_gate).await;
            steering.await.unwrap().unwrap();
            let outcome = turn.await.unwrap();
            let first_request = model.requests.lock().unwrap().first().cloned();
            (outcome, assembled_while_held, first_request)
        }
    });

    assert!(
        !assembled_while_held,
        "the request was assembled while another writer held the session"
    );
    assert_eq!(outcome.unwrap().final_answer, "Seen.");
    let first_request = first_request.unwrap();
    let user_message = first_request
        .input
        .iter()
        .find_map(|item| match item {
            ModelInputItem::UserMessage { content } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        user_message,
        vec![
            ModelUserContent::InputText {
                text: "Describe it".to_string()
            },
            ModelUserContent::InputImage {
                mime_type: "image/png".to_string(),
                data: vec![1, 2, 3],
                detail: ModelImageDetail::High,
            },
        ]
    );
    assert_eq!(store.attachment_loads.lock().unwrap()[0], vec![image]);
}
