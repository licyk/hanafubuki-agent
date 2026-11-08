use crate::ApprovalClock;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct ManualApprovalClock {
    now_millis: AtomicU64,
}

impl ManualApprovalClock {
    pub fn new(now_millis: u64) -> Self {
        Self {
            now_millis: AtomicU64::new(now_millis),
        }
    }

    pub fn set(&self, now_millis: u64) {
        self.now_millis.store(now_millis, Ordering::Release);
    }

    pub fn advance(&self, millis: u64) {
        self.now_millis.fetch_add(millis, Ordering::AcqRel);
    }
}

impl ApprovalClock for ManualApprovalClock {
    fn now_millis(&self) -> u128 {
        self.now_millis.load(Ordering::Acquire).into()
    }
}
