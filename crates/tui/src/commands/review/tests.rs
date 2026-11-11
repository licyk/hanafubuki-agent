//! `/review`'s dispatch, its picker over the targets, the running review
//! in the status line, and the review cell.

use std::path::PathBuf;

use hanafubuki_agent::{
    AgentReviewCodeLocation, AgentReviewCommit, AgentReviewFinding, AgentReviewLineRange,
    AgentReviewOutput, AgentReviewTargets, AgentSessionId,
};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::agent_bridge::TurnUpdate;
use crate::commands::tests::session::{chat, key, run, screen, sent, Sent};
use crate::history_cell::HistoryCell;

fn review(call: ReviewCall) -> Sent {
    Sent::Op(Op::Command(AgentCommand::Review(call)))
}

fn targets() -> AgentReviewTargets {
    AgentReviewTargets {
        branches: vec!["main".to_string(), "feature".to_string()],
        current_branch: Some("feature".to_string()),
        commits: vec![AgentReviewCommit {
            sha: "0123456789abcdef".to_string(),
            subject: "Fix the parser".to_string(),
        }],
    }
}

fn output(findings: Vec<AgentReviewFinding>) -> AgentReviewOutput {
    AgentReviewOutput {
        findings,
        overall_correctness: "patch is incorrect".to_string(),
        overall_explanation: "One panic on empty input; the rest reads well.".to_string(),
        overall_confidence_score: 0.7,
        child_session: AgentSessionId::new(2),
    }
}

fn finding(title: &str, priority: Option<i32>, located: bool) -> AgentReviewFinding {
    AgentReviewFinding {
        title: title.to_string(),
        body: "`items[0]` panics when the list is empty, which the caller allows.\nCheck the length first.".to_string(),
        confidence_score: 0.8,
        priority,
        code_location: located.then(|| AgentReviewCodeLocation {
            absolute_file_path: PathBuf::from("/work/src/lib.rs"),
            line_range: AgentReviewLineRange { start: 10, end: 12 },
        }),
    }
}

fn text(lines: &[ratatui::text::Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn type_text(chat: &mut crate::chatwidget::ChatWidget, text: &str) {
    for character in text.chars() {
        key(chat, KeyCode::Char(character));
    }
}

#[test]
fn review_opens_the_picker_takes_instructions_and_cancels() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "review"),
        [review(ReviewCall::Open)]
    );
    assert_eq!(
        run(&mut chat, &events, "review  look at the error handling "),
        [review(ReviewCall::Start(AgentReviewTarget::Custom {
            instructions: "look at the error handling".to_string()
        }))]
    );
    assert_eq!(
        run(&mut chat, &events, "review cancel"),
        [review(ReviewCall::Cancel)]
    );
}

#[test]
fn no_review_starts_during_a_turn_but_one_running_is_cancelled() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Started);
    assert_eq!(
        run(&mut chat, &events, "review"),
        [Sent::Cell(
            "■ '/review' is disabled while a task is in progress.".to_string()
        )]
    );
    assert_eq!(
        run(&mut chat, &events, "review cancel"),
        [review(ReviewCall::Cancel)]
    );
}

#[test]
fn the_picker_offers_the_presets_and_a_branch_starts_the_review() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Ready(Ok(targets()))));
    let presets = screen(&chat, 80);
    for preset in [
        "Select a review preset",
        "1. Review uncommitted changes",
        "2. Review against a base branch",
        "3. Review a commit",
        "4. Custom review instructions",
    ] {
        assert!(presets.contains(preset), "{presets}");
    }

    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    let branches = screen(&chat, 80);
    assert!(branches.contains("Current branch: feature"), "{branches}");
    assert!(branches.contains("feature (current)"), "{branches}");
    assert!(sent(&events).is_empty());

    // Esc goes back to the presets, which pick the commit page.
    key(&mut chat, KeyCode::Esc);
    assert!(screen(&chat, 80).contains("Select a review preset"));
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    let commits = screen(&chat, 80);
    assert!(commits.contains("Fix the parser"), "{commits}");
    assert!(commits.contains("0123456"), "{commits}");
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [review(ReviewCall::Start(AgentReviewTarget::Commit {
            sha: "0123456789abcdef".to_string(),
            title: Some("Fix the parser".to_string()),
        }))]
    );
    assert!(!screen(&chat, 80).contains("Select"));

    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Ready(Ok(targets()))));
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [review(ReviewCall::Start(AgentReviewTarget::BaseBranch {
            branch: "main".to_string()
        }))]
    );
}

#[test]
fn custom_instructions_are_typed_and_without_targets_only_they_and_the_changes_remain() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Ready(Err(
        "not a git repository".to_string(),
    ))));
    let presets = screen(&chat, 100);
    assert!(presets.contains("not a git repository"), "{presets}");

    // The base branch and the commit are skipped.
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert!(screen(&chat, 100).contains("Custom review instructions"));
    type_text(&mut chat, "Check the locking");
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [review(ReviewCall::Start(AgentReviewTarget::Custom {
            instructions: "Check the locking".to_string()
        }))]
    );

    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Ready(Err(
        "not a git repository".to_string(),
    ))));
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [review(ReviewCall::Start(
            AgentReviewTarget::UncommittedChanges
        ))]
    );
}

#[test]
fn a_running_review_shows_in_the_status_line_until_it_ends() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Started(
        "current changes".to_string(),
    )));
    assert!(chat.is_task_running());
    assert!(screen(&chat, 80).contains("Reviewing current changes"));
    assert_eq!(
        sent(&events),
        [Sent::Cell(
            ">> Code review started: current changes <<".to_string()
        )]
    );
    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Ended(ReviewEnd::Cancelled)));
    assert!(!chat.is_task_running());
    assert_eq!(
        sent(&events),
        [Sent::Cell("<< Code review cancelled >>".to_string())]
    );

    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Started(
        "current changes".to_string(),
    )));
    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Ended(ReviewEnd::Finished(
        Box::new(output(Vec::new())),
    ))));
    assert!(!chat.is_task_running());
    let cells = sent(&events);
    assert!(
        matches!(cells.last(), Some(Sent::Cell(cell)) if cell.contains("<< Code review finished >>")),
        "{cells:?}"
    );
}

#[test]
fn a_failed_call_leaves_a_running_turn_alone() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::Review(ReviewReply::Failed(
        "No review is running.".to_string(),
    )));
    assert!(chat.is_task_running());
    assert_eq!(
        sent(&events),
        [Sent::Cell("■ /review: No review is running.".to_string())]
    );
}

#[test]
fn the_review_cell_shows_the_verdict_and_each_finding() {
    let cell = ReviewCell::new(output(vec![
        finding("[P1] Check the length before indexing", Some(1), true),
        finding("Name the magic number", Some(3), false),
    ]));
    insta::assert_snapshot!(text(&cell.display_lines(60)));
    assert_eq!(
        text(&cell.raw_lines()),
        "<< Code review finished >>\nOne panic on empty input; the rest reads well.\n\nFull review comments:\n\n- [P1] Check the length before indexing — /work/src/lib.rs:10-12\n  `items[0]` panics when the list is empty, which the caller allows.\n  Check the length first.\n\n- Name the magic number\n  `items[0]` panics when the list is empty, which the caller allows.\n  Check the length first."
    );
}

#[test]
fn a_review_with_nothing_to_show_says_so() {
    let cell = ReviewCell::new(AgentReviewOutput {
        overall_explanation: String::new(),
        overall_correctness: String::new(),
        ..output(Vec::new())
    });
    assert_eq!(
        text(&cell.display_lines(80)),
        "<< Code review finished >>\n  Reviewer failed to output a response."
    );
}
