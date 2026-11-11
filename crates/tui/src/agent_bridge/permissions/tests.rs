// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

fn review(id: usize, status: PermissionReviewStatus) -> AgentEventKind {
    AgentEventKind::PermissionReviewCompleted {
        review_id: format!("review-{id}"),
        call_id: format!("call-{id}"),
        action_summary: format!("rm -rf /tmp/test-{id}"),
        status,
        assessment: None,
        started_at: 0,
        completed_at: 1,
    }
}

fn ids(denials: &RecentDenials, session_id: AgentSessionId) -> Vec<String> {
    denials
        .of(session_id)
        .into_iter()
        .map(|denial| denial.review_id)
        .collect()
}

#[test]
fn only_denials_are_kept_the_latest_ten_newest_first() {
    let session = AgentSessionId::new(1);
    let mut denials = RecentDenials::default();
    for status in [
        PermissionReviewStatus::Approved,
        PermissionReviewStatus::TimedOut,
        PermissionReviewStatus::Failed,
        PermissionReviewStatus::Aborted,
        PermissionReviewStatus::Reviewing,
    ] {
        denials.observe(session, &review(99, status));
    }
    denials.observe(session, &AgentEventKind::TurnStarted { role: None });
    assert_eq!(ids(&denials, session), Vec::<String>::new());

    for id in 0..12 {
        denials.observe(session, &review(id, PermissionReviewStatus::Denied));
    }
    let expected: Vec<String> = (2..12).rev().map(|id| format!("review-{id}")).collect();
    assert_eq!(ids(&denials, session), expected);
    let latest = &denials.of(session)[0];
    assert_eq!(latest.summary, "rm -rf /tmp/test-11");
    assert_eq!(latest.rationale, None);

    // The same review again moves to the front, once.
    denials.observe(session, &review(5, PermissionReviewStatus::Denied));
    assert_eq!(ids(&denials, session)[..2], ["review-5", "review-11"]);
    assert_eq!(ids(&denials, session).len(), 10);
}

#[test]
fn each_session_keeps_its_own() {
    let (first, second) = (AgentSessionId::new(1), AgentSessionId::new(2));
    let mut denials = RecentDenials::default();
    denials.observe(first, &review(1, PermissionReviewStatus::Denied));
    denials.observe(second, &review(2, PermissionReviewStatus::Denied));
    assert_eq!(ids(&denials, first), ["review-1"]);
    assert_eq!(ids(&denials, second), ["review-2"]);
    assert!(denials.of(AgentSessionId::new(3)).is_empty());

    denials.remove(first, "review-1");
    assert!(denials.of(first).is_empty());
    assert_eq!(ids(&denials, second), ["review-2"]);
}
