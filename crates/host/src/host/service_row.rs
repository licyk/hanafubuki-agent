//! A row that is one service: what a host of application modules mounts.
//!
//! Its `apply` runs the service's constructor on the blocking pool of the
//! runtime the host runs on, registers the stop of what the constructor
//! started, and provides the service under the row's name, where
//! [`super::ModuleHost::service`] finds it. A constructor error or panic
//! fails only the row. The stop runs on the blocking pool too when the row
//! is disposed (turned off, faulted, shut down), given `stop_bound`; what
//! it reports is logged and fails the disposal, which the host's shutdown
//! lists. Every mount constructs a new service, so a retried row never
//! inherits what a fault left behind.
//!
//! The constructor reaches what the row injects ([`ModuleRow::inject`])
//! through its [`ServiceContext`]: a sibling's service, or a registry it
//! contributes an entry to. A registration leaves when the row stops,
//! before the stop runs: the stop's disposer retires the row's
//! registrations (draining their leases) and only then runs the stop. They
//! share that one disposer because kardia disposes a fiber's effects
//! concurrently, so separate effects would keep no order.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_foundation::panic::panic_message;
use kardia_core::{Context, Disposer, PluginDef, Service};
use parking_lot::Mutex;

use super::row::ModuleRow;
use super::HostProfile;
use crate::RegistryService;

/// Stops what one mount of a service row started, by the deadline it is
/// given; reports what did not stop.
pub type ServiceStop = Box<dyn FnOnce(Instant) -> Vec<WorkerShutdownDiagnostic> + Send>;

/// Builds one mount of a service row: the service and how to stop it. Runs
/// on every mount, on the blocking pool.
pub type ServiceMount<S> =
    Arc<dyn Fn(&ServiceContext) -> Result<(Arc<S>, ServiceStop), String> + Send + Sync>;

type Retirement = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What a service row's constructor reaches: the services the row injects.
pub struct ServiceContext {
    ctx: Context,
    retirements: Arc<Mutex<Vec<Retirement>>>,
}

impl ServiceContext {
    /// The service `T` the row injects (under `T::NAME`).
    pub fn get<T: Service>(&self) -> Result<Arc<T>, String> {
        self.ctx
            .get::<T>()
            .map(|service| service.arc().clone())
            .map_err(|error| error.to_string())
    }

    /// The service of type `T` the row injects under `key`.
    pub fn get_as<T: std::any::Any + Send + Sync>(&self, key: &str) -> Result<Arc<T>, String> {
        self.ctx
            .get_as::<T>(key)
            .map(|service| service.arc().clone())
            .map_err(|error| error.to_string())
    }

    /// Adds `item` to the registry `R` the row injects: it is removed when
    /// the row stops, after the calls holding a lease on it drained, and
    /// before the row's stop runs.
    pub fn register<R: RegistryService>(&self, item: R::Item) -> Result<(), String> {
        let registry: Arc<R> = self.get::<R>()?;
        let entry = registry
            .registry()
            .insert(self.ctx.fiber().name(), item, R::admit)
            .map_err(|error| error.to_string())?;
        self.retirements.lock().push(Box::pin(async move {
            registry.registry().retire(&entry).await;
        }));
        Ok(())
    }
}

impl<K: HostProfile> ModuleRow<K> {
    /// A row named `name` whose every mount is `mount`: the service it
    /// returns is provided under `name` until the row is disposed, and its
    /// stop is given `stop_bound`.
    pub fn service<S: Send + Sync + 'static>(
        group: K::Group,
        name: &'static str,
        stop_bound: Duration,
        mount: ServiceMount<S>,
    ) -> Self {
        let def = PluginDef::new(move |ctx: Context, _| {
            let mount = mount.clone();
            async move {
                // Registered before the constructor runs, so a constructor
                // that fails after registering still retires what it added.
                let stop: Arc<Mutex<Option<ServiceStop>>> = Arc::default();
                let retirements: Arc<Mutex<Vec<Retirement>>> = Arc::default();
                let (stopping, retiring) = (stop.clone(), retirements.clone());
                ctx.effect_sync("stop", move |scope| {
                    scope.defer(Disposer::try_future(move || async move {
                        let pending = std::mem::take(&mut *retiring.lock());
                        for retirement in pending.into_iter().rev() {
                            retirement.await;
                        }
                        let stop = stopping.lock().take();
                        stop_on_blocking_pool::<K>(name, stop, stop_bound).await
                    }));
                    Ok(())
                })?;
                let context = ServiceContext {
                    ctx: ctx.clone(),
                    retirements,
                };
                let (service, started) = tokio::task::spawn_blocking(move || mount(&context))
                    .await
                    .map_err(|error| {
                        kardia_core::Error::msg(if error.is_panic() {
                            format!(
                                "its construction panicked: {}",
                                panic_message(error.into_panic().as_ref())
                            )
                        } else {
                            "its construction was cancelled".to_string()
                        })
                    })?
                    .map_err(kardia_core::Error::msg)?;
                *stop.lock() = Some(started);
                ctx.provide_arc_as(name, service)?;
                Ok(())
            }
        })
        .name(name)
        .provides([name]);
        Self {
            stop_bound: Some(stop_bound),
            ..Self::unconfigured(group, def)
        }
    }
}

/// Runs `stop` off the runtime's workers and logs what did not stop.
async fn stop_on_blocking_pool<K: HostProfile>(
    name: &'static str,
    stop: Option<ServiceStop>,
    stop_bound: Duration,
) -> kardia_core::Result<()> {
    let Some(stop) = stop else {
        return Ok(());
    };
    let diagnostics = tokio::task::spawn_blocking(move || stop(Instant::now() + stop_bound))
        .await
        .map_err(|error| kardia_core::Error::msg(format!("its stop failed: {error}")))?;
    if diagnostics.is_empty() {
        return Ok(());
    }
    let message = diagnostics
        .iter()
        .map(|diagnostic| {
            log::warn!(
                "{} {name} stop diagnostic worker={}: {}",
                K::NOUN,
                diagnostic.worker,
                diagnostic.message
            );
            format!("{}: {}", diagnostic.worker, diagnostic.message)
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(kardia_core::Error::msg(message))
}
