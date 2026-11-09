//! Editing an earlier message: the session is forked before the message's
//! turn, the original archived, and the message handed back to edit.

use super::children::{finish, turn_running, wait_until};
use super::*;

/// The turns of `session_id`, oldest first.
fn turn_ids(service: &AgentApplicationService, session_id: AgentSessionId) -> Vec<AgentTurnId> {
    let mut turns = service
        .open_session(session_id, 50)
        .unwrap()
        .transcript
        .items
        .into_iter()
        .filter_map(|item| match item.content {
            ConversationItem::TurnStart { turn_id } => Some(turn_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    turns.reverse();
    turns
}

fn transcript_text(service: &AgentApplicationService, session_id: AgentSessionId) -> String {
    serde_json::to_string(&service.open_session(session_id, 50).unwrap().transcript).unwrap()
}

// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn editing_a_message_forks_before_its_turn_and_archives_the_original() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 4]).await;
    let (service, paths) = configured_service("edit-from-turn", &server.base_url);
    let source = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    for question in ["First question.", "Second question."] {
        service
            .start_turn(source.id, question.to_string().into())
            .await
            .unwrap();
    }
    let turns = turn_ids(&service, source.id);
    assert_eq!(turns.len(), 2);

    let edited = service.edit_from_turn(source.id, turns[1]).unwrap();

    assert_eq!(edited.restored_prompt, "Second question.");
    assert!(edited.restored_images.is_empty());
    assert_eq!(edited.session.forked_from_session_id, Some(source.id));
    let fork = transcript_text(&service, edited.session.id);
    assert!(fork.contains("First question."), "{fork}");
    assert!(!fork.contains("Second question."), "{fork}");
    assert_eq!(
        service.snapshot(source.id).unwrap().lifecycle,
        AgentSessionLifecycle::Archived
    );
    let listed = |lifecycle| {
        service
            .list_sessions_page(AgentSessionListRequest {
                lifecycle,
                ..AgentSessionListRequest::default()
            })
            .unwrap()
            .sessions
            .into_iter()
            .map(|summary| summary.session_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        listed(AgentSessionLifecycle::Available),
        vec![edited.session.id]
    );
    assert_eq!(listed(AgentSessionLifecycle::Archived), vec![source.id]);
    // The archived original is not edited again.
    let error = service.edit_from_turn(source.id, turns[0]).unwrap_err();
    assert!(error.contains("archived"), "{error}");
    assert_eq!(
        listed(AgentSessionLifecycle::Available),
        vec![edited.session.id]
    );

    // The fork goes on from there.
    service
        .start_turn(
            edited.session.id,
            "Second question, edited.".to_string().into(),
        )
        .await
        .unwrap();
    let fork = transcript_text(&service, edited.session.id);
    assert!(fork.contains("Second question, edited."), "{fork}");
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_is_refused_while_a_turn_runs_and_in_a_side_conversation() {
    let server = FakeServer::start(vec![
        (assistant_events("First."), Duration::ZERO),
        (assistant_events("Second."), Duration::from_secs(2)),
    ])
    .await;
    let (service, paths) = configured_service("edit-refused", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(session.id, "First question.".to_string().into())
        .await
        .unwrap();
    let first = turn_ids(&service, session.id)[0];
    let side = service.open_side_conversation(session.id).unwrap();
    let error = service.edit_from_turn(side.id, first).unwrap_err();
    assert!(error.contains("side conversation"), "{error}");
    service.close_side_conversation(side.id).await.unwrap();

    let running = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .start_turn(session.id, "Second question.".to_string().into())
                .await
        }
    });
    wait_until("the second turn runs", || {
        turn_running(&service, session.id)
    })
    .await;
    let error = service.edit_from_turn(session.id, first).unwrap_err();
    assert!(error.contains("a turn is running"), "{error}");
    running.await.unwrap().unwrap();

    assert_eq!(
        service.snapshot(session.id).unwrap().lifecycle,
        AgentSessionLifecycle::Available
    );
    finish(service, paths);
}
