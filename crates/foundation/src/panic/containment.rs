//! Whether a panic on the current thread is known to be caught.
//!
//! Two thread-local facts answer it: the thread is marked contained for its
//! whole life (a runtime's `on_thread_start`, a contained worker's body), or
//! a guard ([`super::catch_panic`], each poll of
//! [`super::catch_panic_async`]) is active on it now.

use std::cell::Cell;

thread_local! {
    static CONTAINED_THREAD: Cell<bool> = const { Cell::new(false) };
    static GUARD_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Marks the current thread as one whose panics are caught where they are
/// raised: a task's join handle, a worker group's body, a guard. Call it
/// first thing on a thread whose code survives a panic; the mark lasts until
/// [`unmark_thread_contained`] or the thread's end.
///
/// A thread this one spawns is not marked: it decides for itself.
pub fn mark_thread_contained() {
    let _ = CONTAINED_THREAD.try_with(|contained| contained.set(true));
}

/// Clears [`mark_thread_contained`], for a pooled thread leaving its runtime.
pub fn unmark_thread_contained() {
    let _ = CONTAINED_THREAD.try_with(|contained| contained.set(false));
}

/// Whether a panic raised now on this thread would be caught: the thread is
/// marked contained or a guard is active.
pub fn is_panic_contained() -> bool {
    CONTAINED_THREAD.try_with(Cell::get).unwrap_or(false)
        || GUARD_DEPTH
            .try_with(|depth| depth.get() > 0)
            .unwrap_or(false)
}

/// One active guard on this thread, for as long as it lives.
pub(super) struct GuardScope(());

impl GuardScope {
    pub(super) fn enter() -> Self {
        let _ = GUARD_DEPTH.try_with(|depth| depth.set(depth.get() + 1));
        Self(())
    }
}

impl Drop for GuardScope {
    fn drop(&mut self) {
        let _ = GUARD_DEPTH.try_with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}
