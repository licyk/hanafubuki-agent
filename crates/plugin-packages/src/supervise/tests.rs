use std::sync::{mpsc, Arc};
use std::time::Duration;

use kardia_core::{Context, FiberState, PluginDef, StatusChanged};
use kardia_loader::{Catalog, Loader};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::guest_of;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guest_is_recognized_before_the_loader_records_its_fiber() {
    let root = Context::new();
    let catalog = Catalog::new();
    let loader_plugin = root
        .plugin_def(&Loader::plugin(catalog.clone()), Value::Null)
        .unwrap();
    loader_plugin.settled().await.unwrap();
    let loader = (*root.get::<Loader>().unwrap()).clone();
    let observations = Arc::new(Mutex::new(Vec::new()));
    let observed = observations.clone();
    let listener = loader.clone();
    let (active, started) = mpsc::channel();
    let started = Mutex::new(started);
    root.on(move |event: &StatusChanged| {
        if !matches!(event.new, FiberState::Loading | FiberState::Active) {
            return;
        }
        let Some(entry) = listener.entry_of(event.fiber.context()) else {
            return;
        };
        let name = event
            .fiber
            .def()
            .and_then(|def| def.plugin_name().map(str::to_string));
        observed.lock().push((
            name.clone(),
            entry.inner_fiber().is_none(),
            guest_of(&listener, &event.fiber),
            event.new,
        ));
        if name.as_deref() == Some("guest:demo@local") {
            match event.new {
                // Hold plugin_def's synchronous notification until another
                // worker finishes apply, before the loader can store inner.
                FiberState::Loading => tokio::task::block_in_place(|| {
                    started
                        .lock()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap()
                }),
                FiberState::Active => active.send(()).unwrap(),
                _ => unreachable!(),
            }
        }
    })
    .unwrap();
    catalog.register_def(
        "guest",
        PluginDef::new(|ctx, _| async move {
            ctx.plugin_def(&PluginDef::new(|_, _| async { Ok(()) }), Value::Null)?;
            Ok(())
        })
        .name("guest:demo@local"),
    );
    // An injected dependency makes the loader create its own outer fiber.
    root.provide_as("ready", true).unwrap();
    loader
        .create(
            json!({"id": "demo@local", "name": "guest", "inject": ["ready"]}),
            None,
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), loader.settled())
        .await
        .unwrap();
    let observations = observations.lock().clone();
    assert!(
        observations.iter().any(|(name, pending, id, state)| {
            name.as_deref() == Some("guest:demo@local")
                && *pending
                && id.as_deref() == Some("demo@local")
                && *state == FiberState::Active
        }),
        "{observations:?}"
    );
    assert!(
        observations
            .iter()
            .any(|(name, _, id, _)| name.is_none() && id.is_none()),
        "the guest's unnamed child: {observations:?}"
    );
    assert!(
        observations
            .iter()
            .filter(|(name, _, _, _)| name.as_deref() != Some("guest:demo@local"))
            .all(|(_, _, id, _)| id.is_none()),
        "{observations:?}"
    );
    loader_plugin.dispose().await.unwrap();
}
