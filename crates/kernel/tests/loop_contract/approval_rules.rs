//! Approval rules registered by plugins only tighten the turn's permission
//! mode: a rule requiring a person routes the action to the user under
//! `AutoReview` and `FullAccess`, an earlier "allow for this session" does
//! not answer it, and a denial beats every other verdict.

use super::*;

/// A rule with a fixed verdict for every domain mutation whose summary
/// contains `matching`.
#[derive(Clone)]
struct FixedRule {
    id: &'static str,
    verdict: ApprovalRuleVerdict,
    matching: &'static str,
}

impl ApprovalRule for FixedRule {
    fn id(&self) -> &str {
        self.id
    }

    fn evaluate(&self, action: &ApprovalAction) -> Option<ApprovalRuleVerdict> {
        (action.kind == hanafubuki_agent_kernel::ApprovalActionKind::DomainMutation
            && action.summary.contains(self.matching))
        .then(|| self.verdict.clone())
    }
}

fn rule(id: &'static str, verdict: ApprovalRuleVerdict) -> Arc<dyn ApprovalRule> {
    Arc::new(FixedRule {
        id,
        verdict,
        matching: "",
    })
}

#[tokio::test]
async fn a_rule_requiring_a_person_overrides_auto_review_and_full_access() {
    for mode in [
        hanafubuki_agent_kernel::AgentPermissionMode::AutoReview,
        hanafubuki_agent_kernel::AgentPermissionMode::FullAccess,
    ] {
        let model = ScriptedEventModel::new([
            mutation_tool_call_events(),
            deterministic_assistant_events("started"),
        ]);
        let tools = FakeTools::new([FakeRegisteredExchange::success(
            "control_instance",
            json!({ "instanceId": "instance-a", "action": "start" }),
            json!({ "operationId": "launch-1", "instanceId": "instance-a" }),
        )])
        .await;
        tools
            .mount(&registering::<ApprovalRuleRegistry>(
                "person-rule",
                vec![rule("person", ApprovalRuleVerdict::RequireUser)],
            ))
            .await;
        let service = AgentService::new(Arc::new(model.clone()), tools.registries());
        let session = service.create_session(config()).unwrap();
        let session_id = session.id;
        let executing = service.clone();
        let turn_task = tokio::spawn(async move {
            executing
                .start_turn(session_id, permission_turn_input(mode))
                .await
        });

        let (turn, request) = loop {
            let snapshot = test_session_view(&service, session_id).unwrap();
            if let Some(turn) = snapshot.runtime.turn {
                if let Some(request) = service
                    .events_since(session_id, None)
                    .unwrap()
                    .events
                    .into_iter()
                    .find_map(|event| match event.kind {
                        AgentEventKind::ApprovalRequested { request } => Some(request),
                        _ => None,
                    })
                {
                    break (turn, request);
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        assert!(tools.calls().is_empty(), "{mode:?}");
        service
            .resolve_approval(
                session_id,
                turn.id,
                turn.generation,
                request.request_id,
                ApprovalUserDecision::AllowOnce,
            )
            .unwrap();
        assert_eq!(turn_task.await.unwrap().unwrap().final_answer, "started");
        assert_eq!(tools.approved_calls(), 1, "{mode:?}");
        // The reviewer model was never asked: only the two turn samples ran.
        assert_eq!(model.requests().len(), 2, "{mode:?}");
        let events = service.events_since(session_id, None).unwrap().events;
        assert!(!events.iter().any(|event| matches!(
            event.kind,
            AgentEventKind::PermissionReviewStarted { .. }
                | AgentEventKind::PermissionReviewCompleted { .. }
        )));
    }
}

#[tokio::test]
async fn a_denying_rule_wins_over_a_rule_requiring_a_person() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        deterministic_assistant_events("not started"),
    ]);
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<ApprovalRuleRegistry>(
            "person-rule",
            vec![rule("person", ApprovalRuleVerdict::RequireUser)],
        ))
        .await;
    tools
        .mount(&registering::<ApprovalRuleRegistry>(
            "deny-rule",
            vec![rule(
                "deny",
                ApprovalRuleVerdict::Deny {
                    reason: "instances are frozen".to_string(),
                },
            )],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::Manual),
        )
        .await
        .unwrap();

    assert_eq!(result.final_answer, "not started");
    assert!(tools.calls().is_empty());
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(!events
        .iter()
        .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. })));
    let second_input = serde_json::to_string(&model.requests()[1].input).unwrap();
    assert!(second_input.contains("instances are frozen"));
}

/// The tool is allowed for the session, but a rule requires a person for
/// the second action: it is asked, and only as "allow once".
#[tokio::test]
async fn a_rule_requiring_a_person_still_asks_after_allow_for_session() {
    let instance_b = json!({ "instanceId": "instance-b", "action": "start" });
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        calls_events(&[("call-b", "control_instance", instance_b.clone())]),
        deterministic_assistant_events("started both"),
    ]);
    let tools = FakeTools::new([
        FakeRegisteredExchange::success(
            "control_instance",
            json!({ "instanceId": "instance-a", "action": "start" }),
            json!({ "operationId": "launch-1" }),
        ),
        FakeRegisteredExchange::success(
            "control_instance",
            instance_b,
            json!({ "operationId": "launch-2" }),
        ),
    ])
    .await;
    tools
        .mount(&registering::<ApprovalRuleRegistry>(
            "person-rule",
            vec![Arc::new(FixedRule {
                id: "person",
                verdict: ApprovalRuleVerdict::RequireUser,
                matching: "instance-b",
            }) as Arc<dyn ApprovalRule>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(
                session_id,
                permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::Manual),
            )
            .await
    });

    let (turn, request) = approval_request(&service, session_id, 0).await;
    assert!(request.offers_allow_for_session);
    service
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowForSession,
        )
        .unwrap();
    let (turn, request) = approval_request(&service, session_id, 1).await;
    assert_eq!(request.action.summary, "Start instance instance-b");
    assert!(!request.offers_allow_for_session);
    assert_eq!(
        service.resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowForSession,
        ),
        Err(AgentError::Approval(
            "the approval request does not offer this decision".to_string()
        ))
    );
    service
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(
        turn_task.await.unwrap().unwrap().final_answer,
        "started both"
    );
    assert_eq!(tools.approved_calls(), 2);
}
