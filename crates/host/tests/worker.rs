//! Plugin workers stop with their plugin, and a panicking worker is
//! reported as a fault of its plugin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hanafubuki_agent_host::{
    spawn_scoped_worker, FaultPoint, PluginFault, PluginFaults, WORKER_STOP_BOUND,
};
use kardia_core::{Context, Inject, PluginDef, Service};
use serde_json::Value;

/// A root that provides `faults`, as a core row of a host does (the Agent's
/// kernel plugin).
async fn root_with(faults: Arc<PluginFaults>) -> Context {
    let root = Context::new();
    let host = PluginDef::new(move |ctx: Context, _| {
        let faults = faults.clone();
        async move {
            ctx.provide_arc(faults)?;
            Ok(())
        }
    })
    .provides([PluginFaults::NAME]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    root
}

fn worker_plugin(observed: Arc<AtomicBool>, honours_cancellation: bool) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let observed = observed.clone();
        async move {
            spawn_scoped_worker(&ctx, "test-worker", move |cancellation| async move {
                loop {
                    if honours_cancellation && cancellation.is_cancelled() {
                        observed.store(true, Ordering::Release);
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })?;
            Ok(())
        }
    })
    .name("worker")
    .inject(Inject::new().with(PluginFaults::NAME))
}

#[tokio::test(start_paused = true)]
async fn a_worker_observes_cancellation_when_its_plugin_stops() {
    let root = root_with(Arc::default()).await;
    let observed = Arc::new(AtomicBool::new(false));
    let fiber = root
        .plugin_def(&worker_plugin(observed.clone(), true), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    let started = tokio::time::Instant::now();

    fiber.dispose().await.unwrap();

    assert!(observed.load(Ordering::Acquire));
    assert!(started.elapsed() < WORKER_STOP_BOUND);
    root.fiber().dispose().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_worker_ignoring_cancellation_is_aborted_after_the_bound() {
    let root = root_with(Arc::default()).await;
    let observed = Arc::new(AtomicBool::new(false));
    let fiber = root
        .plugin_def(&worker_plugin(observed.clone(), false), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    let started = tokio::time::Instant::now();

    fiber.dispose().await.unwrap();

    assert!(!observed.load(Ordering::Acquire));
    assert!(started.elapsed() >= WORKER_STOP_BOUND);
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_panicking_worker_is_reported_as_a_fault_of_its_plugin() {
    let faults = Arc::new(PluginFaults::default());
    let reported = Arc::new(Mutex::new(Vec::<PluginFault>::new()));
    let sink = reported.clone();
    faults.set_handler(move |fault| sink.lock().unwrap().push(fault));
    let root = root_with(faults).await;
    let plugin = PluginDef::new(|ctx: Context, _| async move {
        spawn_scoped_worker(&ctx, "doomed-worker", |_cancellation| async {
            panic!("the worker gave up");
        })?;
        Ok(())
    })
    .name("doomed")
    .inject(Inject::new().with(PluginFaults::NAME));
    let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
    fiber.settled().await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while reported.lock().unwrap().is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no fault arrived");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        *reported.lock().unwrap(),
        vec![PluginFault {
            plugin: "doomed".to_string(),
            point: FaultPoint::worker("doomed-worker"),
            message: "the worker gave up".to_string(),
        }]
    );
    // The panic ended only the worker; the plugin stops cleanly.
    fiber.dispose().await.unwrap();
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_plugin_spawning_a_worker_without_the_fault_service_fails() {
    let root = root_with(Arc::default()).await;
    let plugin = PluginDef::new(|ctx: Context, _| async move {
        spawn_scoped_worker(&ctx, "undeclared", |_cancellation| async {})?;
        Ok(())
    })
    .name("undeclared");
    let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
    assert!(fiber.settled().await.is_err());
    root.fiber().dispose().await.unwrap();
}
