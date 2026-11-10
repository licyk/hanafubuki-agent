//! A call into a plugin made before a turn exists (a context or turn
//! admission contribution), bounded by the plugin call deadline and by the
//! admission's cancellation.

use std::future::Future;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::service::retry_wait::cancellable_delay;

/// How a bounded call ended.
pub(crate) enum Bounded<T> {
    Returned(T),
    /// It ran past the deadline and was cancelled.
    Overran,
    /// The admission was cancelled, and the call with it.
    Cancelled,
}

/// Runs `call`, which observes `call_token` (a child of `admission`), until
/// it returns, `deadline` passes or `admission` is cancelled. Unless the
/// call returned, its future is dropped and its token cancelled, also when
/// this future is itself dropped first, so a plugin's own work learns that
/// an abandoned admission no longer waits for it.
pub(crate) async fn bounded<T>(
    call: impl Future<Output = T>,
    call_token: CancellationToken,
    admission: &CancellationToken,
    deadline: Duration,
) -> Bounded<T> {
    let mut unless_returned = CancelUnlessReturned(Some(call_token));
    tokio::select! {
        value = call => {
            unless_returned.0 = None;
            Bounded::Returned(value)
        }
        cancelled = cancellable_delay(admission, deadline) => {
            if cancelled {
                Bounded::Cancelled
            } else {
                Bounded::Overran
            }
        }
    }
}

/// The issue of a contributor whose call overran.
pub(crate) fn overran(deadline: Duration) -> String {
    format!(
        "it did not answer within {} s and was cancelled",
        deadline.as_secs_f64()
    )
}

struct CancelUnlessReturned(Option<CancellationToken>);

impl Drop for CancelUnlessReturned {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.cancel();
        }
    }
}
