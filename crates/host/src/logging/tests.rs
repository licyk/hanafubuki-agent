use std::sync::Mutex;

use kardia_core::{Exporter, FiberId, Level, Message};

use super::{LogExporter, GUEST_LOGGER_PREFIX};

static RECORDS: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        RECORDS
            .lock()
            .unwrap()
            .push((record.level(), record.args().to_string()));
    }

    fn flush(&self) {}
}

fn message(name: &str, level: Level, text: &str) -> Message {
    Message {
        sn: 1,
        ts: 0,
        name: name.to_string(),
        level,
        text: text.to_string(),
        fiber: FiberId::ROOT,
    }
}

fn levels_of(text: &str) -> Vec<log::Level> {
    RECORDS
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, record)| record.contains(text))
        .map(|(level, _)| *level)
        .collect()
}

#[test]
fn a_guests_messages_are_trace_and_bounded_the_others_keep_their_level() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Trace);
    let guest = format!("{GUEST_LOGGER_PREFIX}demo@local");
    let long = format!("guest-long {}", "x".repeat(200 * 1024));

    LogExporter.export(&message(&guest, Level::Error, "guest-error"));
    LogExporter.export(&message(&guest, Level::Debug, &long));
    LogExporter.export(&message("loader", Level::Warn, "loader-warning"));

    assert_eq!(levels_of("guest-error"), [log::Level::Trace]);
    assert_eq!(levels_of("guest-long"), [log::Level::Trace]);
    assert!(RECORDS
        .lock()
        .unwrap()
        .iter()
        .all(|(_, record)| record.len() < 80 * 1024));
    assert_eq!(levels_of("loader-warning"), [log::Level::Warn]);
}
