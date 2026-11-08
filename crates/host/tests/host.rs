//! The generic host routes a fault the application's module fault sink
//! records to the row of that name: the row is stopped and `Failed` until a
//! retry mounts it again, while the other rows keep running. A locked row
//! that is not core cannot be turned off but still fails alone; the service
//! a row provides is resolved from the host while the row is active; and
//! the ordered shutdown stops the rows newest first, each within its bound;
//! and a host with a restart policy mounts a faulted row again by itself
//! while the row's window allows. A row's status lists the rows that inject
//! what it provides, which wait while it is off.
//! The Agent's plugin host suites cover the rest of the host's mechanics.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::module_faults::{
    recorded_module_faults, report_module_fault, FaultOwner,
};
use hanafubuki_agent_host::{
    HostProfile, ModuleGroup, ModuleHost, ModuleHostOptions, ModuleLock, ModuleRow, ModuleState,
    ModuleStatus, PluginFaults, RestartPolicy,
};
use kardia_core::{Context, Disposer, PluginDef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Core,
    Platform,
    Feature,
}

impl ModuleGroup for Group {
    fn is_core(self) -> bool {
        self == Self::Core
    }

    fn lock(self) -> Option<ModuleLock> {
        match self {
            Self::Core => Some(ModuleLock::Core),
            Self::Platform => Some(ModuleLock::Platform),
            Self::Feature => None,
        }
    }
}

#[derive(Default)]
struct Probe {
    changes: Arc<AtomicUsize>,
    disabled: Arc<Mutex<BTreeSet<String>>>,
}

impl HostProfile for Probe {
    type Group = Group;
    type Settings = BTreeSet<String>;

    const NOUN: &'static str = "module";
    const HOST: &'static str = "the application";
    const SHUTDOWN_WORKER: &'static str = "modules";

    fn settings(&self) -> Option<BTreeSet<String>> {
        Some(self.disabled.lock().unwrap().clone())
    }

    fn disables(disabled: &BTreeSet<String>, name: &str) -> bool {
        disabled.contains(name)
    }

    fn modules_changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }

    fn host_task_panicked(&self, message: &str) {
        panic!("a host task panicked: {message}");
    }
}

fn row(group: Group, name: &'static str) -> ModuleRow<Probe> {
    ModuleRow::unconfigured(group, PluginDef::new(|_, _| async { Ok(()) }).name(name))
}

fn host(rows: Vec<ModuleRow<Probe>>, module_faults: bool) -> Arc<ModuleHost<Probe>> {
    host_with(Probe::default(), rows, module_faults)
}

fn host_with(
    probe: Probe,
    rows: Vec<ModuleRow<Probe>>,
    module_faults: bool,
) -> Arc<ModuleHost<Probe>> {
    host_options(
        probe,
        rows,
        ModuleHostOptions {
            safe_mode: false,
            module_faults,
            automatic_restarts: None,
        },
    )
}

fn host_options(
    probe: Probe,
    rows: Vec<ModuleRow<Probe>>,
    options: ModuleHostOptions,
) -> Arc<ModuleHost<Probe>> {
    Arc::new(ModuleHost::new(probe, Arc::new(PluginFaults::default()), rows, options).unwrap())
}

fn status_of(host: &ModuleHost<Probe>, name: &str) -> ModuleStatus<Group> {
    host.status()
        .into_iter()
        .find(|status| status.name == name)
        .unwrap_or_else(|| panic!("no status for {name}"))
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "the condition never held");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_module_fault_stops_its_row_until_a_retry_mounts_it_again() {
    let changes = Arc::new(AtomicUsize::new(0));
    let host = host_with(
        Probe {
            changes: changes.clone(),
            ..Probe::default()
        },
        vec![
            row(Group::Feature, "host-test-faulty"),
            row(Group::Feature, "host-test-healthy"),
        ],
        true,
    );
    host.ensure_started().await;
    assert_eq!(
        status_of(&host, "host-test-faulty").state,
        ModuleState::Active
    );

    report_module_fault(
        FaultOwner::Module("host-test-faulty"),
        "host-test-worker",
        "the worker broke",
    );
    eventually(|| status_of(&host, "host-test-faulty").state == ModuleState::Failed).await;
    assert_eq!(
        status_of(&host, "host-test-faulty").error.as_deref(),
        Some("panicked in host-test-worker: the worker broke")
    );
    assert_eq!(
        status_of(&host, "host-test-healthy").state,
        ModuleState::Active
    );
    assert!(changes.load(Ordering::SeqCst) > 0);

    // The retry waits for the stopped fiber's teardown, then mounts anew.
    host.retry("host-test-faulty").await.unwrap();
    assert_eq!(
        status_of(&host, "host-test-faulty").state,
        ModuleState::Active
    );
    assert_eq!(status_of(&host, "host-test-faulty").error, None);

    // Shut down, the host no longer follows the sink.
    host.shutdown(Instant::now() + Duration::from_secs(5)).await;
    report_module_fault(FaultOwner::Module("host-test-faulty"), "late", "after");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_ne!(
        status_of(&host, "host-test-faulty").error.as_deref(),
        Some("panicked in late: after")
    );
}

#[tokio::test]
async fn faults_of_names_no_row_takes_stay_recorded_only() {
    // A core row and a host of plugins that are not modules keep running.
    let modules = host(vec![row(Group::Core, "host-test-core")], true);
    let plugins = host(vec![row(Group::Feature, "host-test-plugin")], false);
    modules.ensure_started().await;
    plugins.ensure_started().await;

    report_module_fault(FaultOwner::Module("host-test-core"), "worker", "core broke");
    report_module_fault(
        FaultOwner::Module("host-test-plugin"),
        "worker",
        "not a module",
    );
    report_module_fault(
        FaultOwner::Module("host-test-unknown"),
        "worker",
        "nobody's",
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        status_of(&modules, "host-test-core").state,
        ModuleState::Active
    );
    assert_eq!(
        status_of(&plugins, "host-test-plugin").state,
        ModuleState::Active
    );
    let recorded = recorded_module_faults();
    for owner in ["host-test-core", "host-test-plugin", "host-test-unknown"] {
        assert!(recorded
            .recent
            .iter()
            .any(|fault| fault.owner == FaultOwner::Module(owner)));
    }
}

/// The service a test row provides under its name.
#[derive(Debug)]
struct Counter(&'static str);

fn providing(group: Group, name: &'static str) -> ModuleRow<Probe> {
    ModuleRow::unconfigured(
        group,
        PluginDef::new(move |ctx: Context, _| async move {
            ctx.provide_arc_as(name, Arc::new(Counter(name)))?;
            Ok(())
        })
        .name(name)
        .provides([name]),
    )
}

#[tokio::test]
async fn a_rows_service_resolves_while_it_is_active_and_says_why_it_does_not_otherwise() {
    let failing = ModuleRow::unconfigured(
        Group::Feature,
        PluginDef::new(|_, _| async { Err(kardia_core::Error::msg("the store is unreadable")) })
            .name("lookup-failing"),
    );
    let host = host_with(
        Probe {
            disabled: Arc::new(Mutex::new(BTreeSet::from(["lookup-off".to_string()]))),
            ..Probe::default()
        },
        vec![
            providing(Group::Feature, "lookup-active"),
            failing,
            providing(Group::Feature, "lookup-off"),
        ],
        true,
    );
    assert_eq!(
        host.available("lookup-active").unwrap_err(),
        "the lookup-active module is unavailable: it has not started yet"
    );
    host.ensure_started().await;

    assert_eq!(
        host.service::<Counter>("lookup-active").unwrap().0,
        "lookup-active"
    );
    let failed = host.service::<Counter>("lookup-failing").unwrap_err();
    assert!(
        failed.starts_with("the lookup-failing module is unavailable: ")
            && failed.contains("the store is unreadable"),
        "{failed}"
    );
    assert_eq!(
        host.available("lookup-off").unwrap_err(),
        "the lookup-off module is unavailable: it is turned off"
    );
    assert!(host.service::<String>("lookup-active").is_err());

    // A fault stops the row; its service is gone until a retry provides a
    // new one.
    report_module_fault(FaultOwner::Module("lookup-active"), "worker", "broke");
    eventually(|| host.available("lookup-active").is_err()).await;
    assert_eq!(
        host.available("lookup-active").unwrap_err(),
        "the lookup-active module is unavailable: panicked in worker: broke"
    );
    host.retry("lookup-active").await.unwrap();
    assert!(host.service::<Counter>("lookup-active").is_ok());
}

#[tokio::test]
async fn a_platform_row_cannot_be_turned_off_but_fails_alone_and_restarts() {
    let host = host_with(
        Probe {
            disabled: Arc::new(Mutex::new(BTreeSet::from(["platform-row".to_string()]))),
            ..Probe::default()
        },
        vec![row(Group::Platform, "platform-row")],
        true,
    );
    host.ensure_started().await;
    let status = status_of(&host, "platform-row");
    assert_eq!(
        (status.state, status.enabled, status.lock),
        (ModuleState::Active, true, Some(ModuleLock::Platform))
    );
    assert_eq!(
        host.check_enablement("platform-row", false).unwrap_err(),
        "module platform-row cannot be disabled"
    );

    report_module_fault(FaultOwner::Module("platform-row"), "worker", "broke");
    eventually(|| status_of(&host, "platform-row").state == ModuleState::Failed).await;
    host.retry("platform-row").await.unwrap();
    assert_eq!(status_of(&host, "platform-row").state, ModuleState::Active);
}

/// A row whose disposer records its name, after `hang` if it hangs.
fn recording(
    name: &'static str,
    stopped: Arc<Mutex<Vec<&'static str>>>,
    hang: bool,
) -> ModuleRow<Probe> {
    ModuleRow::unconfigured(
        Group::Feature,
        PluginDef::new(move |ctx: Context, _| {
            let stopped = stopped.clone();
            async move {
                ctx.effect_sync("record stop", move |scope| {
                    scope.defer(Disposer::future(move || async move {
                        if hang {
                            tokio::time::sleep(Duration::from_secs(60)).await;
                        }
                        stopped.lock().unwrap().push(name);
                    }));
                    Ok(())
                })?;
                Ok(())
            }
        })
        .name(name),
    )
}

#[tokio::test]
async fn the_ordered_shutdown_stops_rows_newest_first_each_within_its_bound() {
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let host = host(
        vec![
            recording("order-first", stopped.clone(), false),
            recording("order-hangs", stopped.clone(), true),
            recording("order-last", stopped.clone(), false),
        ],
        true,
    );
    host.ensure_started().await;

    let started = Instant::now();
    let diagnostics = host
        .shutdown_in_reverse(
            Instant::now() + Duration::from_secs(5),
            Duration::from_millis(200),
        )
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(*stopped.lock().unwrap(), vec!["order-last", "order-first"]);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(
        diagnostics[0].message,
        "module order-hangs did not stop in time"
    );
    assert_eq!(status_of(&host, "order-first").state, ModuleState::Disposed);
}

/// A row that counts its mounts.
fn counting(name: &'static str, mounts: Arc<AtomicUsize>) -> ModuleRow<Probe> {
    ModuleRow::unconfigured(
        Group::Feature,
        PluginDef::new(move |_, _| {
            let mounts = mounts.clone();
            async move {
                mounts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .name(name),
    )
}

/// A host following the module fault sink whose rows restart after
/// `delay`, three times in a ten-minute window.
fn restarting(
    probe: Probe,
    rows: Vec<ModuleRow<Probe>>,
    delay: Duration,
) -> Arc<ModuleHost<Probe>> {
    host_options(
        probe,
        rows,
        ModuleHostOptions {
            safe_mode: false,
            module_faults: true,
            automatic_restarts: Some(RestartPolicy {
                delays: vec![delay; 3],
                window: Duration::from_secs(600),
            }),
        },
    )
}

fn fault(name: &'static str) {
    report_module_fault(FaultOwner::Module(name), "restart-test-worker", "it broke");
}

#[tokio::test]
async fn a_faulted_row_restarts_after_its_backoff_until_its_window_is_used_up() {
    let mounts = Arc::new(AtomicUsize::new(0));
    let name = "restart-auto";
    let host = restarting(
        Probe::default(),
        vec![counting(name, mounts.clone())],
        Duration::from_millis(100),
    );
    host.ensure_started().await;
    assert_eq!(status_of(&host, name).restart, None);

    for made in 0..3u32 {
        fault(name);
        eventually(|| status_of(&host, name).state == ModuleState::Failed).await;
        let pending = status_of(&host, name).restart.unwrap();
        assert_eq!(pending.restarts_in_window, made);
        assert!(pending.next_attempt_at_unix_millis.is_some());
        eventually(|| status_of(&host, name).state == ModuleState::Active).await;
        let made = made + 1;
        assert_eq!(mounts.load(Ordering::SeqCst), made as usize + 1);
        let after = status_of(&host, name).restart.unwrap();
        assert_eq!(
            (after.restarts_in_window, after.next_attempt_at_unix_millis),
            (made, None)
        );
    }

    // The fourth fault in the window stays failed: no restart is pending.
    fault(name);
    eventually(|| status_of(&host, name).state == ModuleState::Failed).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let used_up = status_of(&host, name);
    assert_eq!(used_up.state, ModuleState::Failed);
    assert_eq!(
        used_up.restart.map(|restart| (
            restart.restarts_in_window,
            restart.next_attempt_at_unix_millis
        )),
        Some((3, None))
    );
    assert_eq!(mounts.load(Ordering::SeqCst), 4);

    // A retry starts it and empties the window, so a fault restarts it again.
    host.retry(name).await.unwrap();
    assert_eq!(status_of(&host, name).state, ModuleState::Active);
    assert_eq!(status_of(&host, name).restart, None);
    fault(name);
    eventually(|| status_of(&host, name).state == ModuleState::Failed).await;
    eventually(|| status_of(&host, name).state == ModuleState::Active).await;
    assert_eq!(mounts.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn a_failed_start_is_not_restarted_by_itself() {
    let host = restarting(
        Probe::default(),
        vec![ModuleRow::unconfigured(
            Group::Feature,
            PluginDef::new(|_, _| async { Err(kardia_core::Error::msg("no store")) })
                .name("restart-construction"),
        )],
        Duration::from_millis(50),
    );
    host.ensure_started().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let failed = status_of(&host, "restart-construction");
    assert_eq!((failed.state, failed.restart), (ModuleState::Failed, None));
}

#[tokio::test]
async fn turning_a_row_off_or_shutting_down_cancels_its_pending_restart() {
    let disabled = Arc::new(Mutex::new(BTreeSet::new()));
    let (off, down) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let host = restarting(
        Probe {
            disabled: disabled.clone(),
            ..Probe::default()
        },
        vec![
            counting("restart-off", off.clone()),
            counting("restart-down", down.clone()),
        ],
        Duration::from_millis(300),
    );
    host.ensure_started().await;

    fault("restart-off");
    eventually(|| status_of(&host, "restart-off").restart.is_some()).await;
    disabled.lock().unwrap().insert("restart-off".to_string());
    host.reconfigure();
    let turned_off = status_of(&host, "restart-off");
    assert_eq!(turned_off.state, ModuleState::Disabled);
    assert!(turned_off
        .restart
        .is_none_or(|restart| restart.next_attempt_at_unix_millis.is_none()));

    fault("restart-down");
    eventually(|| status_of(&host, "restart-down").restart.is_some()).await;
    host.shutdown(Instant::now() + Duration::from_secs(5)).await;

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(off.load(Ordering::SeqCst), 1);
    assert_eq!(down.load(Ordering::SeqCst), 1);
    assert_eq!(status_of(&host, "restart-off").state, ModuleState::Disabled);
}

#[tokio::test]
async fn a_row_lists_the_rows_that_wait_for_it_and_they_stop_while_it_is_off() {
    let disabled = Arc::new(Mutex::new(BTreeSet::new()));
    let host = host_with(
        Probe {
            disabled: disabled.clone(),
            ..Probe::default()
        },
        vec![
            providing(Group::Feature, "graph-base"),
            providing(Group::Feature, "graph-middle").inject(["graph-base"]),
            row(Group::Feature, "graph-top").inject(["graph-middle"]),
            providing(Group::Feature, "graph-apart"),
        ],
        false,
    );
    let dependents = |name| status_of(&host, name).dependents;
    assert_eq!(dependents("graph-base"), ["graph-middle", "graph-top"]);
    assert_eq!(dependents("graph-middle"), ["graph-top"]);
    assert!(dependents("graph-top").is_empty());
    assert!(dependents("graph-apart").is_empty());

    host.ensure_started().await;
    assert_eq!(status_of(&host, "graph-top").state, ModuleState::Active);
    disabled.lock().unwrap().insert("graph-base".to_string());
    host.reconfigure();
    eventually(|| {
        ["graph-middle", "graph-top"]
            .iter()
            .all(|name| status_of(&host, name).state == ModuleState::Pending)
    })
    .await;
    assert_eq!(status_of(&host, "graph-apart").state, ModuleState::Active);
}
