//! Turning a panic into a value at an isolation boundary, and deciding which
//! panics may unwind at all.
//!
//! A component that runs code it does not own (a plugin's extension point, a
//! worker's body, a turn's task) catches a panic there, so the failure stays
//! in that component instead of unwinding into the caller. The payload
//! becomes a bounded message for the log and for diagnostics.
//!
//! Unwinding is contained: the application's panic hook
//! ([`install_panic_hook`]) aborts the process on a panic unless it is known
//! to be caught, because code outside the Agent was written for a process
//! that aborts on panic (std locks that would poison, promises that would
//! never settle). A panic is known to be caught on a thread marked contained
//! ([`mark_thread_contained`]: the contained runtimes' threads, contained
//! worker groups' members and contained threads) and inside [`catch_panic`]
//! or [`catch_panic_async`].

use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::task::Poll;

mod containment;
mod crash_record;
mod faults;
mod hook;

use containment::GuardScope;
pub use containment::{is_panic_contained, mark_thread_contained, unmark_thread_contained};
pub use crash_record::{configure_crash_record, read_crash_record, CrashRecord};
pub use faults::{
    recorded_panics, PanicFault, PanicFaultLog, PanicFaultSummary, MAX_RECORDED_PANICS,
};
pub use hook::install_panic_hook;

/// The longest panic message kept, in bytes.
pub const MAX_PANIC_MESSAGE_BYTES: usize = 512;

/// The message of a panic payload: its `&str` or `String`, otherwise
/// `"panicked"`, cut to [`MAX_PANIC_MESSAGE_BYTES`] on a character boundary.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("panicked");
    let mut end = message.len().min(MAX_PANIC_MESSAGE_BYTES);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_string()
}

/// Runs `call`, returning its panic's message instead of unwinding. A panic
/// inside it is contained: the panic hook lets it unwind to here.
///
/// The caller asserts unwind safety: what `call` shares with it must stay
/// consistent at any panic point (non-poisoning locks are released, and
/// nothing half-applied is read afterwards).
pub fn catch_panic<R>(call: impl FnOnce() -> R) -> Result<R, String> {
    let _guard = GuardScope::enter();
    catch_unwind(AssertUnwindSafe(call)).map_err(|payload| panic_message(payload.as_ref()))
}

/// Awaits `future`, returning the message of a panic raised while it is
/// polled instead of unwinding. Each poll is a guard scope, so the panic
/// hook lets such a panic unwind to here. The future is not polled again
/// after it panicked.
pub async fn catch_panic_async<F: Future>(future: F) -> Result<F::Output, String> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(move |context| {
        let _guard = GuardScope::enter();
        match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(payload) => Poll::Ready(Err(panic_message(payload.as_ref()))),
        }
    })
    .await
}

#[cfg(test)]
mod tests;
