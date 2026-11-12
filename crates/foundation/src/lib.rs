//! The Agent's copy of `hanafubuki-foundation`: the domain-neutral
//! primitives its crates share, trimmed to what they use. A fix to a module
//! with a twin in the application's foundation is ported by hand.
//!
//! This crate deliberately contains no Tauri types, domain phase enums,
//! registries, filesystem repositories, or application orchestration.

pub mod agent_settings;
pub mod background_worker;
pub mod blocking;
pub mod cancellation;
pub mod environment;
pub mod json_normalization;
pub mod module_faults;
pub mod panic;
pub mod paths;
pub mod persistence;
pub mod secrets;
pub mod settings;
pub mod terminal_text;
pub mod trace_content;
pub mod url_policy;
pub mod user_agent;
