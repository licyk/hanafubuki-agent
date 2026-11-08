use super::*;

#[test]
fn sampling_request_result_matches_codex_outer_loop_contract() {
    let result = SamplingRequestResult {
        needs_follow_up: true,
        last_agent_message_id: Some(AgentMessageId::new(1)),
    };
    assert!(result.needs_follow_up);
    assert_eq!(result.last_agent_message_id, Some(AgentMessageId::new(1)));
}

#[test]
fn progress_digest_distinguishes_missing_commentary_and_final_phase() {
    let reasons = BTreeSet::new();
    let missing = sampling_progress_digest(&[(None, "text".into())], &[], &reasons);
    let commentary = sampling_progress_digest(
        &[(Some(AgentMessagePhase::Commentary), "text".into())],
        &[],
        &reasons,
    );
    let final_answer = sampling_progress_digest(
        &[(Some(AgentMessagePhase::FinalAnswer), "text".into())],
        &[],
        &reasons,
    );
    assert_ne!(missing, commentary);
    assert_ne!(commentary, final_answer);
}

#[test]
fn repeated_follow_up_fails_at_stall_budget_but_changed_progress_resets_it() {
    let first = AgentProgressDigest([1; 32]);
    let changed = AgentProgressDigest([2; 32]);
    let mut state = AgentNoProgressState::default();
    state.observe_follow_up(first, 2).unwrap();
    state.observe_follow_up(first, 2).unwrap();
    assert_eq!(
        state.observe_follow_up(first, 2),
        Err(AgentNoProgressError::Exhausted {
            consecutive_steps: 2,
            limit: 2
        })
    );
    state.observe_follow_up(changed, 2).unwrap();
    assert_eq!(state.consecutive_stalled_steps, 0);
}
