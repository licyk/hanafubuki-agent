//! Tasks the Agent's own code (the plugin host, the service) spawns on the
//! Agent runtime. Nothing awaits some of them, so a panic in one would end
//! it silently; it is a kernel fault instead, reported to the instance's
//! sink, since kernel code stopped half-way.

use std::future::Future;
use std::sync::Arc;

use hanafubuki_agent_foundation::panic::catch_panic_async;
use hanafubuki_agent_kernel::AgentKernelFaultSink;
use tokio::task::JoinHandle;

/// Spawns `future` on the ambient runtime; a panic in it is logged,
/// reported to `faults` as a fault at `site`, and the task ends with `None`.
pub(crate) fn spawn_kernel_task<F>(
    faults: &Arc<dyn AgentKernelFaultSink>,
    site: &'static str,
    future: F,
) -> JoinHandle<Option<F::Output>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let faults = faults.clone();
    tokio::spawn(async move {
        match catch_panic_async(future).await {
            Ok(output) => Some(output),
            Err(message) => {
                log::error!("the Agent's {site} task panicked: {message}");
                faults.kernel_fault(site, &message);
                None
            }
        }
    })
}
