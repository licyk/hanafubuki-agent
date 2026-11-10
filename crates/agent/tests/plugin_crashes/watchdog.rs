//! Each check runs under a watchdog: one still running after
//! [`CHECK_DEADLINE`] ends the process with its name and the steps it
//! reached, instead of holding CI until the job is cancelled. The watchdog
//! is a plain thread, since what hangs may be the async runtime itself.

use std::fmt::Write as _;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// How long a check may run. The longest, the restart loop, waits out the
/// restart policy's delays (2 s, 10 s, 30 s); a check's own waits fail
/// after the fixture's 60 s.
const CHECK_DEADLINE: Duration = Duration::from_secs(150);

/// How many of the latest steps a report shows.
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
            for (at, what) in &steps[steps.len().saturating_sub(REPORTED)..] {
                let at = at.saturating_duration_since(started).as_secs_f64();
                let _ = writeln!(report, "  {at:>8.3}s {what}");
            }
        }
        None => report += "  (their lock is held)\n",
    }
    eprintln!("{report}");
}
