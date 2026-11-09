//! The plugin host: settings-driven configuration and enablement, status,
//! contributions and retry, and shutdown. Most tests mount small probe plugins
//! on a bare host; the last ones mount the application's real manifest.

use super::host::{
    AgentHost, AgentPluginEventSink, AgentPluginGroup, AgentPluginLock, AgentPluginRow,
    AgentPluginState, AgentPluginStatus,
};
use super::*;
use crate::test_support::TestBlockingOperationPort;
use crate::test_support::TestPaths;
use crate::AgentConfig;
use hanafubuki_agent_contracts::{
    spawn_scoped_worker, AgentNotification, AgentObserver, ObserverRegistry, PluginError,
    PluginFaults, Register, RegistryService, WORKER_STOP_BOUND,
};
use kardia_core::{Context, Disposer, Inject, PluginDef, Service};
use serde_json::Value;

use AgentPluginGroup::Capability;

/// A plugin that records every configuration it was applied with.
fn recording_plugin(name: &'static str, applied: Arc<Mutex<Vec<Value>>>) -> PluginDef {
    PluginDef::new(move |_, config: Value| {
        let applied = applied.clone();
        async move {
            applied.lock().unwrap().push(config);
            Ok(())
        }
    })
    .name(name)
}

#[derive(Default)]
struct CountingEvents(AtomicUsize);

impl AgentPluginEventSink for CountingEvents {
    fn plugins_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl CountingEvents {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

struct ProbeObserver;

impl AgentObserver for ProbeObserver {
    fn id(&self) -> &str {
        "probe.observer"
    }

    fn observe(&self, _notification: &AgentNotification) {}
}

/// How often the probe started, how many of its fibers are alive, and
/// whether two ever were at once.
#[derive(Default)]
struct ProbeLife {
    starts: AtomicUsize,
    live: AtomicUsize,
    overlapped: std::sync::atomic::AtomicBool,
}

impl ProbeLife {
    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    fn overlapped(&self) -> bool {
        self.overlapped.load(Ordering::SeqCst)
    }
}

/// Registers one observer. Its teardown takes a moment, and a fiber that
/// starts while an earlier one is still stopping is recorded.
fn observing_plugin(life: Arc<ProbeLife>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let life = life.clone();
        async move {
            life.starts.fetch_add(1, Ordering::SeqCst);
            if life.live.fetch_add(1, Ordering::SeqCst) > 0 {
                life.overlapped.store(true, Ordering::SeqCst);
            }
            ctx.get::<ObserverRegistry>()?
                .register(Arc::new(ProbeObserver))?;
            ctx.defer(
                "slow-stop",
                Disposer::future(move || async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    life.live.fetch_sub(1, Ordering::SeqCst);
                }),
            )?;
            Ok(())
        }
    })
    .name("probe")
    .inject(Inject::new().with(ObserverRegistry::NAME))
}

fn host_with(
    name: &str,
    manifest: Vec<AgentPluginRow>,
) -> (Arc<AgentHost>, TestPaths, AgentConfig) {
    host_configured(name, |_, _| {}, manifest).0
}

fn host_configured(
    name: &str,
    configure: impl FnOnce(&TestPaths, &AgentConfig),
    manifest: Vec<AgentPluginRow>,
) -> (
    (Arc<AgentHost>, TestPaths, AgentConfig),
    Arc<CountingEvents>,
) {
    let paths = test_paths(name);
    let config = AgentConfig::new(&paths.config_dir);
    config.settings().unwrap();
    configure(&paths, &config);
    let events = Arc::new(CountingEvents::default());
    let host = AgentHost::new(
        hanafubuki_agent_kernel::AgentKernelRegistries::default(),
        Arc::new(TestBlockingOperationPort),
        config.clone(),
        Arc::new(FixedProxy(AgentProxy::System)),
        events.clone(),
        manifest,
        super::host::AgentHostOptions::default(),
    )
    .unwrap();
    ((Arc::new(host), paths, config), events)
}

fn status_of<'a>(statuses: &'a [AgentPluginStatus], name: &str) -> &'a AgentPluginStatus {
    statuses
        .iter()
        .find(|status| status.name == name)
        .unwrap_or_else(|| panic!("no status for {name}"))
}

fn disable(config: &AgentConfig, names: &[&str]) {
    for name in config.settings().unwrap().disabled_plugins {
        config.set_plugin_disabled(&name, false).unwrap();
    }
    for name in names {
        config.set_plugin_disabled(name, true).unwrap();
    }
}

fn jina_reader_on(config: &AgentConfig) {
    config
        .write_settings(AgentSettingsPatch {
            web_fetch_use_jina_reader: Some(true),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
}

fn observers_owned_by(host: &AgentHost, owner: &str) -> usize {
    host.registries()
        .observers
        .registry()
        .snapshot()
        .entries()
        .iter()
        .filter(|entry| entry.owner() == owner)
        .count()
}

/// Waits for a teardown that runs in the background.
async fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "the condition never held");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn state_of(statuses: &[AgentPluginStatus], name: &str) -> AgentPluginState {
    status_of(statuses, name).state
}

#[tokio::test]
async fn a_settings_change_updates_only_the_plugins_whose_configuration_changed() {
    let follows = Arc::new(Mutex::new(Vec::new()));
    let fixed = Arc::new(Mutex::new(Vec::new()));
    let unconfigured = Arc::new(Mutex::new(Vec::new()));
    let (host, paths, config) = host_with(
        "plugin-host-live-config",
        vec![
            AgentPluginRow::from_settings(
                Capability,
                recording_plugin("follows", follows.clone()),
                |settings| Ok(Value::Bool(settings.agent.web_fetch_use_jina_reader)),
            ),
            AgentPluginRow::from_settings(
                Capability,
                recording_plugin("fixed", fixed.clone()),
                |settings| Ok(serde_json::to_value(&settings.agent.searxng_base_url).unwrap()),
            ),
            AgentPluginRow::unconfigured(
                Capability,
                recording_plugin("unconfigured", unconfigured.clone()),
            ),
        ],
    );
    host.ensure_started().await;
    assert_eq!(*follows.lock().unwrap(), vec![Value::Bool(false)]);
    assert!(host.reconfigure().is_empty());

    jina_reader_on(&config);
    let updates = host.reconfigure();
    assert_eq!(
        updates
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["follows"]
    );
    for (_, settle) in updates {
        settle.await.unwrap();
    }

    assert_eq!(
        *follows.lock().unwrap(),
        vec![Value::Bool(false), Value::Bool(true)]
    );
    assert_eq!(fixed.lock().unwrap().len(), 1);
    assert_eq!(unconfigured.lock().unwrap().len(), 1);
    assert!(host.reconfigure().is_empty());
    assert_eq!(
        state_of(&host.status(), "follows"),
        AgentPluginState::Active
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_settings_change_before_start_up_is_mounted_with_the_new_configuration() {
    let follows = Arc::new(Mutex::new(Vec::new()));
    let (host, paths, config) = host_with(
        "plugin-host-config-before-start",
        vec![AgentPluginRow::from_settings(
            Capability,
            recording_plugin("follows", follows.clone()),
            |settings| Ok(Value::Bool(settings.agent.web_fetch_use_jina_reader)),
        )],
    );
    assert_eq!(
        state_of(&host.status(), "follows"),
        AgentPluginState::Pending
    );

    jina_reader_on(&config);
    assert!(host.reconfigure().is_empty());
    host.ensure_started().await;

    assert_eq!(*follows.lock().unwrap(), vec![Value::Bool(true)]);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_failed_plugin_reports_its_error_and_starts_again_on_retry() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let failing = {
        let attempts = attempts.clone();
        PluginDef::new(move |_, _| {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    return Err(kardia_core::Error::msg("probe refused to start"));
                }
                Ok(())
            }
        })
        .name("flaky")
    };
    let (host, paths, _) = host_with(
        "plugin-host-retry",
        vec![AgentPluginRow::unconfigured(Capability, failing)],
    );
    host.ensure_started().await;

    let statuses = host.status();
    let flaky = statuses
        .iter()
        .find(|status| status.name == "flaky")
        .unwrap();
    assert_eq!(flaky.state, AgentPluginState::Failed);
    assert!(flaky
        .error
        .as_deref()
        .unwrap()
        .contains("probe refused to start"));
    assert_eq!(
        state_of(&statuses, "agent-kernel"),
        AgentPluginState::Active
    );
    assert!(host
        .retry("missing")
        .await
        .unwrap_err()
        .contains("does not exist"));
    assert!(host
        .retry("agent-kernel")
        .await
        .unwrap_err()
        .contains("has not failed"));

    host.retry("flaky").await.unwrap();

    let statuses = host.status();
    let flaky = statuses
        .iter()
        .find(|status| status.name == "flaky")
        .unwrap();
    assert_eq!(flaky.state, AgentPluginState::Active);
    assert_eq!(flaky.error, None);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A plugin whose code faults is stopped and `Failed` with its pending
/// restart, then starts again by itself after the restart policy's delay.
#[tokio::test]
async fn a_faulted_plugin_starts_again_by_itself_under_the_restart_policy() {
    let starts = Arc::new(AtomicUsize::new(0));
    let breaking = {
        let starts = starts.clone();
        PluginDef::new(move |ctx: Context, _| {
            let first = starts.fetch_add(1, Ordering::SeqCst) == 0;
            async move {
                spawn_scoped_worker(&ctx, "breaks", move |_| async move {
                    assert!(!first, "the worker broke");
                })?;
                Ok(())
            }
        })
        .name("breaks")
        .inject(Inject::new().with(PluginFaults::NAME))
    };
    let paths = test_paths("plugin-host-automatic-restart");
    let config = AgentConfig::new(&paths.config_dir);
    config.settings().unwrap();
    let host = AgentHost::new(
        hanafubuki_agent_kernel::AgentKernelRegistries::default(),
        Arc::new(TestBlockingOperationPort),
        config,
        Arc::new(FixedProxy(AgentProxy::System)),
        Arc::new(CountingEvents::default()),
        vec![AgentPluginRow::unconfigured(Capability, breaking)],
        super::host::AgentHostOptions {
            restart_policy: hanafubuki_agent_host::RestartPolicy {
                delays: vec![Duration::from_millis(300); 3],
                window: Duration::from_secs(600),
            },
            ..super::host::AgentHostOptions::default()
        },
    )
    .unwrap();
    host.ensure_started().await;

    eventually(|| state_of(&host.status(), "breaks") == AgentPluginState::Failed).await;
    let failed = status_of(&host.status(), "breaks").clone();
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("panicked in worker breaks")),
        "{failed:?}"
    );
    let pending = failed.restart.expect("a restart is pending");
    assert_eq!(pending.restarts_in_window, 0);
    assert!(pending.next_attempt_at_unix_millis.is_some());

    eventually(|| state_of(&host.status(), "breaks") == AgentPluginState::Active).await;
    let restarted = status_of(&host.status(), "breaks").clone();
    assert_eq!(restarted.error, None);
    assert_eq!(
        restarted.restart.map(|restart| restart.restarts_in_window),
        Some(1)
    );
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert!(host
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .await
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A plugin's warning reaches the interface as a plugin change when it
/// appears and when it clears, once each; the same warning recorded again,
/// as a contributor failing every turn would, announces nothing.
#[tokio::test]
async fn a_warning_is_announced_when_it_appears_and_clears_but_not_when_repeated() {
    let ((host, paths, _), events) = host_configured(
        "plugin-host-warning-events",
        |_, _| {},
        vec![AgentPluginRow::unconfigured(
            Capability,
            recording_plugin("probe", Arc::default()),
        )],
    );
    host.ensure_started().await;
    let faults = &host.registries().faults;
    let before = events.count();

    faults.set_issue("probe", "context contributor probe", Some("offline".into()));
    assert_eq!(events.count(), before + 1);
    assert_eq!(
        status_of(&host.status(), "probe").warning.as_deref(),
        Some("context contributor probe: offline")
    );
    faults.set_issue("probe", "context contributor probe", Some("offline".into()));
    assert_eq!(
        events.count(),
        before + 1,
        "a repeated warning is no change"
    );

    faults.set_issue("probe", "context contributor probe", None);
    assert_eq!(events.count(), before + 2);
    assert_eq!(status_of(&host.status(), "probe").warning, None);
    faults.set_issue("probe", "context contributor probe", None);
    assert_eq!(
        events.count(),
        before + 2,
        "a contributor that keeps working"
    );

    host.shutdown(Instant::now() + Duration::from_secs(7)).await;
    let after_shutdown = events.count();
    faults.set_issue("probe", "context contributor probe", Some("offline".into()));
    assert_eq!(
        events.count(),
        after_shutdown,
        "shutdown stops the announcements"
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_worker_ignoring_cancellation_cannot_hold_shutdown_past_its_deadline() {
    let stubborn = PluginDef::new(|ctx, _| async move {
        spawn_scoped_worker(&ctx, "stubborn", |_cancellation| {
            std::future::pending::<()>()
        })?;
        Ok(())
    })
    .name("stubborn")
    .inject(Inject::new().with(PluginFaults::NAME));
    let (host, paths, _) = host_with(
        "plugin-host-stubborn-shutdown",
        vec![AgentPluginRow::unconfigured(Capability, stubborn)],
    );
    host.ensure_started().await;
    assert_eq!(
        state_of(&host.status(), "stubborn"),
        AgentPluginState::Active
    );

    let started = Instant::now();
    let diagnostics = host.shutdown(started + Duration::from_millis(200)).await;

    assert!(started.elapsed() < WORKER_STOP_BOUND);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].worker, "agent-plugins");
    assert!(diagnostics[0].message.contains("deadline"));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn shutdown_leaves_no_registration_in_any_kernel_registry() {
    let (service, paths) = tools::service_with("plugin-host-shutdown-registries", |_, _| {});
    let registries = service.test_plugin_registries();
    service.start_plugins().await;
    assert!(service
        .plugin_status()
        .iter()
        .all(|status| status.state == AgentPluginState::Active));
    assert!(!registries.tools.registry().snapshot().is_empty());
    assert!(!registries.tool_sources.registry().snapshot().is_empty());
    assert!(!registries.context.registry().snapshot().is_empty());
    assert!(!registries.approval_rules.registry().snapshot().is_empty());
    assert!(!registries.observers.registry().snapshot().is_empty());

    let diagnostics = service
        .shutdown_plugins(Instant::now() + Duration::from_secs(7))
        .await;

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(registries.tools.registry().snapshot().is_empty());
    assert!(registries.tool_sources.registry().snapshot().is_empty());
    assert!(registries.context.registry().snapshot().is_empty());
    assert!(registries.approval_rules.registry().snapshot().is_empty());
    assert!(registries
        .continuation_hooks
        .registry()
        .snapshot()
        .is_empty());
    assert!(registries.observers.registry().snapshot().is_empty());
    assert!(service
        .plugin_status()
        .iter()
        .all(|status| status.state == AgentPluginState::Disposed));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn disabling_a_plugin_removes_its_registrations_and_enabling_it_registers_them_again() {
    let life = Arc::new(ProbeLife::default());
    let ((host, paths, config), events) = host_configured(
        "plugin-host-enablement",
        |_, _| {},
        vec![AgentPluginRow::unconfigured(
            Capability,
            observing_plugin(life.clone()),
        )],
    );
    host.ensure_started().await;
    let probe = status_of(&host.status(), "probe").clone();
    assert_eq!(probe.state, AgentPluginState::Active);
    assert!(probe.enabled);
    assert_eq!(probe.lock, None);
    assert_eq!(probe.contributions.observers, ["probe.observer"]);
    let after_start = events.count();
    assert!(after_start >= 1);

    disable(&config, &["probe"]);
    assert!(host.reconfigure().is_empty());
    let probe = status_of(&host.status(), "probe").clone();
    assert_eq!(probe.state, AgentPluginState::Disabled);
    assert!(!probe.enabled);
    assert_eq!(events.count(), after_start + 1);
    // Told again once the teardown ended, when the registrations are gone.
    eventually(|| events.count() == after_start + 2).await;
    assert_eq!(observers_owned_by(&host, "probe"), 0);
    assert!(status_of(&host.status(), "probe")
        .contributions
        .observers
        .is_empty());
    // A save that does not touch the set changes nothing.
    assert!(host.reconfigure().is_empty());
    assert_eq!(events.count(), after_start + 2);

    disable(&config, &[]);
    let settles = host.reconfigure();
    assert_eq!(settles.len(), 1);
    for (_, settle) in settles {
        settle.await.unwrap();
    }
    assert_eq!(state_of(&host.status(), "probe"), AgentPluginState::Active);
    assert_eq!(observers_owned_by(&host, "probe"), 1);
    assert_eq!(life.starts(), 2);
    assert!(!life.overlapped());
    // Told once when the row flipped and once when its start settled.
    assert_eq!(events.count(), after_start + 4);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn re_enabling_waits_for_the_disabled_fiber_to_stop_before_mounting_again() {
    let life = Arc::new(ProbeLife::default());
    let ((host, paths, config), _) = host_configured(
        "plugin-host-quick-toggle",
        |_, _| {},
        vec![AgentPluginRow::unconfigured(
            Capability,
            observing_plugin(life.clone()),
        )],
    );
    host.ensure_started().await;

    // Off and on again before the old fiber's slow teardown has ended: the
    // new fiber must not register while the old registration is still there.
    disable(&config, &["probe"]);
    assert!(host.reconfigure().is_empty());
    disable(&config, &[]);
    let settles = host.reconfigure();
    assert_eq!(state_of(&host.status(), "probe"), AgentPluginState::Pending);
    for (_, settle) in settles {
        settle.await.unwrap();
    }

    let probe = status_of(&host.status(), "probe").clone();
    assert_eq!(probe.state, AgentPluginState::Active, "{:?}", probe.error);
    assert_eq!(observers_owned_by(&host, "probe"), 1);
    assert_eq!(life.starts(), 2);
    assert!(!life.overlapped());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_disabled_plugin_is_not_mounted_at_start_up_and_core_plugins_cannot_be_disabled() {
    let life = Arc::new(ProbeLife::default());
    let ((host, paths, _), _) = host_configured(
        "plugin-host-disabled-at-start",
        |_, config| {
            disable(
                config,
                &[
                    "agent-kernel",
                    "agent-host-services",
                    "probe",
                    "not-a-plugin",
                ],
            )
        },
        vec![AgentPluginRow::unconfigured(
            Capability,
            observing_plugin(life.clone()),
        )],
    );
    assert_eq!(
        state_of(&host.status(), "probe"),
        AgentPluginState::Disabled
    );
    host.ensure_started().await;

    let statuses = host.status();
    assert_eq!(state_of(&statuses, "probe"), AgentPluginState::Disabled);
    assert_eq!(life.starts(), 0);
    assert_eq!(observers_owned_by(&host, "probe"), 0);
    for core in ["agent-host-services", "agent-kernel"] {
        let status = status_of(&statuses, core);
        assert_eq!(status.state, AgentPluginState::Active);
        assert_eq!(status.group, AgentPluginGroup::Core);
        assert!(status.enabled);
        assert_eq!(status.lock, Some(AgentPluginLock::Core));
        assert!(host
            .check_enablement(core, false)
            .unwrap_err()
            .contains("cannot be disabled"));
        host.check_enablement(core, true).unwrap();
    }
    host.check_enablement("probe", true).unwrap();
    assert!(host
        .check_enablement("not-a-plugin", true)
        .unwrap_err()
        .contains("does not exist"));
    assert!(host.retry("probe").await.unwrap_err().contains("disabled"));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn the_status_lists_what_each_manifest_plugin_contributes_and_its_group() {
    let (service, paths) = tools::service_with("plugin-host-contributions", |_, _| {});
    service.start_plugins().await;
    let statuses = service.plugin_status();

    let names = statuses
        .iter()
        .map(|status| status.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(&names[..2], ["agent-host-services", "agent-kernel"]);
    let web = status_of(&statuses, "web");
    assert_eq!(web.group, AgentPluginGroup::Capability);
    assert_eq!(web.contributions.tools, ["web_search", "fetch_web_page"]);
    let mcp = status_of(&statuses, "mcp");
    assert_eq!(mcp.contributions.tool_sources.len(), 1);
    assert!(mcp
        .contributions
        .tools
        .contains(&"read_mcp_resource".to_string()));
    let execution = status_of(&statuses, "execution");
    assert_eq!(execution.contributions.approval_rules.len(), 1);
    let memory = status_of(&statuses, "memory");
    assert_eq!(memory.contributions.observers.len(), 1);
    assert_eq!(memory.contributions.prompt, ["memory"]);
    let skills = status_of(&statuses, "skills");
    assert_eq!(skills.contributions.turn_admission.len(), 1);
    assert_eq!(skills.contributions.prompt.len(), 1);
    let workspace = status_of(&statuses, "workspace-context");
    assert_eq!(workspace.group, AgentPluginGroup::Context);
    assert_eq!(
        workspace
            .contributions
            .context
            .iter()
            .flat_map(|contributor| contributor.kinds.iter().map(String::as_str))
            .collect::<Vec<_>>(),
        ["workspace_instructions", "workspace_roots", "environment"]
    );
    let kernel = status_of(&statuses, "agent-kernel");
    assert_eq!(kernel.contributions, Default::default());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn set_plugin_enabled_persists_the_choice_and_applies_it_to_the_running_plugins() {
    let (service, paths) = tools::service_with("plugin-host-set-enabled", |_, _| {});
    let registries = service.test_plugin_registries();
    service.start_plugins().await;
    let web_tools = || {
        registries
            .tools
            .registry()
            .snapshot()
            .entries()
            .iter()
            .filter(|entry| entry.owner() == "web")
            .count()
    };
    assert_eq!(web_tools(), 2);

    let settings = service
        .set_plugin_enabled("web".into(), false)
        .await
        .unwrap();
    assert!(settings.disabled_plugins.contains("web"));
    let reloaded = AgentConfig::new(&paths.config_dir).settings().unwrap();
    assert!(reloaded.disabled_plugins.contains("web"));
    assert_eq!(
        state_of(&service.plugin_status(), "web"),
        AgentPluginState::Disabled
    );
    eventually(|| web_tools() == 0).await;

    assert!(service
        .set_plugin_enabled("agent-kernel".into(), false)
        .await
        .unwrap_err()
        .contains("cannot be disabled"));
    assert!(service
        .set_plugin_enabled("not-a-plugin".into(), false)
        .await
        .unwrap_err()
        .contains("does not exist"));

    let settings = service
        .set_plugin_enabled("web".into(), true)
        .await
        .unwrap();
    assert!(settings.disabled_plugins.is_empty());
    eventually(|| {
        state_of(&service.plugin_status(), "web") == AgentPluginState::Active && web_tools() == 2
    })
    .await;
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A plugin whose first start takes `first_start` (its `apply` does not
/// return until then) and whose later starts are immediate.
fn slow_first_start(name: &'static str, first_start: Duration) -> (PluginDef, Arc<AtomicUsize>) {
    let starts = Arc::new(AtomicUsize::new(0));
    let counted = starts.clone();
    let def = PluginDef::new(move |_, _| {
        let start = counted.fetch_add(1, Ordering::SeqCst);
        async move {
            if start == 0 {
                tokio::time::sleep(first_start).await;
            }
            Ok(())
        }
    })
    .name(name);
    (def, starts)
}

#[tokio::test(start_paused = true)]
async fn a_plugin_that_does_not_start_in_time_fails_and_the_next_one_starts() {
    use super::host::{PLUGIN_START_TIMEOUT, PLUGIN_STOP_BOUND};

    // It starts long after the timeout, so its teardown (which waits for
    // its start) ends then too.
    let (slow, starts) = slow_first_start("slow", PLUGIN_START_TIMEOUT * 2);
    let applied = Arc::new(Mutex::new(Vec::new()));
    let (host, paths, _) = host_with(
        "plugin-host-start-timeout",
        vec![
            AgentPluginRow::unconfigured(Capability, slow),
            AgentPluginRow::unconfigured(Capability, recording_plugin("after", applied.clone())),
        ],
    );
    let started = tokio::time::Instant::now();
    host.ensure_started().await;

    // Start-up waited the timeout for the slow row and no longer.
    let waited = started.elapsed();
    assert!(waited >= PLUGIN_START_TIMEOUT, "{waited:?}");
    assert!(
        waited < PLUGIN_START_TIMEOUT + PLUGIN_STOP_BOUND,
        "{waited:?}"
    );
    let statuses = host.status();
    let slow = status_of(&statuses, "slow");
    assert_eq!(slow.state, AgentPluginState::Failed);
    assert_eq!(slow.error.as_deref(), Some("did not start within 20 s"));
    assert_eq!(state_of(&statuses, "after"), AgentPluginState::Active);
    assert_eq!(applied.lock().unwrap().len(), 1);

    // Its teardown waits for the start kardia never cancels: past the stop
    // bound the row says so, and a retry is refused until it ends.
    tokio::time::sleep(PLUGIN_STOP_BOUND + Duration::from_secs(1)).await;
    let stuck = host.status();
    assert!(status_of(&stuck, "slow")
        .error
        .as_deref()
        .unwrap()
        .ends_with("its teardown did not end within 10 s"));
    assert!(host
        .retry("slow")
        .await
        .unwrap_err()
        .contains("its previous instance has not stopped yet"));

    // Once the late start returned and was torn down, a retry mounts it.
    eventually(|| {
        matches!(
            status_of(&host.status(), "slow").error.as_deref(),
            Some(error) if !error.contains("teardown")
        )
    })
    .await;
    host.retry("slow").await.unwrap();
    assert_eq!(state_of(&host.status(), "slow"), AgentPluginState::Active);
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test(start_paused = true)]
async fn a_reconfiguration_that_does_not_settle_in_time_fails_the_row() {
    // Starts with the setting off; with it on, its start never returns.
    let follows = PluginDef::new(|_, config: Value| async move {
        if config == Value::Bool(true) {
            std::future::pending::<()>().await;
        }
        Ok(())
    })
    .name("follows");
    let (host, paths, config) = host_with(
        "plugin-host-update-timeout",
        vec![AgentPluginRow::from_settings(
            Capability,
            follows,
            |settings| Ok(Value::Bool(settings.agent.web_fetch_use_jina_reader)),
        )],
    );
    host.ensure_started().await;
    assert_eq!(
        state_of(&host.status(), "follows"),
        AgentPluginState::Active
    );

    jina_reader_on(&config);
    let updates = host.reconfigure();
    assert_eq!(updates.len(), 1);
    let started = tokio::time::Instant::now();
    for (_, settle) in updates {
        let error = settle.await.unwrap_err();
        assert_eq!(
            PluginError::from(error).to_string(),
            "did not start within 20 s"
        );
    }
    assert!(started.elapsed() < super::host::PLUGIN_START_TIMEOUT + Duration::from_secs(1));
    let statuses = host.status();
    assert_eq!(state_of(&statuses, "follows"), AgentPluginState::Failed);
    assert_eq!(
        status_of(&statuses, "follows").error.as_deref(),
        Some("did not start within 20 s")
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test(start_paused = true)]
async fn a_teardown_that_never_ends_holds_neither_a_remount_nor_shutdown() {
    let life = Arc::new(ProbeLife::default());
    let counted = life.clone();
    let stuck = PluginDef::new(move |ctx: Context, _| {
        let life = counted.clone();
        async move {
            life.starts.fetch_add(1, Ordering::SeqCst);
            ctx.get::<ObserverRegistry>()?
                .register(Arc::new(ProbeObserver))?;
            ctx.defer("never-stops", Disposer::future(std::future::pending::<()>))?;
            Ok(())
        }
    })
    .name("stuck")
    .inject(Inject::new().with(ObserverRegistry::NAME));
    let (host, paths, config) = host_with(
        "plugin-host-stuck-teardown",
        vec![AgentPluginRow::unconfigured(Capability, stuck)],
    );
    host.ensure_started().await;
    assert_eq!(state_of(&host.status(), "stuck"), AgentPluginState::Active);

    // Turned off: its teardown never ends, which the row reports past the
    // stop bound.
    disable(&config, &["stuck"]);
    assert!(host.reconfigure().is_empty());
    tokio::time::sleep(super::host::PLUGIN_STOP_BOUND + Duration::from_secs(1)).await;
    let off = host.status();
    assert_eq!(state_of(&off, "stuck"), AgentPluginState::Disabled);
    assert_eq!(
        status_of(&off, "stuck").error.as_deref(),
        Some("its teardown did not end within 10 s")
    );

    // Turned on again: the remount does not wait for the stuck teardown; it
    // fails the row instead of mounting a second fiber beside the first.
    disable(&config, &[]);
    let settles = host.reconfigure();
    assert_eq!(settles.len(), 1);
    for (_, settle) in settles {
        assert!(settle.await.is_err());
    }
    let on = host.status();
    assert_eq!(state_of(&on, "stuck"), AgentPluginState::Failed);
    assert!(status_of(&on, "stuck").error.as_deref().unwrap().contains(
        "its previous instance has not stopped yet; \
             its teardown did not end within 10 s"
    ));
    assert_eq!(life.starts(), 1);

    // Shutdown is bounded by its deadline all the same.
    let diagnostics = host.shutdown(Instant::now()).await;
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("did not stop"));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
