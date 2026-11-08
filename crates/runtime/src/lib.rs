//! The Agent's runtime: a Tokio runtime whose every thread is contained.
//!
//! The Agent's own copy of the mechanism the application's feature runtime
//! uses (`hanafubuki_application::contained_runtime`), kept separate so the
//! Agent depends on no application crate (docs/extraction-plan.md, §4).

mod runtime;

pub use runtime::{
    contained_threads, ContainedRuntime, RuntimeTaskFailure, AGENT_RUNTIME_THREAD_NAME,
};
