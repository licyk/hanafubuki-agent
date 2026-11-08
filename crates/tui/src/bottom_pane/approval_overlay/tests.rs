// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::bottom_pane::tests::fixtures::approval_request;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyModifiers;

fn make_exec_request() -> ApprovalRequest {
    approval_request(1, "Run Posix command in /work: echo hi", false)
}

fn overlay(request: ApprovalRequest) -> (ApprovalOverlay, Receiver<AppEvent>) {
    let (tx, rx) = channel::<AppEvent>();
    (ApprovalOverlay::new(request, AppEventSender::new(tx)), rx)
}

fn decisions(rx: &Receiver<AppEvent>) -> Vec<ApprovalUserDecision> {
    rx.try_iter()
        .filter_map(|event| match event {
            AppEvent::Op(Op::Approval { decision, .. }) => Some(decision),
            _ => None,
        })
        .collect()
}

fn render(view: &ApprovalOverlay, width: u16) -> String {
    let height = view.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    (0..area.height)
        .map(|row| {
            (0..area.width)
                .map(|col| buf[(col, row)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn ctrl_c_denies_and_clears_queue() {
    let (mut view, rx) = overlay(make_exec_request());
    view.enqueue_request(approval_request(2, "second", false));
    assert_eq!(CancellationEvent::Handled, view.on_ctrl_c());
    assert!(view.queue.is_empty());
    assert!(view.is_complete());
    assert_eq!(
        decisions(&rx),
        vec![ApprovalUserDecision::Deny, ApprovalUserDecision::Deny]
    );
}

#[test]
fn shortcut_triggers_selection() {
    let (mut view, rx) = overlay(make_exec_request());
    assert!(!view.is_complete());
    view.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(decisions(&rx), vec![ApprovalUserDecision::AllowOnce]);
    assert!(view.is_complete());
}

#[test]
fn esc_denies_the_request() {
    let (mut view, rx) = overlay(make_exec_request());
    view.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(decisions(&rx), vec![ApprovalUserDecision::Deny]);
    assert!(view.is_complete());
}

#[test]
fn session_option_only_when_offered() {
    let (mut view, rx) = overlay(make_exec_request());
    assert_eq!(view.options.len(), 2);
    view.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(decisions(&rx).is_empty());
    assert!(!view.is_complete());

    let (mut view, rx) = overlay(approval_request(1, "echo hi", true));
    assert_eq!(view.options.len(), 3);
    view.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(decisions(&rx), vec![ApprovalUserDecision::AllowForSession]);
}

#[test]
fn header_includes_the_summary_without_terminal_controls() {
    let (view, _rx) = overlay(approval_request(
        1,
        "Run Posix command in /work: echo \u{1b}[31mhello world",
        false,
    ));
    let rendered = render(&view, 80);
    assert!(
        rendered.contains("echo hello world"),
        "expected header to include command snippet, got {rendered}"
    );
}

#[test]
fn approval_overlay_renders_the_question_summary_and_options() {
    let (view, _rx) = overlay(approval_request(
        1,
        "Run Posix command in /work: cargo test",
        true,
    ));
    assert_snapshot!("approval_overlay_command", render(&view, 60));
}

#[test]
fn history_cell_records_the_decision() {
    let (mut view, rx) = overlay(make_exec_request());
    view.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    let cell = rx
        .try_iter()
        .find_map(|event| match event {
            AppEvent::InsertHistoryCell(cell) => Some(cell),
            _ => None,
        })
        .expect("a decision cell");
    let text: String = cell
        .display_lines(80)
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
        .collect();
    assert_eq!(
        text,
        "✗ You did not approve Run Posix command in /work: echo hi"
    );
}

#[test]
fn enter_selects_the_highlighted_option() {
    let (mut view, rx) = overlay(make_exec_request());
    view.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(
        view.is_complete(),
        "approval should complete without queued requests"
    );
    assert_eq!(decisions(&rx), vec![ApprovalUserDecision::AllowOnce]);
}

#[test]
fn queued_requests_are_asked_in_order() {
    let (mut view, rx) = overlay(make_exec_request());
    view.enqueue_request(approval_request(2, "second", false));
    view.enqueue_request(approval_request(3, "third", false));
    for _ in 0..3 {
        view.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    }
    let ids: Vec<u64> = rx
        .try_iter()
        .filter_map(|event| match event {
            AppEvent::Op(Op::Approval { request_id, .. }) => Some(request_id.value()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, vec![1, 2, 3]);
    assert!(view.is_complete());
}
