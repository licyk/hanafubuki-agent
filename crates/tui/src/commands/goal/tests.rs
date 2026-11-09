//! `/goal`'s dispatch and arguments, its menu, the replace confirmation,
//! the edit and budget prompts, and the goal the chat keeps.

use hanafubuki_agent::AgentGoalOrigin;
use ratatui::crossterm::event::KeyCode;

use super::menu::{elapsed, summary_lines};
use super::*;
use crate::agent_bridge::TurnUpdate;
use crate::chatwidget::ChatWidget;
use crate::commands::tests::session::{chat, key, run, screen, sent, Sent};

fn goal_call(call: GoalCall) -> Sent {
    Sent::Op(Op::Command(AgentCommand::Goal(call)))
}

fn goal(status: AgentGoalStatus) -> AgentGoal {
    AgentGoal {
        objective: "Ship the release".to_string(),
        status,
        origin: AgentGoalOrigin::User,
        token_budget: Some(200_000),
        time_budget_seconds: None,
        tokens_used: 12_400,
        time_used_seconds: 5_400,
        created_at: 0,
        updated_at: 0,
    }
}

fn set(objective: &str, token_budget: Option<u64>, confirmed: bool) -> Sent {
    goal_call(GoalCall::Set {
        objective: objective.to_string(),
        budgets: AgentGoalBudgets {
            token_budget,
            time_budget_seconds: None,
        },
        confirmed,
    })
}

fn reply(chat: &mut ChatWidget, reply: GoalReply) {
    chat.handle_turn_update(TurnUpdate::Goal(reply));
}

fn type_text(chat: &mut ChatWidget, text: &str) {
    for character in text.chars() {
        key(chat, KeyCode::Char(character));
    }
}

#[test]
fn goal_reads_its_subcommands_and_an_objective_with_a_budget() {
    let (mut chat, events) = chat(false);
    for (line, call) in [
        ("goal", GoalCall::Menu),
        ("goal clear", GoalCall::Clear),
        ("goal pause", GoalCall::Pause),
        ("goal resume", GoalCall::Resume),
        ("goal edit", GoalCall::Edit),
    ] {
        assert_eq!(
            run(&mut chat, &events, line),
            [Sent::Op(Op::EnsureSession), goal_call(call)],
            "{line}"
        );
    }
    assert_eq!(
        run(&mut chat, &events, "goal  Ship the release "),
        [
            Sent::Op(Op::EnsureSession),
            set("Ship the release", None, false)
        ]
    );
    assert_eq!(
        run(&mut chat, &events, "goal --tokens 50k Ship it"),
        [
            Sent::Op(Op::EnsureSession),
            set("Ship it", Some(50_000), false)
        ]
    );
    for wrong in ["goal --tokens many Ship it", "goal --tokens 5000"] {
        assert_eq!(
            run(&mut chat, &events, wrong),
            [
                Sent::Op(Op::EnsureSession),
                Sent::Cell(format!("■ {USAGE}"))
            ],
            "{wrong}"
        );
    }
}

#[test]
fn token_counts_take_thousands_and_millions() {
    assert_eq!(token_count("200000"), Some(200_000));
    assert_eq!(token_count("200k"), Some(200_000));
    assert_eq!(token_count("1M"), Some(1_000_000));
    assert_eq!(token_count("0"), None);
    assert_eq!(token_count("1.5m"), None);
    assert_eq!(elapsed(59), "59s");
    assert_eq!(elapsed(5_400), "1h 30m");
    assert_eq!(elapsed(2 * 86_400 + 3_600), "2d 1h 0m");
}

#[test]
fn the_menu_shows_the_goal_and_its_actions() {
    let (mut chat, events) = chat(false);
    reply(
        &mut chat,
        GoalReply::Menu(Some(goal(AgentGoalStatus::Active))),
    );
    insta::assert_snapshot!(screen(&chat, 72));
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent(&events), [goal_call(GoalCall::Pause)]);

    reply(
        &mut chat,
        GoalReply::Menu(Some(goal(AgentGoalStatus::Paused))),
    );
    let menu = screen(&chat, 72);
    assert!(menu.contains("Resume goal"), "{menu}");
    assert!(!menu.contains("Pause goal"), "{menu}");

    let lines = summary_lines(&goal(AgentGoalStatus::Complete));
    let status = lines
        .iter()
        .map(|line| line.to_string())
        .find(|line| line.starts_with("Status"));
    assert_eq!(status.as_deref(), Some("Status: complete"));

    reply(&mut chat, GoalReply::Menu(None));
    key(&mut chat, KeyCode::Esc);
    let cells = sent(&events);
    assert!(
        cells.contains(&Sent::Cell(format!("• No goal is set. {USAGE}"))),
        "{cells:?}"
    );
}

#[test]
fn replacing_an_unfinished_goal_asks_first_and_keeping_it_is_first() {
    let (mut chat, events) = chat(false);
    reply(
        &mut chat,
        GoalReply::ConfirmReplace {
            current: goal(AgentGoalStatus::Active),
            objective: "Fix the build".to_string(),
            budgets: AgentGoalBudgets::default(),
        },
    );
    let question = screen(&chat, 80);
    assert!(question.contains("Replace goal?"), "{question}");
    assert!(
        question.contains("Current goal (active): Ship the release"),
        "{question}"
    );
    key(&mut chat, KeyCode::Enter);
    assert!(sent(&events).is_empty());

    reply(
        &mut chat,
        GoalReply::ConfirmReplace {
            current: goal(AgentGoalStatus::Active),
            objective: "Fix the build".to_string(),
            budgets: AgentGoalBudgets::default(),
        },
    );
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent(&events), [set("Fix the build", None, true)]);
}

#[test]
fn edit_and_budget_prompts_set_the_goal_again() {
    let (mut chat, events) = chat(false);
    reply(&mut chat, GoalReply::Edit(goal(AgentGoalStatus::Active)));
    let prompt = screen(&chat, 80);
    assert!(prompt.contains("Edit goal"), "{prompt}");
    assert!(prompt.contains("Ship the release"), "{prompt}");
    assert!(prompt.contains("its usage starts again"), "{prompt}");
    type_text(&mut chat, " today");
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [set("Ship the release today", Some(200_000), true)]
    );

    reply(&mut chat, GoalReply::Budget(goal(AgentGoalStatus::Active)));
    assert!(screen(&chat, 80).contains("200000"));
    for _ in 0.."200000".len() {
        key(&mut chat, KeyCode::Backspace);
    }
    type_text(&mut chat, "50k");
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent(&events), [set("Ship the release", Some(50_000), true)]);
}

#[test]
fn each_reply_says_what_became_of_the_goal() {
    let (mut chat, events) = chat(false);
    reply(&mut chat, GoalReply::Paused(goal(AgentGoalStatus::Paused)));
    reply(
        &mut chat,
        GoalReply::Paused(goal(AgentGoalStatus::Complete)),
    );
    reply(&mut chat, GoalReply::Resumed(goal(AgentGoalStatus::Active)));
    reply(
        &mut chat,
        GoalReply::Resumed(goal(AgentGoalStatus::BudgetLimited)),
    );
    reply(
        &mut chat,
        GoalReply::Failed("goals are unavailable: off".to_string()),
    );
    reply(&mut chat, GoalReply::Cleared(true));
    reply(&mut chat, GoalReply::Cleared(false));
    assert_eq!(
        sent(&events),
        [
            Sent::Cell("• Goal paused /goal resume".to_string()),
            Sent::Cell("• The goal is complete; only an active goal pauses.".to_string()),
            Sent::Cell("• Goal resumed It continues from your next message.".to_string()),
            Sent::Cell(
                "• The goal's budget is spent, so it stays limited by budget. /goal edit gives it a new start"
                    .to_string()
            ),
            Sent::Cell("■ /goal: goals are unavailable: off".to_string()),
            Sent::Cell("• Goal cleared".to_string()),
            Sent::Cell("• No goal is set.".to_string()),
        ]
    );
}
