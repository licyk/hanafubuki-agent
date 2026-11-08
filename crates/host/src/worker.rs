use std::future::Future;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use kardia_core::{Context, Disposer, EffectHandle};

use crate::fault::{FaultPoint, PluginFaults};

/// How long a stopping plugin waits for one of its workers to observe
/// cancellation before the worker task is aborted.
pub const WORKER_STOP_BOUND: Duration = Duration::from_secs(3);

/// Runs `work` as a Tokio task owned by the plugin of `ctx`.
///
/// kardia never cancels an async effect body, so a plugin must not run a
/// long-lived loop as one: its fiber could then never stop. This helper
/// spawns the task from a synchronous effect and, when the plugin stops,
/// cancels the token, waits up to [`WORKER_STOP_BOUND`], then aborts.
///
/// The plugin injects [`PluginFaults`] (`PluginFaults::NAME`): a panic in
/// `work` ends only the worker and is reported as a fault of the plugin,
/// which the host then stops.
pub fn spawn_scoped_worker<W, F>(
    ctx: &Context,
    label: &'static str,
    work: W,
) -> kardia_core::Result<EffectHandle>
where
    W: FnOnce(CancellationToken) -> F,
    F: Future<Output = ()> + Send + 'static,
{
    let faults = ctx.get::<PluginFaults>()?.arc().clone();
    let plugin = ctx.fiber().name();
    ctx.effect_sync(label, move |scope| {
        let cancellation = CancellationToken::default();
        let worker = work(cancellation.clone());
        let mut task = tokio::spawn(async move {
            let _ = faults
                .guard_async(&plugin, FaultPoint::worker(label), || worker)
                .await;
        });
        scope.defer(Disposer::future(move || async move {
            cancellation.cancel();
            if tokio::time::timeout(WORKER_STOP_BOUND, &mut task)
                .await
                .is_err()
            {
                task.abort();
                log::warn!("plugin worker {label} ignored cancellation and was aborted");
            }
        }));
        Ok(())
    })
}
