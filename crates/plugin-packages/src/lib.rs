//! The guest host: the code of installed plugins, run out of process as
//! Kardia guests (docs/plan-plugin-system.md, §4.2; the site's
//! architecture page, "Code plugins").
//!
//! A package in this project's format may declare a `runtime`, a program
//! speaking Kardia's guest protocol (`kardia-guest/2`). The facade's
//! `plugins` row mounts [`GuestHost::plugin`], which runs a kardia-loader
//! subtree with one entry per such package: an enabled package whose
//! permissions the user granted gets a guest fiber, whose `apply` starts its
//! process and completes the handshake. The loader resolves an entry to a
//! kardia-hmr process guest whose metadata comes from the manifest, and
//! writes its own changes to the settings through [`GuestSettingsPort`].
//!
//! Each process is the Agent's: reserved and started through the
//! instance's `ExecutorManager`, in its owner group, ended only with its
//! receipt and released once its end is confirmed. It is confined by the
//! sandbox's plugin profile (the package read-only, its data directory
//! writable, the workspace and the network as granted), starts with a
//! cleared environment, and only while its package has the content recorded
//! at install. A guest that fails while it runs (its process exits, a call
//! outlives its deadline, it breaks the protocol) restarts under the
//! restart policy; one that fails more often stays failed until it is
//! retried. What a guest writes is logged only at `trace`.
//!
//! What a guest contributes is what its manifest declares: once its fiber
//! is active, proxies registered as that fiber's effects (a tool source,
//! context sections, a prompt, hooks, an observer) call the service it
//! provides ([`GUEST_SERVICE`]), each within a deadline. It may view only
//! its workspace and its settings, and provide only that service.

mod contribute;
mod guests;
mod host_services;
mod package;
mod plugin;
mod resolver;
mod settings;
mod spawn;
mod state;
mod supervise;

pub use guests::{GuestHost, GuestHostInputs};
pub use package::{GuestPackage, GUEST_SERVICE};
pub use plugin::GUESTS_PLUGIN;
pub use settings::GuestSettingsPort;
pub use spawn::WorkspaceRoots;
pub use state::{GuestExitCause, GuestExitReport, GuestState, GuestStatus};
