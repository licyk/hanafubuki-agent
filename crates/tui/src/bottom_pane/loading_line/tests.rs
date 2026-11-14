use std::sync::mpsc::channel;

use super::*;
use crate::app_event::{AppEvent, AppEventSender};

fn text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn the_wait_is_counted_from_its_first_second() {
    let mut loading = LoadingLine::still();
    assert_eq!(text(&loading.line("Listing models…")), "• Listing models…");

    // A test process starts well after the machine's clock.
    loading.start = Instant::now()
        .checked_sub(Duration::from_secs(90))
        .expect("the clock started over 90 s ago");
    let line = text(&loading.line("Listing models…"));
    assert!(line.starts_with("• Listing models… · 1m "), "{line}");
}

#[test]
fn each_drawing_asks_for_the_next_frame() {
    let (sender, events) = channel();
    let loading = LoadingLine::new(FrameRequester::new(AppEventSender::new(sender)), true);
    assert!(
        events.try_recv().is_err(),
        "nothing is asked before a drawing"
    );
    let before = Instant::now();
    loading.line("Listing models…");
    assert!(matches!(
        events.try_recv(),
        Ok(AppEvent::ScheduleFrame(at)) if at >= before + NEXT_FRAME
    ));
}
