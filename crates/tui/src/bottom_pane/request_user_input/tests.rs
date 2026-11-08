use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::bottom_pane::tests::fixtures::user_input_request;
use insta::assert_snapshot;

fn two_questions(id: u64) -> AgentUserInputRequest {
    user_input_request(
        id,
        serde_json::json!([
            {
                "id": "scope",
                "header": "Scope",
                "question": "Which crates should the change touch?",
                "options": [
                    {"id": "tui", "label": "The TUI only", "description": "Leave the CLI alone"},
                    {"id": "both", "label": "The TUI and the CLI", "description": ""}
                ]
            },
            {
                "id": "tests",
                "header": "Tests",
                "question": "Add snapshot tests?",
                "options": [
                    {"id": "yes", "label": "Yes", "description": "Regenerate them"},
                    {"id": "no", "label": "No", "description": ""}
                ]
            }
        ]),
    )
}

fn overlay(request: AgentUserInputRequest) -> (RequestUserInputOverlay, Receiver<AppEvent>) {
    let (tx, rx) = channel::<AppEvent>();
    (
        RequestUserInputOverlay::new(request, AppEventSender::new(tx)),
        rx,
    )
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn answers(rx: &Receiver<AppEvent>) -> Vec<Vec<(String, String)>> {
    rx.try_iter()
        .filter_map(|event| match event {
            AppEvent::Op(Op::UserInputAnswer { answers, .. }) => Some(
                answers
                    .into_iter()
                    .map(|answer| (answer.question_id, answer.option_id))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn render(view: &RequestUserInputOverlay, width: u16) -> String {
    let height = view.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    (0..area.height)
        .map(|row| {
            (0..area.width)
                .map(|col| buf[(col, row)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn enter_answers_each_question_then_submits_them_all() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Down));
    view.handle_key_event(key(KeyCode::Enter));
    assert!(answers(&rx).is_empty(), "the first Enter only moves on");
    view.handle_key_event(key(KeyCode::Enter));
    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "both".to_string()),
            ("tests".to_string(), "yes".to_string()),
        ]]
    );
    assert!(view.is_complete());
}

#[test]
fn a_digit_picks_that_option_and_moves_on() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Char('2')));
    view.handle_key_event(key(KeyCode::Char('2')));
    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "both".to_string()),
            ("tests".to_string(), "no".to_string()),
        ]]
    );
}

#[test]
fn esc_interrupts_the_turn() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Esc));
    let ops: Vec<Op> = rx
        .try_iter()
        .filter_map(|event| match event {
            AppEvent::Op(op) => Some(op),
            _ => None,
        })
        .collect();
    assert_eq!(ops, vec![Op::Interrupt]);
    assert!(view.is_complete());
}

#[test]
fn queued_requests_follow_and_a_dismissed_one_is_skipped() {
    let (mut view, rx) = overlay(two_questions(1));
    let second = two_questions(2);
    let second_id = second.request_id;
    view.try_consume_user_input_request(second);
    view.try_consume_user_input_request(two_questions(3));
    // The first was withdrawn (its turn ended elsewhere).
    view.dismiss_user_input(view.request.request_id);
    assert_eq!(view.request.request_id, second_id);
    for _ in 0..4 {
        view.handle_key_event(key(KeyCode::Enter));
    }
    assert_eq!(answers(&rx).len(), 2);
    assert!(view.is_complete());
}

#[test]
fn request_user_input_renders_the_current_question() {
    let (mut view, _rx) = overlay(two_questions(1));
    assert_snapshot!("request_user_input_first_question", render(&view, 60));
    view.handle_key_event(key(KeyCode::Right));
    assert_snapshot!("request_user_input_second_question", render(&view, 60));
}
