//! The process's record of the panics it saw, for diagnostics.
//!
//! The panic hook records every panic, contained or fatal, before it decides
//! whether to abort. A fatal one rarely outlives its stderr line, but a
//! contained one stays here for the diagnostic bundle.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

/// How many recent panics the process-wide record keeps.
pub const MAX_RECORDED_PANICS: usize = 16;

/// One panic the hook saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanicFault {
    pub at_unix_millis: u64,
    pub thread: String,
    pub location: String,
    /// Bounded by [`super::MAX_PANIC_MESSAGE_BYTES`].
    pub message: String,
    /// Whether it unwound to a catch instead of aborting the process.
    pub contained: bool,
}

impl PanicFault {
    pub fn now(thread: String, location: String, message: String, contained: bool) -> Self {
        Self {
            at_unix_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0),
            thread,
            location,
            message,
            contained,
        }
    }
}

/// Every panic counted, and the most recent ones, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanicFaultSummary {
    pub count: u64,
    pub recent: Vec<PanicFault>,
}

struct FaultLogState {
    count: u64,
    recent: VecDeque<PanicFault>,
}

/// A bounded record of panics: a count of all of them and the last
/// `capacity`.
pub struct PanicFaultLog {
    capacity: usize,
    state: Mutex<FaultLogState>,
}

impl PanicFaultLog {
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: parking_lot::const_mutex(FaultLogState {
                count: 0,
                recent: VecDeque::new(),
            }),
        }
    }

    pub fn record(&self, fault: PanicFault) {
        let mut state = self.state.lock();
        state.count = state.count.saturating_add(1);
        if self.capacity == 0 {
            return;
        }
        while state.recent.len() >= self.capacity {
            state.recent.pop_front();
        }
        state.recent.push_back(fault);
    }

    pub fn summary(&self) -> PanicFaultSummary {
        let state = self.state.lock();
        PanicFaultSummary {
            count: state.count,
            recent: state.recent.iter().cloned().collect(),
        }
    }
}

static PROCESS_PANICS: PanicFaultLog = PanicFaultLog::new(MAX_RECORDED_PANICS);

/// The panics the installed hook has recorded in this process.
pub fn recorded_panics() -> PanicFaultSummary {
    PROCESS_PANICS.summary()
}

pub(super) fn record_panic(fault: PanicFault) {
    PROCESS_PANICS.record(fault);
}
