use std::sync::mpsc::channel;

use super::*;

/// A reply comes even when the read never ran or panicked, so the call's
/// entry is never left waiting.
#[test]
fn a_display_reply_is_sent_when_dropped_with_or_without_a_display() {
    let (sender, events) = channel();
    let reply = |call_id: &str| DisplayReply {
        app_event_tx: AppEventSender::new(sender.clone()),
        session_id: AgentSessionId::new(3),
        call_id: call_id.to_string(),
        display: None,
    };

    drop(reply("never-read"));
    let mut read = reply("read");
    read.shows(Some(AgentToolCallDisplay {
        subject: Some("notes.txt".to_string()),
        display: None,
    }));
    drop(read);
    let unwound = reply("panicked");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _reply = unwound;
        panic!("the read panicked");
    }));

    let replies: Vec<_> = events
        .try_iter()
        .map(|event| match event {
            AppEvent::AgentReply(AgentReply::ToolDisplayed {
                session_id,
                call_id,
                display,
            }) => (
                session_id.value(),
                call_id,
                display.and_then(|shown| shown.subject),
            ),
            _ => panic!("not a display reply"),
        })
        .collect();
    assert_eq!(
        replies,
        vec![
            (3, "never-read".to_string(), None),
            (3, "read".to_string(), Some("notes.txt".to_string())),
            (3, "panicked".to_string(), None),
        ]
    );
}
