//! What a guest contributes to the Agent: proxies, built from what its
//! manifest declares (`contributes`), never from the guest's word, that call
//! its `plugin` service over the guest protocol.
//!
//! The proxies of one guest are registered when its fiber becomes active,
//! as effects of that fiber: they retire when it fails, stops or restarts,
//! and its next start registers them again. A declaration this host cannot
//! honour (a name or kind that is not valid, one another plugin holds, more
//! kinds than the bound) is left out with an issue of the package; the rest
//! is registered. Every call has a deadline and observes its caller's
//! cancellation; a guest that does not answer in time is killed and
//! restarts (the guest host's restart loop), and the call takes its error
//! path, which never fails a turn.
//!
//! The prompt contributor is the one exception: the turn's prepared values
//! are keyed by type, so one contributor of the guest host asks every guest
//! that declares a prompt ([`prompt`]).

mod check;
mod context;
mod hooks;
mod observers;
mod prompt;
mod protocol;
mod tools;

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_contracts::CancellationToken;
use hanafubuki_agent_foundation::trace_content::bounded_trace_content;
use hanafubuki_agent_packages::PluginId;
use kardia_core::{Context, Fiber};
use serde_json::Value;

use crate::guests::{GuestHost, Shared};
use crate::package::GuestPackage;
use crate::state::GuestState;

pub(crate) use check::check_contributes;
pub(crate) use prompt::GuestPrompts;
pub(crate) use protocol::GuestMethod;

/// How often a call checks its caller's cancellation.
const CANCELLATION_CHECK: Duration = Duration::from_millis(50);

/// One guest, as its proxies call it.
#[derive(Clone)]
pub(crate) struct GuestCaller {
    host: GuestHost,
    id: PluginId,
}

impl GuestCaller {
    pub(crate) fn new(host: GuestHost, id: PluginId) -> Self {
        Self { host, id }
    }

    pub(crate) fn id(&self) -> &PluginId {
        &self.id
    }

    /// Whether its guest runs: a proxy of a guest that does not answers at
    /// once, without a call.
    pub(crate) fn running(&self) -> bool {
        self.host
            .0
            .records
            .lock()
            .get(&self.id.key())
            .is_some_and(|record| record.state == GuestState::Running)
    }

    /// The guest host's call deadline: what a call the kernel itself does
    /// not bound tighter is given.
    pub(crate) fn deadline(&self) -> Duration {
        self.host.0.context.call_timeout
    }

    /// The bound of a call the kernel bounds by the same deadline (a context
    /// section, a prompt): shorter, so a hung guest is killed for it and
    /// the kernel still gets the call's error in time.
    pub(crate) fn kernel_bounded(&self) -> Duration {
        self.deadline().mul_f64(0.8)
    }

    /// Calls `method` with `args` within `timeout`; `Err(None)` when
    /// `cancellation` was cancelled first, which drops (and so cancels) the
    /// call.
    pub(crate) async fn call(
        &self,
        method: GuestMethod,
        args: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<Value, Option<String>> {
        if cancellation.is_cancelled() {
            return Err(None);
        }
        let call = self.host.call(&self.id, method.name(), args, timeout);
        tokio::select! {
            answer = call => answer.map_err(Some),
            () = cancelled(cancellation) => Err(None),
        }
    }
}

impl GuestCaller {
    /// What the Agent is told about a failed call: the Agent's words, which
    /// its logs may carry. The guest's own words (its error) are logged only
    /// at `trace`.
    pub(crate) fn failure(&self, method: GuestMethod, error: Option<String>) -> String {
        let Some(error) = error else {
            return format!("the call `{}` was cancelled", method.name());
        };
        log::trace!(
            "plugin {}: its guest's `{}` failed: {}",
            self.id,
            method.name(),
            bounded_trace_content(&error)
        );
        format!("the plugin's guest failed in `{}`", method.name())
    }
}

async fn cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_CHECK).await;
    }
}

impl Shared {
    /// Registers the proxies of the package `id` as effects of its guest's
    /// `fiber`, which just became active, and records what was left out.
    pub(crate) fn contribute(self: &Arc<Self>, id: &str, fiber: &Fiber) {
        let Some(package) = self.package(id) else {
            return;
        };
        let guest = GuestCaller::new(GuestHost(self.clone()), package.id.clone());
        let ctx = fiber.context();
        let mut issues = Vec::new();
        tools::register(ctx, &guest, &package, &mut issues);
        context::register(ctx, &guest, &package, &mut issues);
        let prompt = prompt::register(ctx, &package, &mut issues);
        hooks::register(ctx, &guest, &package, &mut issues);
        observers::register(ctx, &guest, &package, &mut issues);
        for issue in &issues {
            log::warn!("plugin {id}: {issue}");
        }
        if let Some(record) = self.records.lock().get_mut(id) {
            record.issues = issues;
            record.prompt = prompt;
        }
    }

    pub(crate) fn package(&self, id: &str) -> Option<GuestPackage> {
        self.packages
            .lock()
            .iter()
            .find(|package| package.id.key() == id)
            .cloned()
    }
}

/// Registers `item` in the registry `S` as an effect of `ctx`'s fiber;
/// what refused it becomes an issue about `what`.
fn register_in<S>(ctx: &Context, item: S::Item, what: &str, issues: &mut Vec<String>) -> bool
where
    S: hanafubuki_agent_contracts::RegistryService,
{
    use hanafubuki_agent_contracts::Register;
    let registered = ctx
        .get::<S>()
        .map_err(|error| error.to_string())
        .and_then(|registry| registry.register(item).map_err(|error| error.to_string()));
    match registered {
        Ok(_) => true,
        Err(error) => {
            issues.push(format!("{what} was not registered: {error}"));
            false
        }
    }
}

/// `text` cut to at most `bound` bytes on a character boundary.
fn bounded(mut text: String, bound: usize) -> String {
    if text.len() > bound {
        let mut end = bound;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// The name a guest's contributions are known by to the model and in the
/// events: `plugin.<plugin>.<marketplace>`, a tool provider's and a hook
/// provider's id.
fn provider_id(id: &PluginId) -> String {
    format!("plugin.{}.{}", id.plugin_name, id.marketplace_name)
}

/// The text a guest answered, or why the answer is not text.
fn text_answer(answer: Value) -> Result<Option<String>, String> {
    match answer {
        Value::Null => Ok(None),
        // Trimmed before any cut, so a cut text is never blank.
        Value::String(text) if text.trim().is_empty() => Ok(None),
        Value::String(text) => Ok(Some(text.trim().to_string())),
        other => Err(format!(
            "the guest answered {}, not a string or null",
            protocol::kind(&other)
        )),
    }
}
