use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_approval::test_support::ManualApprovalClock;
use hanafubuki_agent_approval::{
    AgentPermissionMode, ApprovalAction, ApprovalActionKind, ApprovalContext, ApprovalContextScope,
    ApprovalDecision, ApprovalDecisionSource, ApprovalError, ApprovalOrigin, ApprovalOutcome,
    ApprovalRiskClass, ApprovalService, ApprovalUserDecision, PermissionActionDetails,
    PermissionRequirement,
};

fn action(payload: &[u8]) -> ApprovalAction {
    ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::DomainMutation,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        "Start instance one".to_string(),
        payload,
    )
    .unwrap()
}

fn approval_context(payload: &[u8], generation: u64) -> ApprovalContext {
    ApprovalContext::from_canonical_bytes("call-1".to_string(), payload)
        .unwrap()
        .with_scope(ApprovalContextScope {
            authority: "agent".to_string(),
            session_id: "session-1".to_string(),
            operation_group_id: "turn-1".to_string(),
            generation,
        })
        .unwrap()
}

#[test]
fn permission_mode_and_requirement_have_only_the_new_contract() {
    assert_eq!(AgentPermissionMode::default(), AgentPermissionMode::Manual);
    let requirement = PermissionRequirement::ReviewRequired(action(b"action"));
    assert!(matches!(
        requirement,
        PermissionRequirement::ReviewRequired(_)
    ));
    let forbidden: PermissionRequirement<ApprovalAction> = PermissionRequirement::Forbidden {
        reason: "outside the workspace".to_string(),
    };
    assert!(matches!(forbidden, PermissionRequirement::Forbidden { .. }));
}

#[test]
fn multi_file_patch_path_details_are_bounded_to_128_targets() {
    let paths = (0..128)
        .map(|index| format!("/workspace/file-{index}"))
        .collect::<Vec<_>>();
    assert!(action(b"multi-file")
        .with_details(PermissionActionDetails::Paths {
            canonical_paths: paths.clone(),
        })
        .is_ok());

    let mut too_many = paths;
    too_many.push("/workspace/overflow".to_string());
    assert!(matches!(
        action(b"too-many").with_details(PermissionActionDetails::Paths {
            canonical_paths: too_many,
        }),
        Err(ApprovalError::InvalidAction(_))
    ));
}

#[tokio::test]
async fn manual_allow_issues_one_exact_user_authorization() {
    let service = ApprovalService::new(Arc::new(ManualApprovalClock::new(100)));
    let action = action(b"action-one");
    let context = approval_context(b"turn-one", 1);
    let admission = service
        .request_manual(
            action.clone(),
            context.clone(),
            Duration::from_secs(30),
            false,
        )
        .unwrap();
    service
        .resolve_user(
            admission.request.request_id,
            &context,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    let ApprovalOutcome::Granted { grant, resolution } = admission.waiter.await else {
        panic!("allow must issue a grant")
    };
    assert_eq!(resolution.source, ApprovalDecisionSource::User);
    let authorized = service.consume(grant, &action, &context).unwrap();
    assert!(authorized.matches(&action, &context));
    assert_eq!(authorized.source(), ApprovalDecisionSource::User);
}

#[tokio::test]
async fn allow_for_session_grants_only_where_the_request_offers_it() {
    let service = ApprovalService::new(Arc::new(ManualApprovalClock::new(100)));
    let action = action(b"session-action");
    let context = approval_context(b"session-turn", 1);
    let refusing = service
        .request_manual(
            action.clone(),
            context.clone(),
            Duration::from_secs(30),
            false,
        )
        .unwrap();
    assert!(!refusing.request.offers_allow_for_session);
    assert_eq!(
        service.resolve_user(
            refusing.request.request_id,
            &context,
            ApprovalUserDecision::AllowForSession,
        ),
        Err(ApprovalError::DecisionNotOffered)
    );
    // The refused answer leaves the request waiting for a valid one.
    service
        .resolve_user(
            refusing.request.request_id,
            &context,
            ApprovalUserDecision::Deny,
        )
        .unwrap();
    assert!(matches!(refusing.waiter.await, ApprovalOutcome::Denied(_)));

    let offering = service
        .request_manual(
            action.clone(),
            context.clone(),
            Duration::from_secs(30),
            true,
        )
        .unwrap();
    assert!(offering.request.offers_allow_for_session);
    service
        .resolve_user(
            offering.request.request_id,
            &context,
            ApprovalUserDecision::AllowForSession,
        )
        .unwrap();
    let ApprovalOutcome::Granted { grant, resolution } = offering.waiter.await else {
        panic!("allow for session must issue a grant")
    };
    assert_eq!(resolution.decision, ApprovalDecision::AllowForSession);
    assert_eq!(resolution.source, ApprovalDecisionSource::User);
    assert!(service
        .consume(grant, &action, &context)
        .unwrap()
        .matches(&action, &context));
}

#[tokio::test]
async fn deny_abort_and_timeout_never_issue_a_grant() {
    for expected in ["deny", "abort", "timeout"] {
        let service = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)));
        let action = action(expected.as_bytes());
        let context = approval_context(expected.as_bytes(), 1);
        let admission = service
            .request_manual(action, context.clone(), Duration::from_secs(1), false)
            .unwrap();
        match expected {
            "deny" => service
                .resolve_user(
                    admission.request.request_id,
                    &context,
                    ApprovalUserDecision::Deny,
                )
                .unwrap(),
            "abort" => service
                .abort(admission.request.request_id, &context)
                .unwrap(),
            "timeout" => service
                .timeout(admission.request.request_id, &context)
                .unwrap(),
            _ => unreachable!(),
        }
        assert!(matches!(
            (expected, admission.waiter.await),
            ("deny", ApprovalOutcome::Denied(_))
                | ("abort", ApprovalOutcome::Aborted(_))
                | ("timeout", ApprovalOutcome::TimedOut(_))
        ));
    }
}

#[tokio::test]
async fn stale_context_generation_and_expired_grant_are_rejected() {
    let clock = Arc::new(ManualApprovalClock::new(10));
    let service = ApprovalService::new(clock.clone());
    let action = action(b"exact-action");
    let context = approval_context(b"exact-turn", 1);
    let stale_generation = approval_context(b"exact-turn", 2);
    let admission = service
        .request_manual(
            action.clone(),
            context.clone(),
            Duration::from_millis(5),
            false,
        )
        .unwrap();
    assert_eq!(
        service.resolve_user(
            admission.request.request_id,
            &stale_generation,
            ApprovalUserDecision::AllowOnce,
        ),
        Err(ApprovalError::StaleContext)
    );
    service
        .resolve_user(
            admission.request.request_id,
            &context,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(
        service.resolve_user(
            admission.request.request_id,
            &context,
            ApprovalUserDecision::AllowOnce,
        ),
        Err(ApprovalError::RequestNotFound)
    );
    let ApprovalOutcome::Granted { grant, .. } = admission.waiter.await else {
        panic!("allow must issue a grant")
    };
    clock.advance(6);
    assert!(matches!(
        service.consume(grant, &action, &context),
        Err(ApprovalError::GrantExpired)
    ));
}

#[tokio::test]
async fn exact_action_metadata_and_digest_are_checked_before_consumption() {
    let service = ApprovalService::new(Arc::new(ManualApprovalClock::new(10)));
    let action = action(b"same-resolved-plan");
    let context = approval_context(b"exact-turn", 1);
    let admission = service
        .request_manual(
            action.clone(),
            context.clone(),
            Duration::from_secs(1),
            false,
        )
        .unwrap();
    service
        .resolve_user(
            admission.request.request_id,
            &context,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    let ApprovalOutcome::Granted { grant, .. } = admission.waiter.await else {
        panic!("allow must issue a grant")
    };
    let mut forged = action.clone();
    forged.risk = ApprovalRiskClass::Low;
    forged.origin = ApprovalOrigin::User;
    assert!(matches!(
        service.consume(grant, &forged, &context),
        Err(ApprovalError::GrantMismatch)
    ));
}

#[test]
fn auto_reviewer_and_full_access_preserve_their_source_and_exact_binding() {
    for source in [
        ApprovalDecisionSource::AutoReviewer,
        ApprovalDecisionSource::FullAccess,
        ApprovalDecisionSource::SessionAllowance,
        ApprovalDecisionSource::UserCommand,
    ] {
        let service = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)));
        let action = action(format!("{source:?}").as_bytes());
        let context = approval_context(format!("{source:?}").as_bytes(), 1);
        let authorized = service
            .authorize(action.clone(), &context, Duration::from_secs(1), source)
            .unwrap();
        assert_eq!(authorized.source(), source);
        assert!(authorized.matches(&action, &context));
        assert!(!authorized.matches(&action, &approval_context(b"different", 1)));
    }
}

#[test]
fn user_and_system_sources_cannot_bypass_manual_review() {
    for source in [ApprovalDecisionSource::User, ApprovalDecisionSource::System] {
        let service = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)));
        assert!(matches!(
            service.authorize(
                action(b"action"),
                &approval_context(b"context", 1),
                Duration::from_secs(1),
                source,
            ),
            Err(ApprovalError::InvalidDecisionSource)
        ));
    }
}
