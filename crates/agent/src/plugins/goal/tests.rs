use hanafubuki_agent_kernel::{
    AgentError, AgentGoal, AgentGoalOrigin, AgentGoalStatus, AgentSessionMode,
};

use super::context::goal_section;
use super::runtime::{accrual, validated_objective, AccountingPoint};
use super::templates;
use super::tools::goal_response;
use crate::goals::AgentGoalSession;

fn goal() -> AgentGoal {
    AgentGoal {
        objective: "Port <the> parser & tests".to_string(),
        status: AgentGoalStatus::Active,
        origin: AgentGoalOrigin::User,
        token_budget: Some(1_000),
        time_budget_seconds: None,
        tokens_used: 250,
        time_used_seconds: 42,
        created_at: 1_700_000_000_123,
        updated_at: 1_700_000_100_999,
    }
}

fn session(tokens: u64, turn_started_at: Option<u128>) -> AgentGoalSession {
    AgentGoalSession {
        mode: AgentSessionMode::Default,
        ephemeral: false,
        tokens,
        turn_started_at,
    }
}

#[test]
fn accrual_counts_tokens_and_the_turns_time_since_the_last_point() {
    let point = AccountingPoint {
        tokens: 100,
        at: 10_000,
    };
    // Within the turn: 2.5 s, of which the half second carries over.
    let (tokens, seconds, next) = accrual(&session(160, Some(9_000)), Some(point), 12_500);
    assert_eq!((tokens, seconds), (60, 2));
    assert_eq!(
        next,
        AccountingPoint {
            tokens: 160,
            at: 12_000
        }
    );
    // A turn that started after the point counts from its start.
    let (_, seconds, _) = accrual(&session(160, Some(11_000)), Some(point), 14_000);
    assert_eq!(seconds, 3);
    // Between turns no time passes for the goal.
    let (tokens, seconds, _) = accrual(&session(170, None), Some(point), 99_000);
    assert_eq!((tokens, seconds), (70, 0));
    // A session's count that started again (another usage source) is
    // never negative.
    assert_eq!(accrual(&session(5, None), Some(point), 0).0, 0);
}

#[test]
fn without_a_point_accrual_only_starts_counting() {
    let (tokens, seconds, next) = accrual(&session(900, Some(0)), None, 5_000);
    assert_eq!((tokens, seconds), (0, 0));
    assert_eq!(
        next,
        AccountingPoint {
            tokens: 900,
            at: 5_000
        }
    );
}

#[test]
fn the_templates_quote_the_objective_and_the_budget() {
    let mut goal = goal();
    let continuation = templates::continuation(&goal);
    assert!(continuation.starts_with("Continue working toward the active session goal."));
    assert!(continuation.contains("<objective>\nPort &lt;the&gt; parser &amp; tests\n</objective>"));
    assert!(continuation.contains("- Tokens used: 250\n- Token budget: 1000\n- Tokens remaining: 750\n- Time used: 42 seconds\n- Time budget: none\n"));
    assert!(!continuation.contains("{{"));

    goal.token_budget = None;
    goal.time_budget_seconds = Some(60);
    let updated = templates::objective_updated(&goal);
    assert!(updated.contains("- Token budget: none\n- Tokens remaining: unbounded\n"));
    let limited = templates::budget_limit(&goal);
    assert!(limited.starts_with("The active session goal has reached its budget."));
    assert!(limited.contains("- Time budget: 60 seconds\n"));
    for text in [updated, limited] {
        assert!(!text.contains("{{"), "{text}");
    }
}

#[test]
fn the_context_quotes_a_short_objective_and_omits_a_long_one_whole() {
    let mut goal = goal();
    goal.objective = "Say \"done\"".to_string();
    assert_eq!(
        goal_section(&goal),
        "User set the goal: \"Say \\\"done\\\"\"\nGoal status: active.\nToken budget: 1000.\n"
    );
    goal.objective = "x".repeat(800);
    goal.origin = AgentGoalOrigin::Model;
    goal.status = AgentGoalStatus::BudgetLimited;
    let section = goal_section(&goal);
    assert!(section.starts_with(
        "You, at the user's request, set the goal: [objective omitted; exceeds the evidence limit"
    ));
    assert!(!section.contains("xxx"));
    assert!(section.contains("Goal status: budget_limited."));
}

#[test]
fn an_objective_is_trimmed_and_bounded() {
    assert_eq!(validated_objective("  Ship it \n").unwrap(), "Ship it");
    assert!(matches!(
        validated_objective(" \t"),
        Err(AgentError::InvalidInput(_))
    ));
    let longest = "é".repeat(super::MAXIMUM_GOAL_OBJECTIVE_CHARS);
    assert!(validated_objective(&longest).is_ok());
    assert!(matches!(
        validated_objective(&format!("{longest}é")),
        Err(AgentError::InvalidInput(_))
    ));
}

#[test]
fn a_completed_budgeted_goal_tells_the_model_to_report_its_usage() {
    let mut goal = goal();
    let active = goal_response(Some(&goal), false);
    assert_eq!(active["remainingTokens"], 750);
    assert_eq!(active["goal"]["createdAt"], 1_700_000_000);
    assert_eq!(active["goal"]["status"], "active");
    assert!(active["completionBudgetReport"].is_null());
    assert!(active["goal"].get("timeBudgetSeconds").is_none());

    goal.status = AgentGoalStatus::Complete;
    goal.time_budget_seconds = Some(600);
    let completed = goal_response(Some(&goal), true);
    assert!(completed["completionBudgetReport"]
        .as_str()
        .is_some_and(|report| report.starts_with("Goal achieved.")));
    assert_eq!(completed["goal"]["timeBudgetSeconds"], 600);

    let none = goal_response(None, false);
    assert!(none["goal"].is_null() && none["remainingTokens"].is_null());
}
