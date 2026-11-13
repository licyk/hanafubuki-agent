// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::app_event::{AppEvent, Op};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyModifiers;

/// The Agent's requests as a turn would carry them, read from JSON: their
/// identifiers have no public constructor.
pub(super) mod fixtures {
    use hanafubuki_agent::{AgentUserInputRequest, ApprovalRequest};

    pub(crate) fn approval_request(
        id: u64,
        summary: &str,
        offers_session: bool,
    ) -> ApprovalRequest {
        serde_json::from_value(serde_json::json!({
            "requestId": id,
            "operationId": format!("call-{id}"),
            "action": {
                "kind": "command_execution",
                "risk": "medium",
                "origin": {"type": "model"},
                "summary": summary,
                "digest": "00",
                "details": null
            },
            "requestedAtMs": 0,
            "expiresAtMs": 0,
            "offersAllowForSession": offers_session
        }))
        .unwrap()
    }

    pub(crate) fn user_input_request(
        id: u64,
        questions: serde_json::Value,
    ) -> AgentUserInputRequest {
        serde_json::from_value(serde_json::json!({
            "requestId": id,
            "callId": format!("call-{id}"),
            "questions": questions,
            "requestedAt": 0
        }))
        .unwrap()
    }
}

fn snapshot_buffer(buf: &Buffer) -> String {
    let mut lines = Vec::new();
    for y in 0..buf.area().height {
        let mut row = String::new();
        for x in 0..buf.area().width {
            row.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        lines.push(row);
    }
    lines.join("\n")
}

fn render_snapshot(pane: &BottomPane, area: Rect) -> String {
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);
    snapshot_buffer(&buf)
}

fn pane() -> (BottomPane, Receiver<AppEvent>) {
    let (tx_raw, rx) = channel::<AppEvent>();
    let pane = BottomPane::new(BottomPaneParams {
        app_event_tx: AppEventSender::new(tx_raw),
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Codex to do anything".to_string(),
    });
    (pane, rx)
}

fn exec_request() -> ApprovalRequest {
    fixtures::approval_request(1, "Run Posix command in /work: echo ok", false)
}

fn ops(rx: &Receiver<AppEvent>) -> Vec<Op> {
    rx.try_iter()
        .filter_map(|event| match event {
            AppEvent::Op(op) => Some(op),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_c_on_modal_consumes_it() {
    let (mut pane, _rx) = pane();
    pane.push_approval_request(exec_request());
    assert_eq!(CancellationEvent::Handled, pane.on_ctrl_c());
    assert!(pane.no_modal_or_popup_active());
    assert_eq!(CancellationEvent::NotHandled, pane.on_ctrl_c());
}

#[test]
fn overlay_not_shown_above_approval_modal() {
    let (mut pane, _rx) = pane();

    // Create an approval modal (active view).
    pane.push_approval_request(exec_request());

    // Render and verify the top row does not include an overlay.
    let area = Rect::new(0, 0, 60, 6);
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);

    let mut r0 = String::new();
    for x in 0..area.width {
        r0.push(buf[(x, 0)].symbol().chars().next().unwrap_or(' '));
    }
    assert!(
        !r0.contains("Working"),
        "overlay should not render above modal"
    );
}

#[test]
fn composer_shown_after_denied_while_task_running() {
    let (mut pane, rx) = pane();

    // Start a running task so the status indicator is active above the composer.
    pane.set_task_running(true);

    // Push an approval modal which should hide the status view.
    pane.push_approval_request(exec_request());

    // Simulate pressing 'n' (No) on the modal.
    pane.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

    // After denial, since the task is still running, the status indicator should be
    // visible above the composer. The modal should be gone.
    assert!(
        pane.view_stack.is_empty(),
        "no active modal view after denial"
    );
    assert!(matches!(
        ops(&rx).as_slice(),
        [Op::Approval {
            decision: hanafubuki_agent::ApprovalUserDecision::Deny,
            ..
        }]
    ));

    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);
    let mut row0 = String::new();
    for x in 0..area.width {
        row0.push(buf[(x, 0)].symbol().chars().next().unwrap_or(' '));
    }
    assert!(
        row0.contains("Working"),
        "expected Working header after denial on row 0: {row0:?}"
    );

    // Composer placeholder should be visible somewhere below.
    let found_composer = (1..area.height).any(|y| {
        let row: String = (0..area.width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        row.contains("Ask Codex")
    });
    assert!(
        found_composer,
        "expected composer visible under status line"
    );
}

#[test]
fn status_indicator_visible_during_command_execution() {
    let (mut pane, _rx) = pane();

    // Begin a task: show initial status.
    pane.set_task_running(true);

    // Use a height that allows the status line to be visible above the composer.
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);

    let bufs = snapshot_buffer(&buf);
    assert!(bufs.contains("• Working"), "expected Working header");
}

#[test]
fn status_and_composer_fill_height_without_bottom_padding() {
    let (mut pane, _rx) = pane();

    pane.set_task_running(true);

    // Use height == desired_height; expect spacer + status + composer rows without trailing padding.
    let height = pane.desired_height(30);
    assert!(
        height >= 3,
        "expected at least 3 rows to render spacer, status, and composer; got {height}"
    );
    let area = Rect::new(0, 0, 30, height);
    assert_snapshot!(
        "status_and_composer_fill_height_without_bottom_padding",
        render_snapshot(&pane, area)
    );
}

#[test]
fn status_only_snapshot() {
    let (mut pane, _rx) = pane();

    pane.set_task_running(true);

    let width = 48;
    let height = pane.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    assert_snapshot!("status_only_snapshot", render_snapshot(&pane, area));
}

#[test]
fn esc_interrupts_running_task_when_no_popup() {
    let (mut pane, rx) = pane();

    pane.set_task_running(true);

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        ops(&rx),
        vec![Op::Interrupt],
        "expected Esc to send Op::Interrupt while a task is running"
    );
}

#[test]
fn approvals_queue_behind_the_open_one_and_dismissals_skip_them() {
    let (mut pane, rx) = pane();
    pane.push_approval_request(fixtures::approval_request(1, "first", false));
    let second = fixtures::approval_request(2, "second", false);
    let second_id = second.request_id;
    pane.push_approval_request(second);
    pane.push_approval_request(fixtures::approval_request(3, "third", false));

    // The second was answered by a policy: it is dropped without an answer.
    pane.dismiss_approval(second_id);
    pane.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    pane.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    let answered: Vec<u64> = ops(&rx)
        .into_iter()
        .map(|op| match op {
            Op::Approval { request_id, .. } => request_id.value(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(answered, vec![1, 3]);
    assert!(pane.no_modal_or_popup_active());
}

#[test]
fn dismissing_every_request_closes_the_modals_without_answers() {
    let (mut pane, rx) = pane();
    pane.push_approval_request(exec_request());
    pane.dismiss_all_requests();
    assert!(pane.no_modal_or_popup_active());
    assert!(ops(&rx).is_empty());
}

#[test]
fn submitted_text_comes_back_from_the_composer() {
    let (mut pane, _rx) = pane();
    pane.set_composer_text("hello".to_string());
    assert_eq!(
        pane.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        InputResult::Submitted("hello".into())
    );
    assert!(pane.composer_is_empty());
}

#[test]
fn esc_interrupts_while_the_status_line_is_hidden() {
    let (mut pane, rx) = pane();
    pane.set_task_running(true);
    // The answer streams.
    pane.hide_status_indicator();
    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(ops(&rx), vec![Op::Interrupt]);
}

#[test]
fn waiting_messages_show_between_the_status_and_the_composer() {
    let (mut pane, _rx) = pane();
    pane.set_task_running(true);
    pane.set_pending_input(PendingInput {
        steers: vec!["Check the tests first.".to_string()],
        held: Vec::new(),
        queued: vec!["Then update the docs.".to_string()],
    });
    pane.set_composer_text("A draft".to_string());
    let width = 60;
    let area = Rect::new(0, 0, width, pane.desired_height(width));
    assert_snapshot!(
        "status_pending_input_and_composer",
        render_snapshot(&pane, area)
    );
}

#[test]
fn waiting_messages_keep_their_rows_until_the_turn_ends() {
    let (mut pane, _rx) = pane();
    pane.set_task_running(true);
    let width = 60;
    let idle = pane.desired_height(width);
    let steer = |text: &str| PendingInput {
        steers: vec![text.to_string()],
        held: Vec::new(),
        queued: Vec::new(),
    };
    pane.set_pending_input(steer("Check the tests first."));
    let waiting = pane.desired_height(width);
    assert!(waiting > idle);
    // The turn reads it: its rows stay, blank, and the next message's
    // take their place.
    pane.set_pending_input(PendingInput::default());
    assert_eq!(pane.desired_height(width), waiting);
    let area = Rect::new(0, 0, width, waiting);
    assert!(!render_snapshot(&pane, area).contains("Check the tests"));
    pane.set_pending_input(steer("Then the docs."));
    assert_eq!(pane.desired_height(width), waiting);
    pane.set_pending_input(PendingInput::default());
    pane.set_task_running(false);
    assert!(pane.desired_height(width) < idle);
}

#[test]
fn tabs_within_a_paste_burst_are_the_pastes_and_send_nothing() {
    for running in [false, true] {
        let (mut pane, rx) = pane();
        pane.set_task_running(running);
        let keys = [
            KeyCode::Char('a'),
            KeyCode::Char('b'),
            KeyCode::Char('c'),
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Tab,
            KeyCode::Char('d'),
        ];
        for code in keys {
            let result = pane.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
            assert_eq!(
                result,
                InputResult::None,
                "{code:?} while running: {running}"
            );
        }
        // Buffering is active, so its idle timeout applies (longer on Windows).
        std::thread::sleep(PasteBurst::recommended_active_flush_delay());
        assert!(pane.flush_paste_burst_if_due());
        assert_eq!(pane.composer_text(), "abc\n\t\td");
        assert!(ops(&rx).is_empty());
    }
}
