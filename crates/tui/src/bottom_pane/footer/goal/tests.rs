// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;
use std::time::Duration;

fn goal(status: AgentGoalStatus) -> AgentGoal {
    serde_json::from_value(serde_json::json!({
        "objective": "Run Python in a subagent", "status": status,
        "tokensUsed": 12400, "timeUsedSeconds": 30
    }))
    .unwrap()
}

#[test]
fn elapsed_time_advances_only_during_active_work_and_survives_repeated_snapshots() {
    let now = Instant::now();
    let mut state = GoalStatus::default();
    let active = goal(AgentGoalStatus::Active);
    state.update(Some(&active), now);
    state.set_running(true, now + Duration::from_secs(60));
    state.update(Some(&active), now + Duration::from_secs(65));
    assert_eq!(
        state
            .line(now + Duration::from_secs(70))
            .unwrap()
            .to_string(),
        "Pursuing goal (40s)"
    );
    state.set_running(false, now + Duration::from_secs(70));
    assert_eq!(
        state
            .line(now + Duration::from_secs(90))
            .unwrap()
            .to_string(),
        "Pursuing goal (40s)"
    );
    state.update(None, now);
    assert!(state.line(now).is_none());
}

#[test]
fn snapshots_reset_the_clock_and_stopped_states_do_not_accrue_time() {
    let now = Instant::now();
    let mut state = GoalStatus::default();
    state.set_running(true, now);
    let mut active = goal(AgentGoalStatus::Active);
    state.update(Some(&active), now);
    active.time_used_seconds = 45;
    state.update(Some(&active), now + Duration::from_secs(15));
    assert_eq!(
        state
            .line(now + Duration::from_secs(20))
            .unwrap()
            .to_string(),
        "Pursuing goal (50s)"
    );
    for (status, text) in [
        (AgentGoalStatus::Paused, "Goal paused (/goal resume)"),
        (AgentGoalStatus::Blocked, "Goal stalled (/goal resume)"),
        (AgentGoalStatus::BudgetLimited, "Goal unmet (30s)"),
        (AgentGoalStatus::Complete, "Goal achieved (30s)"),
    ] {
        state.update(Some(&goal(status)), now);
        assert_eq!(
            state
                .line(now + Duration::from_secs(90))
                .unwrap()
                .to_string(),
            text
        );
    }
}

#[test]
fn explicit_budgets_show_usage_instead_of_only_elapsed_time() {
    let now = Instant::now();
    let mut state = GoalStatus::default();
    let mut active = goal(AgentGoalStatus::Active);
    active.token_budget = Some(50_000);
    state.update(Some(&active), now);
    assert_eq!(
        state.line(now).unwrap().to_string(),
        "Pursuing goal (12.4K / 50K tokens)"
    );
    active.token_budget = None;
    active.time_budget_seconds = Some(60);
    state.update(Some(&active), now);
    assert_eq!(
        state.line(now).unwrap().to_string(),
        "Pursuing goal (30s / 1m 00s)"
    );
}
