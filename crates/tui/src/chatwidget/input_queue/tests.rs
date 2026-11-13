// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

fn start(text: &str) -> Submission {
    Submission::Start(text.to_string())
}

fn steer(id: u64, text: &str) -> Submission {
    Submission::Steer {
        id,
        text: text.to_string(),
    }
}

/// A queue whose turn started with "first" and runs.
fn running() -> InputQueue {
    let mut queue = InputQueue::default();
    assert_eq!(queue.submit("first".to_string(), false), start("first"));
    queue.turn_started();
    queue
}

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|text| text.to_string()).collect()
}

#[test]
fn enter_before_the_turn_starts_queues_and_waits_for_it() {
    let mut queue = InputQueue::default();
    assert_eq!(queue.submit("first".to_string(), false), start("first"));
    assert!(queue.is_busy());
    // The turn has not started: the next message is a turn of its own.
    assert_eq!(queue.submit("second".to_string(), false), Submission::Waits);
    assert_eq!(queue.preview().queued, texts(&["second"]));
    queue.turn_started();
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), Some(Settled::Start("second".to_string())));
    queue.turn_started();
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), None);
    assert!(!queue.is_busy());
}

#[test]
fn steers_go_one_at_a_time_and_are_written_once_the_turn_reads_them() {
    let mut queue = running();
    assert_eq!(queue.submit("a".to_string(), true), steer(0, "a"));
    // The first has no answer yet: the second waits behind it.
    assert_eq!(queue.submit("b".to_string(), true), Submission::Waits);
    assert_eq!(queue.next_steer(), None);
    assert_eq!(queue.steered(0, SteerOutcome::Accepted(1)), None);
    assert_eq!(queue.next_steer(), Some((1, "b".to_string())));
    assert_eq!(queue.steered(1, SteerOutcome::Accepted(2)), None);
    assert_eq!(queue.preview().steers, texts(&["a", "b"]));
    assert_eq!(queue.consumed(2), texts(&["a", "b"]));
    assert!(queue.preview().is_empty());
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), None);
}

#[test]
fn a_steer_the_turn_read_before_its_answer_came_is_written_with_the_answer() {
    let mut queue = running();
    assert_eq!(queue.submit("a".to_string(), true), steer(0, "a"));
    assert_eq!(queue.consumed(1), Vec::<String>::new());
    assert_eq!(
        queue.steered(0, SteerOutcome::Accepted(1)),
        Some("a".to_string())
    );
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), None);
}

#[test]
fn a_steer_racing_the_turns_end_becomes_the_next_turn() {
    let mut queue = running();
    assert_eq!(queue.submit("late".to_string(), true), steer(0, "late"));
    queue.turn_ended(Ending::Finished);
    // Its answer has not come: nothing is decided yet, and Enter queues.
    assert_eq!(queue.settle(), None);
    assert_eq!(queue.submit("after".to_string(), true), Submission::Waits);
    assert_eq!(queue.steered(0, SteerOutcome::NotDelivered), None);
    assert_eq!(queue.next_steer(), None);
    assert_eq!(queue.settle(), Some(Settled::Start("late".to_string())));
    queue.turn_started();
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), Some(Settled::Start("after".to_string())));
}

#[test]
fn a_steer_the_running_turn_refused_is_held_for_its_end_with_the_unread_ones() {
    let mut queue = running();
    assert_eq!(
        queue.submit("refused".to_string(), true),
        steer(0, "refused")
    );
    assert_eq!(queue.steered(0, SteerOutcome::NotDelivered), None);
    assert_eq!(queue.preview().held, texts(&["refused"]));
    assert_eq!(queue.submit("unread".to_string(), true), steer(1, "unread"));
    assert_eq!(queue.steered(1, SteerOutcome::Accepted(1)), None);
    assert_eq!(queue.queue("queued".to_string(), true), Submission::Waits);
    // A failed turn never read "unread": both go as one turn, then the
    // queued message as its own.
    queue.turn_ended(Ending::Finished);
    assert_eq!(
        queue.settle(),
        Some(Settled::Start("refused\nunread".to_string()))
    );
    queue.turn_started();
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), Some(Settled::Start("queued".to_string())));
}

#[test]
fn the_queue_drains_one_turn_at_a_time() {
    let mut queue = running();
    for text in ["one", "two"] {
        assert_eq!(queue.queue(text.to_string(), true), Submission::Waits);
    }
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), Some(Settled::Start("one".to_string())));
    assert_eq!(queue.preview().queued, texts(&["two"]));
    queue.turn_started();
    queue.turn_ended(Ending::Finished);
    assert_eq!(queue.settle(), Some(Settled::Start("two".to_string())));
}

#[test]
fn an_interrupt_puts_what_waits_back_in_the_composer() {
    let mut queue = running();
    assert_eq!(queue.submit("steer".to_string(), true), steer(0, "steer"));
    assert_eq!(queue.steered(0, SteerOutcome::Accepted(1)), None);
    assert_eq!(queue.queue("queued".to_string(), true), Submission::Waits);
    // Ctrl-C: no flag.
    queue.turn_ended(Ending::Interrupted);
    assert_eq!(
        queue.settle(),
        Some(Settled::Restore("steer\nqueued".to_string()))
    );
    assert!(!queue.is_busy());
}

#[test]
fn esc_with_steers_waiting_sends_them_at_once_and_keeps_the_queue() {
    let mut queue = running();
    assert_eq!(queue.submit("a".to_string(), true), steer(0, "a"));
    assert_eq!(queue.steered(0, SteerOutcome::Accepted(1)), None);
    assert_eq!(queue.submit("b".to_string(), true), steer(1, "b"));
    assert_eq!(queue.queue("later".to_string(), true), Submission::Waits);
    assert_eq!(queue.esc(), EscAction::Interrupt);
    assert!(queue.sends_steers_after_interrupt());
    queue.turn_ended(Ending::Interrupted);
    // "b" has no answer yet: the cancelled turn may have taken it.
    assert_eq!(queue.settle(), None);
    assert_eq!(queue.steered(1, SteerOutcome::Accepted(2)), None);
    assert_eq!(queue.settle(), Some(Settled::Start("a\nb".to_string())));
    assert_eq!(queue.preview().queued, texts(&["later"]));
}

#[test]
fn a_steer_the_interrupt_refuses_before_the_turns_end_is_sent_with_the_others() {
    let mut queue = running();
    assert_eq!(queue.submit("a".to_string(), true), steer(0, "a"));
    assert_eq!(queue.submit("b".to_string(), true), Submission::Waits);
    assert_eq!(queue.esc(), EscAction::Interrupt);
    assert_eq!(queue.steered(0, SteerOutcome::Accepted(1)), None);
    // The turn being interrupted is sent nothing more.
    assert_eq!(queue.next_steer(), None);
    assert_eq!(queue.submit("c".to_string(), true), Submission::Waits);
    queue.turn_ended(Ending::Interrupted);
    assert_eq!(queue.settle(), Some(Settled::Start("a\nb\nc".to_string())));

    let mut queue = running();
    assert_eq!(queue.submit("a".to_string(), true), steer(0, "a"));
    assert_eq!(queue.steered(0, SteerOutcome::Accepted(1)), None);
    assert_eq!(queue.submit("b".to_string(), true), steer(1, "b"));
    assert_eq!(queue.esc(), EscAction::Interrupt);
    // The cancelled turn refuses "b" before its end arrives.
    assert_eq!(queue.steered(1, SteerOutcome::NotDelivered), None);
    assert!(queue.preview().held.is_empty());
    queue.turn_ended(Ending::Interrupted);
    assert_eq!(queue.settle(), Some(Settled::Start("a\nb".to_string())));
}

#[test]
fn esc_with_no_steer_waiting_puts_the_queue_back() {
    let mut queue = running();
    assert_eq!(queue.queue("queued".to_string(), true), Submission::Waits);
    assert_eq!(queue.esc(), EscAction::Interrupt);
    assert!(!queue.sends_steers_after_interrupt());
    queue.turn_ended(Ending::Interrupted);
    assert_eq!(queue.settle(), Some(Settled::Restore("queued".to_string())));
}

#[test]
fn a_review_holds_enter_for_its_end_and_esc_does_not_cancel_it() {
    let mut queue = InputQueue::default();
    queue.review_started();
    assert_eq!(queue.esc(), EscAction::Interrupt);
    assert_eq!(queue.submit("a".to_string(), true), Submission::Waits);
    assert_eq!(queue.submit("b".to_string(), true), Submission::Waits);
    assert_eq!(queue.queue("queued".to_string(), true), Submission::Waits);
    assert_eq!(queue.esc(), EscAction::WarnReview);
    queue.review_ended(Ending::Finished);
    assert_eq!(queue.settle(), Some(Settled::Start("a\nb".to_string())));

    let mut queue = InputQueue::default();
    queue.review_started();
    assert_eq!(queue.submit("held".to_string(), true), Submission::Waits);
    queue.review_ended(Ending::Interrupted);
    assert_eq!(queue.settle(), Some(Settled::Restore("held".to_string())));
}

#[test]
fn tab_with_nothing_running_starts_a_turn() {
    let mut queue = InputQueue::default();
    assert_eq!(queue.queue("now".to_string(), false), start("now"));
}

#[test]
fn shift_left_takes_the_last_queued_message_then_the_held_ones() {
    let mut queue = running();
    assert_eq!(
        queue.submit("refused".to_string(), true),
        steer(0, "refused")
    );
    assert_eq!(queue.steered(0, SteerOutcome::NotDelivered), None);
    for text in ["one", "two"] {
        queue.queue(text.to_string(), true);
    }
    assert_eq!(queue.pop_queued(), Some("two".to_string()));
    assert_eq!(queue.pop_queued(), Some("one".to_string()));
    assert_eq!(queue.pop_queued(), Some("refused".to_string()));
    assert_eq!(queue.pop_queued(), None);
}
