//! Service rows that depend on others: a row's constructor reaches the
//! services it injects (a sibling's, a registry the host provides on its
//! root), its registrations leave when it stops, before its stop runs, and
//! a row is mounted again when the sibling it injects is.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::module_faults::{report_module_fault, FaultOwner};
use hanafubuki_agent_host::{
    HostProfile, ModuleGroup, ModuleHost, ModuleHostOptions, ModuleLock, ModuleRow, ModuleState,
    PluginFaults, Registry, RegistryService, RootServices, ServiceMount, ServiceStop,
};
use kardia_macros::Service;
use parking_lot::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Feature;

impl ModuleGroup for Feature {
    fn is_core(self) -> bool {
        false
    }

    fn lock(self) -> Option<ModuleLock> {
        None
    }
}

#[derive(Service, Default)]
#[service(name = "test-contributions")]
struct Contributions {
    registry: Registry<&'static str>,
}

impl RegistryService for Contributions {
    type Item = &'static str;

    fn registry(&self) -> &Registry<&'static str> {
        &self.registry
    }
}

/// A host whose root provides one registry, with one permanent entry.
struct Probe {
    contributions: Arc<Contributions>,
}

impl HostProfile for Probe {
    type Group = Feature;
    type Settings = ();

    const NOUN: &'static str = "module";
    const HOST: &'static str = "the application";
    const SHUTDOWN_WORKER: &'static str = "modules";

    fn settings(&self) -> Option<()> {
        Some(())
    }

    fn disables(_: &(), _: &str) -> bool {
        false
    }

    fn modules_changed(&self) {}

    fn host_task_panicked(&self, message: &str) {
        panic!("a host task panicked: {message}");
    }

    fn provide_root(&self, root: &RootServices<'_>) -> Result<(), String> {
        root.provide(self.contributions.clone())
    }
}

/// A sibling's service: how many times it was built.
struct Upstream(usize);

fn items(contributions: &Contributions) -> Vec<&'static str> {
    contributions.registry.snapshot().items().copied().collect()
}

fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "the condition never held");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_row_reaches_what_it_injects_and_its_registrations_leave_before_its_stop() {
    let contributions = Arc::new(Contributions::default());
    contributions
        .registry
        .register_permanent("core", "core", |_, _| Ok(()))
        .unwrap();
    let built = Arc::new(AtomicUsize::new(0));
    let upstream: ServiceMount<Upstream> = {
        let built = built.clone();
        Arc::new(move |_| {
            let count = built.fetch_add(1, Ordering::SeqCst) + 1;
            Ok((
                Arc::new(Upstream(count)),
                Box::new(|_| Vec::new()) as ServiceStop,
            ))
        })
    };
    // What the dependent row saw: the upstream build it was given, and the
    // registry's entries when its stop ran.
    let seen = Arc::new(Mutex::new(Vec::<usize>::new()));
    let at_stop = Arc::new(Mutex::new(Vec::<Vec<&'static str>>::new()));
    let dependent: ServiceMount<()> = {
        let (seen, at_stop) = (seen.clone(), at_stop.clone());
        let contributions = contributions.clone();
        Arc::new(move |context| {
            let upstream = context.get_as::<Upstream>("upstream")?;
            seen.lock().push(upstream.0);
            context.register::<Contributions>("dependent")?;
            let (at_stop, contributions) = (at_stop.clone(), contributions.clone());
            Ok((
                Arc::new(()),
                Box::new(move |_| {
                    at_stop.lock().push(items(&contributions));
                    Vec::new()
                }) as ServiceStop,
            ))
        })
    };
    let host = Arc::new(
        ModuleHost::new(
            Probe {
                contributions: contributions.clone(),
            },
            Arc::new(PluginFaults::default()),
            vec![
                ModuleRow::service(Feature, "upstream", Duration::from_secs(1), upstream),
                ModuleRow::service(Feature, "dependent", Duration::from_secs(1), dependent)
                    .inject(["upstream", "test-contributions"]),
            ],
            ModuleHostOptions::default(),
        )
        .unwrap(),
    );
    host.ensure_started().await;

    assert!(host.available("dependent").is_ok());
    assert_eq!(*seen.lock(), vec![1]);
    assert_eq!(items(&contributions), vec!["core", "dependent"]);

    let shut = host.clone();
    let diagnostics = shut
        .shutdown_in_reverse(
            Instant::now() + Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .await;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    // Its registration was gone when its stop ran; the permanent one stays.
    assert_eq!(*at_stop.lock(), vec![vec!["core"]]);
    assert_eq!(items(&contributions), vec!["core"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_row_is_mounted_again_when_the_sibling_it_injects_is() {
    let contributions = Arc::new(Contributions::default());
    let built = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(Mutex::new(true));
    let upstream: ServiceMount<Upstream> = {
        let (built, fail_once) = (built.clone(), fail_once.clone());
        Arc::new(move |_| {
            if std::mem::take(&mut *fail_once.lock()) {
                return Err("the first build fails".to_string());
            }
            let count = built.fetch_add(1, Ordering::SeqCst) + 1;
            Ok((
                Arc::new(Upstream(count)),
                Box::new(|_| Vec::new()) as ServiceStop,
            ))
        })
    };
    let seen = Arc::new(Mutex::new(Vec::<usize>::new()));
    let dependent: ServiceMount<()> = {
        let seen = seen.clone();
        Arc::new(move |context| {
            seen.lock()
                .push(context.get_as::<Upstream>("upstream-b")?.0);
            Ok((Arc::new(()), Box::new(|_| Vec::new()) as ServiceStop))
        })
    };
    let host = Arc::new(
        ModuleHost::new(
            Probe { contributions },
            Arc::new(PluginFaults::default()),
            vec![
                ModuleRow::service(Feature, "upstream-b", Duration::from_secs(1), upstream),
                ModuleRow::service(Feature, "dependent-b", Duration::from_secs(1), dependent)
                    .inject(["upstream-b"]),
            ],
            ModuleHostOptions {
                module_faults: true,
                ..ModuleHostOptions::default()
            },
        )
        .unwrap(),
    );
    host.ensure_started().await;

    // Without its sibling it waits, unavailable.
    let state = |name: &str| {
        host.status()
            .into_iter()
            .find(|status| status.name == name)
            .unwrap()
            .state
    };
    assert_eq!(state("upstream-b"), ModuleState::Failed);
    assert!(host.available("dependent-b").is_err());
    assert!(seen.lock().is_empty());

    host.retry("upstream-b").await.unwrap();
    let waiting = host.clone();
    tokio::task::spawn_blocking(move || {
        eventually(|| waiting.available("dependent-b").is_ok());
    })
    .await
    .unwrap();
    assert_eq!(*seen.lock(), vec![1]);

    // The sibling faults: the dependent stops with it, and is mounted
    // again over the sibling's next instance.
    report_module_fault(FaultOwner::Module("upstream-b"), "worker", "it broke");
    let failing = host.clone();
    tokio::task::spawn_blocking(move || {
        eventually(|| failing.available("dependent-b").is_err());
    })
    .await
    .unwrap();
    host.retry("upstream-b").await.unwrap();
    let waiting = host.clone();
    tokio::task::spawn_blocking(move || {
        eventually(|| waiting.available("dependent-b").is_ok());
    })
    .await
    .unwrap();
    assert_eq!(*seen.lock(), vec![1, 2]);
    host.shutdown(Instant::now() + Duration::from_secs(5)).await;
}

/// A service row whose stop takes `stopping`, given `stop_bound`.
fn slow_stop(name: &'static str, stop_bound: Duration, stopping: Duration) -> ModuleRow<Probe> {
    let mount: ServiceMount<Upstream> = Arc::new(move |_| {
        Ok((
            Arc::new(Upstream(0)),
            Box::new(move |_| {
                std::thread::sleep(stopping);
                Vec::new()
            }) as ServiceStop,
        ))
    });
    ModuleRow::service(Feature, name, stop_bound, mount)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ordered_shutdown_waits_for_each_row_its_own_stop_bound_and_the_grace() {
    let host = Arc::new(
        ModuleHost::new(
            Probe {
                contributions: Arc::new(Contributions::default()),
            },
            Arc::new(PluginFaults::default()),
            vec![
                slow_stop(
                    "short-bound",
                    Duration::from_millis(50),
                    Duration::from_millis(500),
                ),
                slow_stop(
                    "long-bound",
                    Duration::from_millis(600),
                    Duration::from_millis(500),
                ),
            ],
            ModuleHostOptions::default(),
        )
        .unwrap(),
    );
    host.ensure_started().await;

    let diagnostics = host
        .shutdown_in_reverse(
            Instant::now() + Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .await;
    let messages = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert_eq!(messages, vec!["module short-bound did not stop in time"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn during_the_ordered_shutdown_a_rows_stop_still_reaches_the_rows_mounted_before_it() {
    let host_cell: Arc<std::sync::OnceLock<Arc<ModuleHost<Probe>>>> = Arc::default();
    let reached = Arc::new(Mutex::new(Vec::new()));
    let upstream: ServiceMount<Upstream> = Arc::new(|_| {
        Ok((
            Arc::new(Upstream(1)),
            Box::new(|_| Vec::new()) as ServiceStop,
        ))
    });
    let downstream: ServiceMount<Upstream> = {
        let (host_cell, reached) = (host_cell.clone(), reached.clone());
        Arc::new(move |_| {
            let (host_cell, reached) = (host_cell.clone(), reached.clone());
            Ok((
                Arc::new(Upstream(2)),
                Box::new(move |_| {
                    let host = host_cell.get().unwrap();
                    reached.lock().push((
                        host.service::<Upstream>("first").map(|service| service.0),
                        host.available("second").is_ok(),
                    ));
                    Vec::new()
                }) as ServiceStop,
            ))
        })
    };
    let host = Arc::new(
        ModuleHost::new(
            Probe {
                contributions: Arc::new(Contributions::default()),
            },
            Arc::new(PluginFaults::default()),
            vec![
                ModuleRow::service(Feature, "first", Duration::from_secs(1), upstream),
                ModuleRow::service(Feature, "second", Duration::from_secs(1), downstream),
            ],
            ModuleHostOptions::default(),
        )
        .unwrap(),
    );
    assert!(host_cell.set(host.clone()).is_ok());
    host.ensure_started().await;

    let diagnostics = host
        .shutdown_in_reverse(
            Instant::now() + Duration::from_secs(5),
            Duration::from_millis(500),
        )
        .await;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    // The first row served the second's stop, the second, stopping, no
    // longer did; nothing serves afterwards.
    assert_eq!(*reached.lock(), vec![(Ok(1), false)]);
    assert_eq!(
        host.available("first").unwrap_err(),
        "the first module is unavailable: the application is stopping"
    );
}
