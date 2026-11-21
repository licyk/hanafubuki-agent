use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::bottom_pane::tests::fixtures::user_input_request;
use crate::render::renderable::Renderable;
use hanafubuki_agent::AgentUserInputResponse;
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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
                "question": "Should tests come with it?",
                "options": [
                    {"id": "yes", "label": "Yes", "description": ""},
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

fn interrupted(rx: &Receiver<AppEvent>) -> bool {
    rx.try_iter()
        .any(|event| matches!(event, AppEvent::Op(Op::Interrupt)))
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

/// The row of `needle` in the rendered view.
fn row_of(view: &RequestUserInputOverlay, needle: &str) -> u16 {
    let width = 60;
    let area = Rect::new(0, 0, width, view.desired_height(width));
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    (0..area.height)
        .find(|row| {
            (0..area.width)
                .map(|col| buf[(col, *row)].symbol().to_string())
                .collect::<String>()
                .contains(needle)
        })
        .expect("the row is shown")
}

fn click(view: &mut RequestUserInputOverlay, row: u16) {
    let mouse = ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(
            ratatui::crossterm::event::MouseButton::Left,
        ),
        column: 4,
        row,
        modifiers: KeyModifiers::NONE,
    };
    if let Some(key) = view.pointer_key(mouse) {
        view.handle_key_event(KeyEvent::new(key, KeyModifiers::NONE));
    }
}

#[test]
fn enter_answers_each_question_then_reviews_and_submits() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Down));
    view.handle_key_event(key(KeyCode::Enter));
    assert!(answers(&rx).is_empty(), "the first Enter only moves on");
    view.handle_key_event(key(KeyCode::Enter));
    assert!(answers(&rx).is_empty(), "the last answer opens the review");

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
fn the_review_page_shows_every_answer_before_it_is_submitted() {
    let (mut view, _rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Char('2')));
    view.handle_key_event(key(KeyCode::Enter));

    let review = render(&view, 70);

    assert!(review.contains("Review"), "{review}");
    assert!(
        review.contains("Which crates should the change touch? → The TUI and the CLI"),
        "{review}"
    );
    assert!(review.contains("Submit the answers"), "{review}");
    assert!(review.contains("Cancel, answering nothing"), "{review}");
}

#[test]
fn cancelling_the_review_answers_nothing_and_interrupts() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Down));

    view.handle_key_event(key(KeyCode::Enter));

    let events = rx.try_iter().collect::<Vec<_>>();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::Op(Op::UserInputAnswer { .. }))),
        "nothing is answered"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Op(Op::Interrupt))),
        "the turn is interrupted"
    );
    assert!(view.is_complete());
}

#[test]
fn going_back_from_the_review_keeps_the_answers() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Char('2')));
    view.handle_key_event(key(KeyCode::Char('2')));
    assert_eq!(view.stage, Stage::Review);

    view.handle_key_event(key(KeyCode::Left));
    // Back on the last question, whose answer is kept; Enter returns to the
    // review page, and Enter again submits.
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "both".to_string()),
            ("tests".to_string(), "no".to_string()),
        ]]
    );
}

fn free_form() -> AgentUserInputRequest {
    user_input_request(
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
    )
}

#[test]
fn writing_is_entered_explicitly_and_then_takes_every_key() {
    let (mut view, _rx) = overlay(free_form());
    // Moving onto the row does not start writing, so the options stay
    // reachable.
    view.handle_key_event(key(KeyCode::Down));
    view.handle_key_event(key(KeyCode::Down));
    assert!(!view.is_writing());
    // Choosing an option still works from there.
    view.handle_key_event(key(KeyCode::Up));
    view.handle_key_event(key(KeyCode::Enter));
    assert_eq!(view.stage, Stage::Review);

    // A digit on the review page goes back out of it, so start again.
    let (mut view, rx) = overlay(free_form());
    view.handle_key_event(key(KeyCode::Char('i')));
    assert!(view.is_writing());
    // Digits and letters are text now, never shortcuts.
    for ch in "1 of 2 reasons: k".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }
    view.handle_key_event(key(KeyCode::Enter));
    assert!(!view.is_writing());
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![(
            "why".to_string(),
            "text:1 of 2 reasons: k".to_string()
        )]]
    );
}

#[test]
fn the_own_answer_row_is_entered_with_enter() {
    let (mut view, rx) = overlay(free_form());
    for _ in 0..2 {
        view.handle_key_event(key(KeyCode::Down));
    }

    view.handle_key_event(key(KeyCode::Enter));
    assert!(view.is_writing());
    for ch in "because".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![("why".to_string(), "text:because".to_string())]]
    );
}

#[test]
fn esc_while_writing_stops_writing_and_keeps_the_text() {
    let (mut view, _rx) = overlay(free_form());
    view.handle_key_event(key(KeyCode::Char('i')));
    for ch in "half".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }

    view.handle_key_event(key(KeyCode::Esc));

    assert!(!view.is_writing(), "it stops writing");
    assert!(!view.is_complete(), "the turn is not interrupted");
    assert_eq!(view.answer_label(0), "half");
}

#[test]
fn backspace_removes_the_last_character_written() {
    let (mut view, rx) = overlay(free_form());
    view.handle_key_event(key(KeyCode::Char('i')));
    for ch in "ox".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }
    view.handle_key_event(key(KeyCode::Backspace));
    view.handle_key_event(key(KeyCode::Char('k')));
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![("why".to_string(), "text:ok".to_string())]]
    );
}

#[test]
fn a_question_without_free_text_takes_no_keys_as_text() {
    let (mut view, rx) = overlay(two_questions(1));

    for ch in "abc".chars() {
        view.handle_key_event(key(KeyCode::Char(ch)));
    }
    view.handle_key_event(key(KeyCode::Enter));
    assert!(!view.is_writing(), "there is nothing to write");
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "tui".to_string()),
            ("tests".to_string(), "yes".to_string()),
        ]],
        "the cursor never moved, so the first option of each is answered"
    );
}

#[test]
fn a_digit_picks_that_option_and_moves_on() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Char('2')));
    view.handle_key_event(key(KeyCode::Char('2')));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "both".to_string()),
            ("tests".to_string(), "no".to_string()),
        ]]
    );
}

#[test]
fn alt_and_a_number_jumps_to_that_question() {
    let (mut view, rx) = overlay(two_questions(1));

    view.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT));
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
fn clicking_a_row_selects_it() {
    let (mut view, rx) = overlay(two_questions(1));
    let row = row_of(&view, "The TUI and the CLI");

    click(&mut view, row);
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        answers(&rx).first().map(|batch| batch[0].1.clone()),
        Some("both".to_string())
    );
}

#[test]
fn clicking_submit_on_the_review_page_submits() {
    let (mut view, rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Enter));
    view.handle_key_event(key(KeyCode::Enter));
    let row = row_of(&view, "Submit the answers");

    click(&mut view, row);

    assert_eq!(
        answers(&rx),
        vec![vec![
            ("scope".to_string(), "tui".to_string()),
            ("tests".to_string(), "yes".to_string()),
        ]]
    );
}

#[test]
fn queued_requests_follow_and_a_dismissed_one_is_skipped() {
    let (mut view, rx) = overlay(two_questions(1));
    let third = two_questions(3);
    let third_id = third.request_id;
    assert!(view
        .try_consume_user_input_request(two_questions(2))
        .is_none());
    assert!(view.try_consume_user_input_request(third).is_none());

    // A request answered or withdrawn elsewhere leaves the rest alone.
    view.dismiss_user_input(third_id);

    // Each request takes three Enters: one per question, then the review.
    for _ in 0..6 {
        view.handle_key_event(key(KeyCode::Enter));
    }

    assert_eq!(
        answers(&rx).len(),
        2,
        "the two requests that were not dismissed are answered"
    );
    assert!(view.is_complete());
}

#[test]
fn esc_interrupts_the_turn() {
    let (mut view, rx) = overlay(two_questions(1));

    view.handle_key_event(key(KeyCode::Esc));

    assert!(interrupted(&rx));
    assert!(view.is_complete());
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
fn request_user_input_renders_the_current_question() {
    let (view, _rx) = overlay(two_questions(1));

    assert_snapshot!("request_user_input_first_question", render(&view, 60));
}

#[test]
fn request_user_input_renders_the_second_question() {
    let (mut view, _rx) = overlay(two_questions(1));
    view.handle_key_event(key(KeyCode::Right));

    assert_snapshot!("request_user_input_second_question", render(&view, 60));
}
