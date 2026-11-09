use serde::Deserialize;

use super::*;

#[derive(Deserialize)]
struct Holder {
    #[serde(default, deserialize_with = "tolerant_goal")]
    goal: Option<AgentGoal>,
}

fn read(json: &str) -> Option<AgentGoal> {
    serde_json::from_str::<Holder>(json).unwrap().goal
}

#[test]
fn a_goal_round_trips_and_leaves_out_what_it_lacks() {
    let goal = AgentGoal {
        objective: "Ship".to_string(),
        status: AgentGoalStatus::BudgetLimited,
        origin: AgentGoalOrigin::Model,
        token_budget: Some(10),
        time_budget_seconds: None,
        tokens_used: 12,
        time_used_seconds: 3,
        created_at: 1,
        updated_at: 2,
    };
    let encoded = serde_json::to_string(&goal).unwrap();
    assert_eq!(
        encoded,
        r#"{"objective":"Ship","status":"budget_limited","origin":"model","tokenBudget":10,"tokensUsed":12,"timeUsedSeconds":3,"createdAt":1,"updatedAt":2}"#
    );
    assert_eq!(read(&format!(r#"{{"goal":{encoded}}}"#)), Some(goal));
}

#[test]
fn a_goal_is_read_tolerantly() {
    assert_eq!(read("{}"), None);
    assert_eq!(read(r#"{"goal":null}"#), None);
    // Unknown fields are ignored; missing usage, origin and times default.
    let goal = read(r#"{"goal":{"objective":"Ship","status":"active","later":true}}"#).unwrap();
    assert_eq!(
        (goal.origin, goal.tokens_used, goal.created_at),
        (AgentGoalOrigin::User, 0, 0)
    );
    // A goal this build cannot read is none, not a failed record.
    assert_eq!(
        read(r#"{"goal":{"objective":"Ship","status":"usage_limited"}}"#),
        None
    );
    assert_eq!(read(r#"{"goal":{"status":"active"}}"#), None);
}
