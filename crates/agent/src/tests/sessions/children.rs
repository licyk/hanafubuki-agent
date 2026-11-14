//! A session's children: its side conversation, the children the session
//! driver creates and runs, the workspace a session records, and archiving
//! or deleting a session with its children.

use super::*;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentSessionOrigin, AgentSessionParentFilter,
    AgentSessionPersistence,
};

struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}

pub(super) async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) fn turn_running(service: &AgentApplicationService, session_id: AgentSessionId) -> bool {
    service
        .snapshot(session_id)
        .unwrap()
        .turn
        .is_some_and(|turn| !turn.phase.is_terminal())
}

fn list(
    service: &AgentApplicationService,
    lifecycle: AgentSessionLifecycle,
    parent: AgentSessionParentFilter,
) -> Vec<AgentSessionId> {
    service
        .list_sessions_page(AgentSessionListRequest {
            lifecycle,
            parent,
            ..AgentSessionListRequest::default()
        })
        .unwrap()
        .sessions
        .into_iter()
        .map(|summary| summary.session_id)
        .collect()
}

fn child_of(
    parent: AgentSessionId,
    origin: AgentSessionOrigin,
    base: AgentChildSessionBase,
) -> AgentChildSessionRequest {
    AgentChildSessionRequest {
        parent_session_id: parent,
        origin,
        persistence: AgentSessionPersistence::Durable,
        base,
        context: Vec::new(),
        scope: Default::default(),
        role_instructions: None,
    }
}

pub(super) fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_side_conversation_inherits_its_parent_and_reads_the_side_policy_first() {
    // The parent's turn, the side's turn and the side's title request.
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 3]).await;
    let (service, paths) = configured_service("side-conversation", &server.base_url);
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(parent.id, "Explain the parent task.".to_string().into())
        .await
        .unwrap();

    let side = service.open_side_conversation(parent.id).unwrap();

    assert_eq!(side.parent_session_id, Some(parent.id));
    assert_eq!(side.origin, Some(AgentSessionOrigin::Side));
    assert!(side.ephemeral);
    assert_eq!(
        service.open_side_conversation(parent.id).unwrap().id,
        side.id,
        "a parent has one side conversation"
    );
    assert_eq!(
        list(
            &service,
            AgentSessionLifecycle::Available,
            AgentSessionParentFilter::TopLevel
        ),
        vec![parent.id]
    );
    assert!(list(
        &service,
        AgentSessionLifecycle::Available,
        AgentSessionParentFilter::ChildrenOf(parent.id)
    )
    .is_empty());
    // Its transcript starts at the side policy and the boundary.
    let opened = service.open_session(side.id, 50).unwrap().transcript.items;
    assert_eq!(opened.len(), 2);
    assert!(opened
        .iter()
        .all(|item| matches!(item.content, ConversationItem::ContinuationContext { .. })));

    service
        .submit_turn_with_sink(
            side.id,
            AgentTurnRequest {
                text: "What is the parent doing?".into(),
                images: Vec::new(),
                skills: Vec::new(),
                host: Default::default(),
            },
            Arc::new(Discard),
        )
        .await
        .unwrap();
    wait_until("the side turn ends", || {
        service
            .snapshot(side.id)
            .unwrap()
            .turn
            .is_some_and(|turn| turn.phase.is_terminal())
    })
    .await;

    let request = server
        .requests()
        .into_iter()
        .map(|request| String::from_utf8(request.body).unwrap())
        .find(|body| body.contains("Side conversation boundary."))
        .expect("the side turn's request");
    let position = |text: &str| request.find(text).unwrap();
    assert!(position("Explain the parent task.") < position("You are in a side conversation"));
    assert!(position("You are in a side conversation") < position("Side conversation boundary."));
    assert!(position("Side conversation boundary.") < position("What is the parent doing?"));
    let parent_transcript =
        serde_json::to_string(&service.open_session(parent.id, 50).unwrap().transcript).unwrap();
    assert!(!parent_transcript.contains("What is the parent doing?"));

    let current = service.snapshot(side.id).unwrap();
    assert!(matches!(
        service.archive_session(side.id, current.generation, current.revision),
        Err(AgentError::InvalidInput(_))
    ));
    assert!(matches!(
        service.close_side_conversation(parent.id).await,
        Err(AgentError::InvalidInput(_))
    ));
    service.close_side_conversation(side.id).await.unwrap();
    assert!(matches!(
        service.snapshot(side.id),
        Err(AgentError::SessionNotFound)
    ));
    assert_ne!(
        service.open_side_conversation(parent.id).unwrap().id,
        side.id
    );
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_side_conversation_opened_while_the_parent_works_stops_before_its_running_turn() {
    let server = FakeServer::start(vec![
        (assistant_events("First."), Duration::ZERO),
        (assistant_events("Second."), Duration::from_secs(2)),
        (assistant_events("Side."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("side-running-parent", &server.base_url);
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert!(
        matches!(
            service.open_side_conversation(parent.id),
            Err(AgentError::InvalidInput(_))
        ),
        "a parent without a turn has nothing to talk beside"
    );
    service
        .start_turn(parent.id, "First question.".to_string().into())
        .await
        .unwrap();
    let running = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .start_turn(parent.id, "Second question.".to_string().into())
                .await
        }
    });
    // Its request is sent: the side's is answered after it.
    wait_until("the second turn waits for its answer", || {
        turn_running(&service, parent.id) && server.requests().len() == 2
    })
    .await;

    let side = service.open_side_conversation(parent.id).unwrap();

    // The fake server answers in order.
    let answer = service
        .start_turn(side.id, "Side question.".to_string().into())
        .await
        .unwrap();
    assert_eq!(answer.final_answer, "Side.");
    let request = String::from_utf8(server.requests()[2].body.clone()).unwrap();
    assert!(request.contains("Side question."));
    assert!(request.contains("First question.") && request.contains("First."));
    assert!(!request.contains("Second question."));
    service.close_side_conversation(side.id).await.unwrap();
    assert_eq!(running.await.unwrap().unwrap().final_answer, "Second.");
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_driven_child_runs_a_turn_answers_and_is_interrupted() {
    let server = FakeServer::start(vec![
        (assistant_events("Parent answer."), Duration::ZERO),
        (assistant_events("Child answer."), Duration::ZERO),
        (assistant_events("Too late."), Duration::from_secs(30)),
    ])
    .await;
    let (service, paths) = configured_service("driven-child", &server.base_url);
    let driver = service.test_session_driver().clone();
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(parent.id, "Plan the work.".to_string().into())
        .await
        .unwrap();

    let child = driver
        .create_child(child_of(
            parent.id,
            AgentSessionOrigin::Subagent,
            AgentChildSessionBase::Empty,
        ))
        .unwrap();
    assert_eq!(child.parent_session_id, Some(parent.id));
    assert_eq!(child.forked_from_session_id, None);
    assert!(!child.ephemeral);
    assert_eq!(driver.last_answer(child.id).unwrap(), None);

    let outcome = driver
        .run_turn(
            child.id,
            "Do the subtask.".into(),
            Arc::new(crate::session_driver::Discard),
        )
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Child answer.");
    assert_eq!(
        driver.last_answer(child.id).unwrap().as_deref(),
        Some("Child answer.")
    );
    let child_request = String::from_utf8(server.requests()[1].body.clone()).unwrap();
    assert!(child_request.contains("Do the subtask."));
    assert!(
        !child_request.contains("Plan the work."),
        "an empty child inherits nothing"
    );

    let running = tokio::spawn({
        let driver = driver.clone();
        async move {
            driver
                .run_turn(
                    child.id,
                    "Take long.".into(),
                    Arc::new(crate::session_driver::Discard),
                )
                .await
        }
    });
    wait_until("the child's turn runs", || turn_running(&service, child.id)).await;
    driver.interrupt(child.id).await.unwrap();
    assert!(running.await.unwrap().is_err());
    assert_eq!(
        service.snapshot(child.id).unwrap().turn.unwrap().phase,
        AgentTurnPhase::Cancelled
    );

    driver.close(child.id).await.unwrap();
    assert_eq!(
        list(
            &service,
            AgentSessionLifecycle::Available,
            AgentSessionParentFilter::ChildrenOf(parent.id)
        ),
        vec![child.id],
        "a durable child stays in the history once closed"
    );
    assert_eq!(
        list(
            &service,
            AgentSessionLifecycle::Available,
            AgentSessionParentFilter::TopLevel
        ),
        vec![parent.id]
    );
    finish(service, paths);
}

#[tokio::test]
async fn a_session_records_its_workspace_and_its_children_take_it() {
    let (service, paths) = service_with("session-workspace", |paths, config| {
        configure_test_provider(config, "http://127.0.0.1:9/v1", None);
        let head = paths.managed_instances_dir.join(".git").join("HEAD");
        std::fs::create_dir_all(head.parent().unwrap()).unwrap();
        std::fs::write(head, "ref: refs/heads/main\n").unwrap();
    });
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let child = service
        .test_session_driver()
        .create_child(child_of(
            parent.id,
            AgentSessionOrigin::Review,
            AgentChildSessionBase::Empty,
        ))
        .unwrap();

    let workspace = ResolvedPath::resolve(&paths.managed_instances_dir).unwrap();
    for (session, parent_filter) in [
        (parent.id, AgentSessionParentFilter::TopLevel),
        (child.id, AgentSessionParentFilter::ChildrenOf(parent.id)),
    ] {
        let summary = service
            .list_sessions_page(AgentSessionListRequest {
                parent: parent_filter,
                ..AgentSessionListRequest::default()
            })
            .unwrap()
            .sessions
            .into_iter()
            .find(|summary| summary.session_id == session)
            .unwrap();
        assert_eq!(summary.cwd.as_deref(), Some(workspace.native_text()));
        assert_eq!(summary.branch.as_deref(), Some("main"));
    }
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_or_deleting_a_session_takes_its_children_with_it() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let (service, paths) = configured_service("children-cascade", &server.base_url);
    let driver = service.test_session_driver().clone();
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(parent.id, "Start.".to_string().into())
        .await
        .unwrap();
    // A fork of the parent's history: deleting the parent alone is refused.
    let review = driver
        .create_child(child_of(
            parent.id,
            AgentSessionOrigin::Review,
            AgentChildSessionBase::ParentHead,
        ))
        .unwrap();
    let nested = driver
        .create_child(child_of(
            review.id,
            AgentSessionOrigin::Subagent,
            AgentChildSessionBase::Empty,
        ))
        .unwrap();
    let side = service.open_side_conversation(parent.id).unwrap();

    let current = service.snapshot(parent.id).unwrap();
    let archived = service
        .archive_session(parent.id, current.generation, current.revision)
        .unwrap();

    assert_eq!(archived.lifecycle, AgentSessionLifecycle::Archived);
    for child in [review.id, nested.id] {
        assert_eq!(
            service.snapshot(child).unwrap().lifecycle,
            AgentSessionLifecycle::Archived
        );
    }
    assert!(matches!(
        service.snapshot(side.id),
        Err(AgentError::SessionNotFound)
    ));
    assert_eq!(
        list(
            &service,
            AgentSessionLifecycle::Archived,
            AgentSessionParentFilter::TopLevel
        ),
        vec![parent.id]
    );
    assert_eq!(
        list(
            &service,
            AgentSessionLifecycle::Archived,
            AgentSessionParentFilter::ChildrenOf(review.id)
        ),
        vec![nested.id]
    );

    let current = service.snapshot(parent.id).unwrap();
    service
        .delete_session_confirmed(parent.id, current.generation, current.revision)
        .unwrap();

    for session in [parent.id, review.id, nested.id] {
        assert!(matches!(
            service.snapshot(session),
            Err(AgentError::SessionNotFound)
        ));
    }
    for lifecycle in [
        AgentSessionLifecycle::Available,
        AgentSessionLifecycle::Archived,
    ] {
        assert!(list(&service, lifecycle, AgentSessionParentFilter::TopLevel).is_empty());
        assert!(list(
            &service,
            lifecycle,
            AgentSessionParentFilter::ChildrenOf(review.id)
        )
        .is_empty());
    }
    finish(service, paths);
}
