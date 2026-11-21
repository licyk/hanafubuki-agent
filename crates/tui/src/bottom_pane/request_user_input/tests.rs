use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::bottom_pane::tests::fixtures::user_input_request;
use hanafubuki_agent::AgentUserInputResponse;
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
                    .map(|answer| {
                        let value = match answer.response {
                            AgentUserInputResponse::Option { option_id } => option_id,
                            AgentUserInputResponse::Text { text } => format!("text:{text}"),
                        };
                        (answer.question_id, value)
                    })
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
fn a_question_that_takes_one_answers_in_the_persons_own_words() {
    let (mut view, rx) = overlay(user_input_request(
        7,
        serde_json::json!([{
            "id": "why",
            "header": "Why",
            "question": "Why this approach?",
            "isOther": true,
            "options": [
                {"id": "speed", "label": "Speed", "description": ""},
                {"id": "cost", "label": "Cost", "description": ""}
            ]
        }]),
    ));

    // Typing goes into the answer in their own words, whatever row the
    // cursor started on.
    for ch in "because it is safer".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![(
            "why".to_string(),
            "text:because it is safer".to_string()
        )]]
    );
}

#[test]
fn a_question_without_is_other_takes_no_free_text() {
    let (mut view, rx) = overlay(two_questions(1));

    for ch in "abc".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "tui".to_string()),
            ("tests".to_string(), "yes".to_string()),
        ]]
    );
}

#[test]
fn alt_and_a_number_jumps_to_that_question() {
    let (mut view, rx) = overlay(two_questions(1));

    view.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "tui".to_string()),
            ("tests".to_string(), "yes".to_string()),
        ]]
    );
}

#[test]
fn clicking_a_row_selects_it() {
    let request = two_questions(1);
    let (mut view, rx) = overlay(request);
    let area = Rect::new(0, 0, 60, view.desired_height(60));
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    // The second option's row, found by its own text.
    let row = (0..area.height)
        .find(|row| {
            (0..area.width)
                .map(|col| buf[(col, *row)].symbol().to_string())
                .collect::<String>()
                .contains("The TUI and the CLI")
        })
        .expect("the second option is shown");
    let mouse = ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(
            ratatui::crossterm::event::MouseButton::Left,
        ),
        column: 4,
        row,
        modifiers: KeyModifiers::NONE,
    };

    let clicked = view.pointer_key(mouse);

    assert!(clicked.is_some(), "the click is taken");
    view.handle_key_event(key(clicked.unwrap()));
    view.handle_key_event(key(KeyCode::Enter));
    assert_eq!(
        answers(&rx).first().map(|batch| batch[0].1.clone()),
        Some("both".to_string())
    );
}

#[test]
fn a_request_that_does_not_block_answers_itself_after_the_countdown() {
    let mut request = two_questions(1);
    request.is_blocking = false;
    let (mut view, rx) = overlay(request);
    // It has been on screen past the hidden grace and the countdown.
    view.shown_at = std::time::Instant::now()
        - AUTO_RESOLUTION_HIDDEN_GRACE
        - AUTO_RESOLUTION_VISIBLE_COUNTDOWN;

    assert!(view.on_deadline(), "the deadline is acted on");

    assert_eq!(answers(&rx), vec![Vec::new()], "answered with nothing");
    assert!(view.is_complete());
}

#[test]
fn a_blocking_request_never_answers_itself() {
    let (mut view, rx) = overlay(two_questions(1));
    view.shown_at = std::time::Instant::now()
        - AUTO_RESOLUTION_HIDDEN_GRACE
        - AUTO_RESOLUTION_VISIBLE_COUNTDOWN;

    assert!(!view.on_deadline());

    assert!(answers(&rx).is_empty());
    assert!(view.next_deadline().is_none(), "a blocking request waits");
}

#[test]
fn a_request_the_person_touched_never_answers_itself() {
    let mut request = two_questions(1);
    request.is_blocking = false;
    let (mut view, rx) = overlay(request);
    view.handle_key_event(key(KeyCode::Down));
    view.shown_at = std::time::Instant::now()
        - AUTO_RESOLUTION_HIDDEN_GRACE
        - AUTO_RESOLUTION_VISIBLE_COUNTDOWN;

    assert!(!view.on_deadline());

    assert!(answers(&rx).is_empty());
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
