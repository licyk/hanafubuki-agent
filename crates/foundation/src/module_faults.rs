//! The application's module fault sink: one process-wide record of the
//! faults that ended a piece of work while the application went on.
//!
//! A contained worker group's panic handler ([`module_fault_handler`]) and a
//! contained thread ([`crate::background_worker::spawn_contained_thread`])
//! report here when their body panics. Each report is logged at error and
//! kept, with its owner, component, message and time, in a bounded list the
//! module host and diagnostics read ([`recorded_module_faults`]). The sink
//! only records and hands each fault on to its subscribers
//! ([`subscribe_module_faults`]): what to do about a fault (restart,
//! disable, a notice) is their decision. A module host subscribes for the
//! modules it runs; a fault of a name no host runs stays recorded only.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::background_worker::WorkerPanicHandler;

/// How many recent faults the sink keeps.
pub const MAX_RECORDED_MODULE_FAULTS: usize = 32;

/// What a fault belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultOwner {
    /// The application's core, which is not a module (process execution,
    /// the instance lifecycle coordinator, the watchdog, start-up): its
    /// fault is an application-level notice.
    Core,
    /// A module, by name.
    Module(&'static str),
}

impl std::fmt::Display for FaultOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core => formatter.write_str("core"),
            Self::Module(name) => formatter.write_str(name),
        }
    }
}

/// One fault the sink recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleFault {
    pub owner: FaultOwner,
    /// The worker or thread that failed, by name.
    pub component: String,
    pub message: String,
    pub at_unix_millis: u64,
}

/// Every fault counted, and the most recent ones, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleFaultSummary {
    pub count: u64,
    pub recent: Vec<ModuleFault>,
}

struct FaultLogState {
    count: u64,
    recent: VecDeque<ModuleFault>,
}

/// A bounded record of faults: a count of all of them and the last
/// `capacity`.
struct FaultLog {
    capacity: usize,
    state: Mutex<FaultLogState>,
}

impl FaultLog {
    const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: parking_lot::const_mutex(FaultLogState {
                count: 0,
                recent: VecDeque::new(),
            }),
        }
    }

    fn record(&self, fault: ModuleFault) {
        let mut state = self.state.lock();
        state.count = state.count.saturating_add(1);
        while state.recent.len() >= self.capacity.max(1) {
            state.recent.pop_front();
        }
        state.recent.push_back(fault);
    }

    fn summary(&self) -> ModuleFaultSummary {
        let state = self.state.lock();
        ModuleFaultSummary {
            count: state.count,
            recent: state.recent.iter().cloned().collect(),
        }
    }
}

static SINK: FaultLog = FaultLog::new(MAX_RECORDED_MODULE_FAULTS);

type FaultSubscriber = Arc<dyn Fn(&ModuleFault) + Send + Sync>;

static SUBSCRIBERS: Mutex<Vec<(u64, FaultSubscriber)>> = parking_lot::const_mutex(Vec::new());
static NEXT_SUBSCRIBER: AtomicU64 = AtomicU64::new(0);

/// Reports that `component` of `owner` failed with `message`: logs it at
/// error, records it, and hands it to every subscriber.
pub fn report_module_fault(owner: FaultOwner, component: &str, message: &str) {
    log::error!("module fault owner={owner} component={component}: {message}");
    let fault = ModuleFault {
        owner,
        component: component.to_string(),
        message: message.to_string(),
        at_unix_millis: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0),
    };
    SINK.record(fault.clone());
    let subscribers = SUBSCRIBERS
        .lock()
        .iter()
        .map(|(_, subscriber)| subscriber.clone())
        .collect::<Vec<_>>();
    for subscriber in subscribers {
        subscriber(&fault);
    }
}

/// Hands every fault reported from now on to `subscriber`, after the sink
/// recorded it, until the returned subscription is dropped. It runs on the
/// thread that reported the fault (a panicking worker's, a command's), with
/// no lock of the sink held, so it only hands the fault on, never waits.
#[must_use = "dropping the subscription ends it"]
pub fn subscribe_module_faults(
    subscriber: impl Fn(&ModuleFault) + Send + Sync + 'static,
) -> ModuleFaultSubscription {
    let id = NEXT_SUBSCRIBER.fetch_add(1, Ordering::Relaxed);
    SUBSCRIBERS.lock().push((id, Arc::new(subscriber)));
    ModuleFaultSubscription(id)
}

/// A subscriber of the sink ([`subscribe_module_faults`]); dropping it
/// unsubscribes.
pub struct ModuleFaultSubscription(u64);

impl Drop for ModuleFaultSubscription {
    fn drop(&mut self) {
        SUBSCRIBERS.lock().retain(|(id, _)| *id != self.0);
    }
}

/// The faults reported in this process.
pub fn recorded_module_faults() -> ModuleFaultSummary {
    SINK.summary()
}

/// The panic handler of a contained worker group of `owner`
/// (`BackgroundWorkerGroup::contained_with_panic_handler`): a member's
/// panic is reported as a fault of `owner`, the member named as its
/// component.
pub fn module_fault_handler(owner: FaultOwner) -> WorkerPanicHandler {
    Arc::new(move |worker, message| report_module_fault(owner, worker, message))
}

#[cfg(test)]
mod tests;
