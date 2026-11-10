//! Each check runs under a watchdog: one still running after
//! [`CHECK_DEADLINE`] ends the process with its name, the steps it reached,
//! the latest log records and, on macOS, every thread's stack, instead of
//! holding CI until the job is cancelled. The watchdog is a plain thread,
//! since what hangs may be the async runtime itself.

use std::fmt::Write as _;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::logging;

/// How long a check may run. The whole suite takes about 35 s on Linux,
/// and a check's own waits fail after the fixture's 60 s deadline.
const CHECK_DEADLINE: Duration = Duration::from_secs(120);

/// How many of the latest steps and log records a report shows.
const REPORTED: usize = 60;

static STEPS: Mutex<Vec<(Instant, String)>> = Mutex::new(Vec::new());

/// Records that the running check reached `what`.
pub fn step(what: impl Into<String>) {
    let what = what.into();
    STEPS.lock().push((Instant::now(), what));
}

/// Runs the check `name`, ending the process if it outlasts
/// [`CHECK_DEADLINE`].
pub fn guard(name: &str, check: impl FnOnce()) {
    STEPS.lock().clear();
    let started = Instant::now();
    // Dropped when the check ends, or unwinds from a failed assertion.
    let (finished, done) = mpsc::channel::<()>();
    let name = name.to_string();
    let watchdog = std::thread::spawn(move || {
        if let Err(RecvTimeoutError::Timeout) = done.recv_timeout(CHECK_DEADLINE) {
            report(&name, started);
            std::process::exit(1);
        }
    });
    check();
    drop(finished);
    watchdog.join().unwrap();
}

fn report(name: &str, started: Instant) {
    let mut report = format!("{name}: still running after {CHECK_DEADLINE:?}\nits steps:\n");
    // A lock that a stuck thread holds must not stop the report.
    match STEPS.try_lock_for(Duration::from_secs(1)) {
        Some(steps) => {
            for (at, what) in latest(&steps) {
                let at = at.saturating_duration_since(started).as_secs_f64();
                let _ = writeln!(report, "  {at:>8.3}s {what}");
            }
        }
        None => report += "  (their lock is held)\n",
    }
    report += "its latest log records, without the compiler's:\n";
    match logging::latest(REPORTED) {
        Some(records) => {
            for record in records {
                let _ = writeln!(report, "  {record}");
            }
        }
        None => report += "  (their lock is held)\n",
    }
    eprintln!("{report}");
    sample_threads();
}

fn latest<T>(items: &[T]) -> &[T] {
    &items[items.len().saturating_sub(REPORTED)..]
}

/// Every thread's stack, by macOS's `sample`, where the check has hung
/// while Linux and Windows pass: as the runner's user, else by `sudo -n`
/// (attaching to a process can need more).
#[cfg(target_os = "macos")]
fn sample_threads() {
    let pid = std::process::id().to_string();
    let file = std::env::temp_dir().join(format!("guests-{pid}.sample.txt"));
    for command in [&["sample"][..], &["sudo", "-n", "sample"]] {
        // `sample <pid> <seconds> [options]`, as its usage gives it.
        let sampled = std::process::Command::new(command[0])
            .args(&command[1..])
            .args([pid.as_str(), "1", "-mayDie", "-file"])
            .arg(&file)
            .status();
        match sampled {
            Ok(status) if status.success() => {
                match std::fs::read_to_string(&file) {
                    Ok(sample) => eprintln!("the threads:\n{sample}"),
                    Err(error) => eprintln!("`sample` wrote no report: {error}"),
                }
                return;
            }
            Ok(status) => eprintln!("`{}` failed ({status})", command.join(" ")),
            Err(error) => eprintln!("`{}` did not run: {error}", command.join(" ")),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn sample_threads() {}
