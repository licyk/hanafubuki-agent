//! The Agent's copy of `hanafubuki-module-contracts`: the module host and
//! the mechanisms its rows share, without the application's window bridge.
//!
//! A host mounts the rows of one manifest as kardia plugins, follows its
//! settings, stops a row whose code faults and shows it `Failed` until it is
//! retried, and bounds every wait on kardia ([`ModuleHost`]). Its rows
//! contribute to [`Registry`] services through registrations kardia reverts
//! when the registering plugin stops, report the panics a guard catches in
//! their code to [`PluginFaults`], and run long-lived work as scoped workers
//! ([`spawn_scoped_worker`]). The Agent's plugin host is one host; its
//! extension points are `hanafubuki-agent-contracts`. Nothing here depends
//! on the Agent's kernel or plugins. See
//! `site/content/docs/plugin-rules.en.mdx`.

mod error;
mod fault;
mod host;
mod logging;
mod registry;
mod restart;
mod status;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod worker;

pub use error::PluginError;
pub use fault::{FaultPoint, PluginFault, PluginFaults, PluginPanicked};
pub use host::{
    HostProfile, ModuleHost, ModuleHostOptions, ModuleRow, RootServices, ServiceContext,
    ServiceMount, ServiceStop, SettingsConfig, PLUGIN_START_TIMEOUT, PLUGIN_STOP_BOUND,
};
pub use logging::{LogExporter, GUEST_LOGGER_PREFIX};
pub use registry::{
    EntryLease, Register, RegistrationError, Registry, RegistryEntry, RegistryService,
    RegistrySnapshot, DRAIN_GRACE, RETIRE_CANCEL_BOUND,
};
pub use restart::{
    RestartPolicy, RestartWindow, AUTOMATIC_RESTART_DELAYS, AUTOMATIC_RESTART_WINDOW,
};
pub use status::{ModuleGroup, ModuleLock, ModuleRestart, ModuleState, ModuleStatus};
pub use worker::{spawn_scoped_worker, WORKER_STOP_BOUND};
