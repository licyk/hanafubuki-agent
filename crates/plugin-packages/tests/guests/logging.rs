//! What a guest writes (its stderr, its log lines) reaches the Agent's log
//! only at `trace` (AGENTS.md, "Agent Content Logging").

use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;

use crate::fixture::{eventually, running, Fixture, Options};

/// Every record: its level, target and message.
static RECORDS: Mutex<Vec<(log::Level, String, String)>> = Mutex::new(Vec::new());

/// A guest's stderr line that makes the logger panic on kardia-hmr's
/// `guest-stderr` thread, which forwards it (`contained`).
pub const PANIC_ON_STDERR: &str = "PANIC-ON-THE-STDERR-THREAD";

/// The targets of the WASM compiler's many records, which reports skip.
const COMPILER_TARGETS: &[&str] = &["cranelift", "regalloc2", "wasmtime_cranelift"];

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
        // Formatted before taking the lock, which formatting could need.
        let message = record.args().to_string();
        if message.contains(PANIC_ON_STDERR)
            && std::thread::current().name() == Some("guest-stderr")
        {
            panic!("the test logger panics");
        }
        RECORDS
            .lock()
            .push((record.level(), record.target().to_string(), message));
    }

    fn flush(&self) {}
}

/// Records every log line of this binary, at every level.
pub fn capture() {
    log::set_logger(&Capture).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
}

/// The latest `count` records but the compiler's, unless a stuck thread
/// holds their lock.
pub fn latest(count: usize) -> Option<Vec<String>> {
    let records = RECORDS.try_lock_for(Duration::from_secs(1))?;
    let mut latest: Vec<String> = records
        .iter()
        .rev()
        .filter(|(_, target, _)| {
            !COMPILER_TARGETS
                .iter()
                .any(|compiler| target.starts_with(compiler))
        })
        .take(count)
        .map(|(level, target, message)| format!("{level} {target}: {message}"))
        .collect();
    latest.reverse();
    Some(latest)
}

fn levels_of(text: &str) -> Vec<log::Level> {
    RECORDS
        .lock()
        .iter()
        .filter(|(_, _, message)| message.contains(text))
        .map(|(level, _, _)| *level)
        .collect()
}

pub async fn a_guests_own_words_are_logged_only_at_trace() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    fixture.apply(vec![package.clone()]).await;
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
