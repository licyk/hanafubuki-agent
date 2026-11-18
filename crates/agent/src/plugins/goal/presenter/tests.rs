use super::*;
use hanafubuki_agent_contracts::ToolResult;
use hanafubuki_agent_kernel::{AgentGoal, AgentGoalStatus};
use serde_json::json;

#[test]
fn goal_receipts_show_objective_status_and_optional_budgets() {
    for status in [
        AgentGoalStatus::Active,
        AgentGoalStatus::Paused,
        AgentGoalStatus::Blocked,
        AgentGoalStatus::BudgetLimited,
        AgentGoalStatus::Complete,
    ] {
        let goal: AgentGoal = serde_json::from_value(json!({
            "objective": "修复渲染", "status": status, "tokensUsed": 250,
            "tokenBudget": 1000, "timeUsedSeconds": 42, "timeBudgetSeconds": 60
        }))
        .unwrap();
        let output =
            ToolResult::success(super::super::tools::goal_response(Some(&goal), true)).output;
        let status = serde_json::to_value(status).unwrap();
        for tool in GoalPresenter.tools() {
            let Some(AgentToolDisplay::Text { text }) =
                GoalPresenter.display(tool, &json!({}), &output)
            else {
                panic!("goal display")
            };
            assert!(text.contains(&format!(
                "Status: {}",
                status.as_str().unwrap().replace('_', " ")
            )));
            assert!(text.contains("Goal: 修复渲染"));
            assert!(text.contains("Tokens: 250 / 1000 · 750 remaining"));
            assert!(text.contains("Time: 42s / 60s · 18s remaining"));
            assert!(!text.contains("Report final usage"));
        }
        let unlimited = AgentGoal {
            token_budget: None,
            time_budget_seconds: None,
            ..goal
        };
        let output =
            ToolResult::success(super::super::tools::goal_response(Some(&unlimited), false)).output;
        let Some(AgentToolDisplay::Text { text }) =
            GoalPresenter.display("get_goal", &json!({}), &output)
        else {
            panic!("goal display")
        };
        assert!(text.contains("Tokens: 250 used\nTime: 42s used"));
        assert!(!text.contains("remaining"));
    }
}

#[test]
fn absent_goals_are_distinct_from_failed_or_unreadable_receipts() {
    let output = ToolResult::success(super::super::tools::goal_response(None, false)).output;
    assert_eq!(
        GoalPresenter.display("get_goal", &json!({}), &output),
        Some(AgentToolDisplay::Text {
            text: "No goal set.".into()
        })
    );
    let mut failed = output;
    failed.is_error = true;
    assert_eq!(GoalPresenter.display("get_goal", &json!({}), &failed), None);
    let malformed = ToolResult::success(json!({"unexpected": true})).output;
    assert_eq!(
        GoalPresenter.display("get_goal", &json!({}), &malformed),
        None
    );
}
