//! The guest host: the code of installed plugins, run as Kardia guests, out
//! of process or as WebAssembly modules (docs/plan-plugin-system.md, §4.2;
//! the site's architecture page, "Code plugins").
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
//! A package linked for development (`plugins link`) runs from its own
//! directory, with no content to keep: while the guest host runs, it
//! watches each linked directory, and once one has changed and been quiet
//! for 500 ms, it tells the facade, whose reload of the packages starts
//! that guest anew under the same grant and sandbox profile.
//!
//! A package may instead name a WebAssembly module, run in the
//! Agent's process by kardia-hmr's wasmtime backend within the fuel, time
//! and memory its manifest allows; it reaches nothing outside itself, so it
//! needs no process, sandbox or grant, and an instance that traps or
//! outruns a limit fails its guest, which restarts as a process guest that
//! exits does.
//!
//! What a guest contributes is what its manifest declares: once its fiber
//! is active, proxies registered as that fiber's effects (a tool source,
//! context sections, a prompt, hooks, an observer) call the service it
//! provides ([`GUEST_SERVICE`]), each within a deadline. It may view only
//! its workspace and its settings, and provide only that service.
//! [`check_package`] finds, without starting anything, what of a package's
//! code would not start or register (`plugins validate`).

mod check;
mod contribute;
mod guests;
mod host_services;
mod linked;
mod package;
mod plugin;
mod resolver;
mod settings;
mod spawn;
mod state;
mod supervise;
mod wasm;

pub use check::check_package;
pub use guests::{GuestHost, GuestHostInputs};
pub use linked::LinkedPackage;
pub use package::{GuestPackage, GUEST_SERVICE};
pub use plugin::GUESTS_PLUGIN;
pub use settings::GuestSettingsPort;
pub use spawn::WorkspaceRoots;
pub use state::{GuestExitCause, GuestExitReport, GuestState, GuestStatus};
