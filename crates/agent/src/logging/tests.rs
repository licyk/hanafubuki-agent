use std::fmt;
use std::sync::atomic::AtomicBool;

use super::*;

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn record<'a>(level: Level, target: &'a str, args: fmt::Arguments<'a>) -> Record<'a> {
    Record::builder()
        .level(level)
        .target(target)
        .args(args)
        .build()
}

#[test]
fn a_full_file_is_rotated_and_the_oldest_beyond_the_kept_ones_removed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("logs").join(LOG_FILE_NAME);
    let mut file = LogFile::open(path.clone(), 10, 2).unwrap();
    for line in ["one\n", "two\n", "three\n", "four\n", "five\n"] {
        file.write(line.as_bytes());
    }
    drop(file);
    let rotated = |index: usize| path.with_file_name(format!("{LOG_FILE_NAME}.{index}"));
    assert_eq!(read(&path), "four\nfive\n");
    assert_eq!(read(&rotated(1)), "three\n");
    assert_eq!(read(&rotated(2)), "one\ntwo\n");
    assert!(!rotated(3).exists());

    // A reopened file counts what it already holds.
    let mut file = LogFile::open(path.clone(), 10, 2).unwrap();
    file.write(b"six\n");
    assert_eq!(read(&path), "six\n");
    assert_eq!(read(&rotated(1)), "four\nfive\n");
    assert_eq!(read(&rotated(2)), "three\n");
}

#[test]
fn the_level_holds_and_other_crates_stay_at_info() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(LOG_FILE_NAME);
    let logger = FileLogger::new(
        LogFile::open(path.clone(), MAX_LOG_FILE_BYTES, 1).unwrap(),
        LevelFilter::Debug,
    );
    logger.log(&record(
        Level::Debug,
        "hanafubuki_agent_kernel",
        format_args!("kept"),
    ));
    logger.log(&record(
        Level::Trace,
        "hanafubuki_agent_kernel",
        format_args!("too low"),
    ));
    logger.log(&record(
        Level::Debug,
        "hyper::proto",
        format_args!("another crate"),
    ));
    logger.log(&record(
        Level::Info,
        "hyper::proto",
        format_args!("its info"),
    ));
    logger.set_level(LevelFilter::Warn);
    logger.log(&record(
        Level::Info,
        "hanafubuki_agent",
        format_args!("now too low"),
    ));
    logger.log(&record(
        Level::Warn,
        "hanafubuki_agent",
        format_args!("a warning"),
    ));
    logger.flush();

    let text = read(&path);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(
        lines[0].ends_with(" DEBUG hanafubuki_agent_kernel: kept"),
        "{text}"
    );
    assert!(
        lines[1].ends_with(" INFO  hyper::proto: its info"),
        "{text}"
    );
    assert!(
        lines[2].ends_with(" WARN  hanafubuki_agent: a warning"),
        "{text}"
    );
}

static BLOCKED_WHILE_FORMATTING: AtomicBool = AtomicBool::new(false);

struct Probe;

impl fmt::Display for Probe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        BLOCKED_WHILE_FORMATTING.store(logging_blocked(), Ordering::SeqCst);
        formatter.write_str("probe")
    }
}

#[test]
fn logging_is_blocked_while_this_thread_writes_a_record() {
    let directory = tempfile::tempdir().unwrap();
    let logger = FileLogger::new(
        LogFile::open(directory.path().join(LOG_FILE_NAME), MAX_LOG_FILE_BYTES, 1).unwrap(),
        LevelFilter::Info,
    );
    assert!(!logging_blocked());
    logger.log(&record(
        Level::Info,
        "hanafubuki_agent",
        format_args!("{Probe}"),
    ));
    assert!(BLOCKED_WHILE_FORMATTING.load(Ordering::SeqCst));
    assert!(!logging_blocked());
}
