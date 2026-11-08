//! The crash-loop marker does not count a start whose run ended in a fatal
//! panic: every fatal panic is on a thread that is not the Agent's.

use super::*;

fn document(unfinished_starts: u32, attempt: Option<(u32, u64)>) -> StartMarkerDocument {
    StartMarkerDocument {
        unfinished_starts,
        last_attempt: attempt.map(|(pid, at_unix_millis)| StartAttempt {
            pid,
            at_unix_millis,
        }),
    }
}

fn crash(pid: u32, at_unix_millis: u64) -> CrashRecord {
    CrashRecord {
        pid,
        at_unix_millis,
        thread: "tauri-main".to_string(),
        location: "src/somewhere.rs:1:1".to_string(),
        message: "not the Agent".to_string(),
    }
}

#[test]
fn a_run_that_ended_in_a_fatal_panic_does_not_count_its_start() {
    let marked = document(2, Some((41, 1_000)));
    assert_eq!(counted_starts(&marked, None), 2);
    // The run that recorded the attempt died in a fatal panic after it.
    assert_eq!(counted_starts(&marked, Some(&crash(41, 1_000))), 1);
    assert_eq!(counted_starts(&marked, Some(&crash(41, 5_000))), 1);
    // A crash of another run, or one before the attempt, is not this one's.
    assert_eq!(counted_starts(&marked, Some(&crash(42, 5_000))), 2);
    assert_eq!(counted_starts(&marked, Some(&crash(41, 999))), 2);
    // A marker without an attempt (cleared, or written before attempts were
    // recorded) pairs with nothing.
    assert_eq!(
        counted_starts(&document(3, None), Some(&crash(41, 5_000))),
        3
    );
    assert_eq!(
        counted_starts(&document(0, Some((41, 1_000))), Some(&crash(41, 5_000))),
        0
    );
}

#[test]
fn the_marker_reads_the_crash_record_it_was_given() {
    let directory = tempfile::tempdir().unwrap();
    let marker_file = directory.path().join("start-marker.json");
    let record = directory.path().join("crash-record.json");
    let marker = AgentStartMarker::new(marker_file.clone()).with_crash_record(record.clone());
    for _ in 0..CRASH_LOOP_STARTS {
        marker.record_attempt().unwrap();
    }
    assert_eq!(
        marker.disabled_reason(true),
        Some(AgentDisabledReason::CrashLoop)
    );

    // This run (the one that recorded the last attempt) ended in a fatal
    // panic the hook recorded.
    let at = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": std::process::id(),
            "atUnixMillis": at + 1,
            "thread": "main",
            "location": "src/lib.rs:1:1",
            "message": "a fatal panic outside the Agent",
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(marker.unfinished_starts(), CRASH_LOOP_STARTS - 1);
    assert_eq!(marker.disabled_reason(true), None);
    // Without the record, the same marker would keep the Agent off.
    assert_eq!(
        AgentStartMarker::new(marker_file).disabled_reason(true),
        Some(AgentDisabledReason::CrashLoop)
    );
}
