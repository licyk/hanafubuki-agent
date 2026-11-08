use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::atomic_write::AtomicPublishHook;
use crate::{WorkspaceFsPolicy, WorkspaceFsService};

pub fn service_with_publish_failures(
    policy: WorkspaceFsPolicy,
    failures: usize,
) -> WorkspaceFsService {
    WorkspaceFsService::with_publish_hook(
        policy,
        Arc::new(FailingPublishHook {
            remaining: AtomicUsize::new(failures),
        }),
    )
}

pub fn service_with_publish_cancellation(
    policy: WorkspaceFsPolicy,
    cancellation: CancellationToken,
) -> WorkspaceFsService {
    WorkspaceFsService::with_publish_hook(policy, Arc::new(CancellingPublishHook { cancellation }))
}

pub fn service_with_partial_write_failure(policy: WorkspaceFsPolicy) -> WorkspaceFsService {
    WorkspaceFsService::with_publish_hook(policy, Arc::new(PartialWriteFailureHook))
}

struct FailingPublishHook {
    remaining: AtomicUsize,
}

impl AtomicPublishHook for FailingPublishHook {
    fn before_publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), String> {
        if self
            .remaining
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            Err("injected failure before atomic publish".to_string())
        } else {
            Ok(())
        }
    }
}

struct CancellingPublishHook {
    cancellation: CancellationToken,
}

impl AtomicPublishHook for CancellingPublishHook {
    fn before_publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), String> {
        self.cancellation.cancel();
        Ok(())
    }
}

struct PartialWriteFailureHook;

impl AtomicPublishHook for PartialWriteFailureHook {
    fn after_write_chunk(&self, _temporary: &Path, _written_bytes: usize) -> Result<(), String> {
        Err("injected partial temporary write failure".to_string())
    }

    fn before_publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), String> {
        Ok(())
    }
}
