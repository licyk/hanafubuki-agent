//! Plugin faults: a panic in a hosted plugin's code stays in that plugin.
//!
//! Code that calls into a plugin (the Agent kernel at its extension points)
//! does so through a guard ([`PluginFaults::guard`],
//! [`PluginFaults::guard_async`], or the same methods on a
//! [`crate::RegistryEntry`], which knows its owner). A panic there becomes
//! that call's error, is logged, and is reported as a [`PluginFault`] of the
//! owning plugin. The [`crate::ModuleHost`] that runs the plugin listens
//! ([`PluginFaults::set_handler`]) and stops it, and the row then shows
//! `Failed` with the fault until it is retried.
//!
//! One `PluginFaults` is shared by a host's registries (each entry holds it)
//! and published as a kardia service, so a plugin's scoped workers report
//! through it too (`spawn_scoped_worker`).
//!
//! It also keeps each plugin's current *issues*: what its caller left out
//! because that plugin's code failed without panicking (an Agent context or
//! turn admission contributor's error). An issue does not stop the plugin;
//! the host shows it in the plugin's status (degraded) until the same
//! source of that plugin succeeds again. The host hears of every change to
//! the issues ([`PluginFaults::set_issues_listener`]) so the status it shows
//! does not wait for the next read.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;

use hanafubuki_agent_foundation::panic::{catch_panic, catch_panic_async};
use kardia_macros::Service;
use parking_lot::Mutex;

/// Where in a plugin's code a fault happened: a kind of call (`tool invoke`)
/// or a worker by label (`worker agent-titles`). A host's own vocabulary
/// converts into it (the Agent's `ExtensionPoint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultPoint {
    kind: &'static str,
    label: Option<&'static str>,
}

impl FaultPoint {
    pub const fn new(kind: &'static str) -> Self {
        Self { kind, label: None }
    }

    /// A worker the plugin spawned with `spawn_scoped_worker`, by label.
    pub const fn worker(label: &'static str) -> Self {
        Self {
            kind: "worker",
            label: Some(label),
        }
    }
}

impl fmt::Display for FaultPoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.label {
            Some(label) => write!(formatter, "{} {label}", self.kind),
            None => formatter.write_str(self.kind),
        }
    }
}

/// A plugin panicked in `point`. `message` is the bounded panic message
/// (`hanafubuki_agent_foundation::panic::panic_message`); it reaches the log and the
/// plugin's status, never the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginFault {
    pub plugin: String,
    pub point: FaultPoint,
    pub message: String,
}

impl fmt::Display for PluginFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "panicked in {}: {}", self.point, self.message)
    }
}

/// A guarded call panicked; the fault is already reported. The caller takes
/// the call's own error path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginPanicked;

type FaultHandler = Arc<dyn Fn(PluginFault) + Send + Sync>;
type IssuesListener = Arc<dyn Fn() + Send + Sync>;

/// Where plugin faults are reported.
#[derive(Service, Default)]
#[service(name = "plugin-faults")]
pub struct PluginFaults {
    handler: Mutex<Option<FaultHandler>>,
    /// The current issue of each (plugin, source).
    issues: Mutex<BTreeMap<(String, String), String>>,
    issues_listener: Mutex<Option<IssuesListener>>,
}

impl PluginFaults {
    /// Sends every later fault to `handler` as well as to the log. It runs
    /// on the thread that caught the panic (an Agent turn's task, a worker),
    /// so it only hands the fault on, never waits.
    pub fn set_handler(&self, handler: impl Fn(PluginFault) + Send + Sync + 'static) {
        *self.handler.lock() = Some(Arc::new(handler));
    }

    /// Calls `listener` after every change to the issues: one that is new,
    /// changed or gone. Setting an issue a plugin already has, or clearing
    /// one it does not have, changes nothing and calls nothing, so a
    /// contributor that fails the same way every turn (or succeeds every
    /// turn) is not announced again. It runs on the thread that recorded the
    /// change, with no lock of this type held, so it must not block.
    pub fn set_issues_listener(&self, listener: impl Fn() + Send + Sync + 'static) {
        *self.issues_listener.lock() = Some(Arc::new(listener));
    }

    /// Stops forwarding faults and issue changes; faults are only logged.
    pub fn clear_handler(&self) {
        self.handler.lock().take();
        self.issues_listener.lock().take();
    }

    /// Logs `fault` and hands it to the handler, if one is set.
    pub fn report(&self, fault: PluginFault) {
        log::error!("plugin {} {fault}", fault.plugin);
        let handler = self.handler.lock().clone();
        if let Some(handler) = handler {
            handler(fault);
        }
    }

    /// Records that `plugin`'s code at `source` (an extension point and the
    /// id of the entry there, such as `context contributor environment`)
    /// failed and its caller went on without it (`issue`), or that it works
    /// again (`None`). Logged at warn when an issue is new or changed.
    pub fn set_issue(&self, plugin: &str, source: &str, issue: Option<String>) {
        let key = (plugin.to_string(), source.to_string());
        let changed = {
            let mut issues = self.issues.lock();
            match issue {
                Some(issue) => {
                    let changed = issues.get(&key) != Some(&issue);
                    if changed {
                        log::warn!("plugin {plugin} {source} left out: {issue}");
                        issues.insert(key, issue);
                    }
                    changed
                }
                None => issues.remove(&key).is_some(),
            }
        };
        if changed {
            self.issues_changed();
        }
    }

    /// Forgets every issue of `plugin`: a new instance of it starts clean.
    pub fn clear_issues(&self, plugin: &str) {
        let changed = {
            let mut issues = self.issues.lock();
            let before = issues.len();
            issues.retain(|(owner, _), _| owner != plugin);
            issues.len() != before
        };
        if changed {
            self.issues_changed();
        }
    }

    fn issues_changed(&self) {
        let listener = self.issues_listener.lock().clone();
        if let Some(listener) = listener {
            listener();
        }
    }

    /// The current issues of `plugin`, one line each (`source: issue`), by
    /// source.
    pub fn issues(&self, plugin: &str) -> Vec<String> {
        self.all_issues().remove(plugin).unwrap_or_default()
    }

    /// [`Self::issues`] of every plugin that has one.
    pub fn all_issues(&self) -> BTreeMap<String, Vec<String>> {
        let mut by_plugin = BTreeMap::<String, Vec<String>>::new();
        for ((plugin, source), issue) in self.issues.lock().iter() {
            by_plugin
                .entry(plugin.clone())
                .or_default()
                .push(format!("{source}: {issue}"));
        }
        by_plugin
    }

    /// Runs plugin code of `plugin`; a panic becomes [`PluginPanicked`] and a
    /// reported fault.
    pub fn guard<R>(
        &self,
        plugin: &str,
        point: impl Into<FaultPoint>,
        call: impl FnOnce() -> R,
    ) -> Result<R, PluginPanicked> {
        let point = point.into();
        catch_panic(call).map_err(|message| self.fault(plugin, point, message))
    }

    /// [`Self::guard`] for an async call: `call` builds the future, and a
    /// panic while building or polling it is caught.
    pub async fn guard_async<F, Fut>(
        &self,
        plugin: &str,
        point: impl Into<FaultPoint>,
        call: F,
    ) -> Result<Fut::Output, PluginPanicked>
    where
        F: FnOnce() -> Fut,
        Fut: Future,
    {
        let point = point.into();
        catch_panic_async(async move { call().await })
            .await
            .map_err(|message| self.fault(plugin, point, message))
    }

    fn fault(&self, plugin: &str, point: FaultPoint, message: String) -> PluginPanicked {
        self.report(PluginFault {
            plugin: plugin.to_string(),
            point,
            message,
        });
        PluginPanicked
    }
}
