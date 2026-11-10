//! What a guest writes (its stderr, its log lines) reaches the Agent's log
//! only at `trace` (AGENTS.md, "Agent Content Logging").

use parking_lot::Mutex;
use serde_json::Value;

use crate::fixture::{eventually, running, Fixture, Options};

static RECORDS: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        if std::env::var_os("HANAFUBUKI_TEST_LOG").is_some() && record.level() <= log::Level::Debug
        {
            eprintln!("{} {}", record.level(), record.args());
        }
        RECORDS
            .lock()
            .push((record.level(), record.args().to_string()));
    }

    fn flush(&self) {}
}

/// Records every log line of this binary, at every level.
pub fn capture() {
    log::set_logger(&Capture).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
}

fn levels_of(text: &str) -> Vec<log::Level> {
    RECORDS
        .lock()
        .iter()
        .filter(|(_, message)| message.contains(text))
        .map(|(level, _)| *level)
        .collect()
}

pub async fn a_guests_own_words_are_logged_only_at_trace() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    fixture.host.apply(vec![package.clone()]).await;
    fixture.wait(&package.id.key(), "start", running).await;

    fixture.call(&package, "speak", Value::Null).await.unwrap();

    for text in ["SECRET-STDERR", "SECRET-LOG"] {
        eventually(&format!("a record of {text}"), || {
            !levels_of(text).is_empty()
        })
        .await;
        assert!(
            levels_of(text)
                .iter()
                .all(|level| *level == log::Level::Trace),
            "{text}: {:?}",
            levels_of(text)
        );
    }
}
