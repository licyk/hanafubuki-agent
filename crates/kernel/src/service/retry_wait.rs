use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

/// Upper bound on how long a cancelled retry wait keeps sleeping.
const CANCELLATION_POLL: Duration = Duration::from_millis(25);

/// Wait out a retry delay on the async runtime, ending early once
/// `cancellation` fires. Returns whether it was cancelled.
///
/// The token is polled rather than parked on, so the wait holds no blocking
/// thread for its whole duration.
pub(crate) async fn cancellable_delay(cancellation: &CancellationToken, delay: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + delay;
    loop {
        if cancellation.is_cancelled() {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        tokio::time::sleep((deadline - now).min(CANCELLATION_POLL)).await;
    }
}
