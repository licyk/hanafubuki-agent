//! Registration lifecycle: entries belong to the registering plugin, and a
//! stopping plugin waits for the calls that still use its entries.

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_host::{
    Register, RegistrationError, Registry, RegistryService, DRAIN_GRACE, RETIRE_CANCEL_BOUND,
};
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};
use kardia_macros::Service;
use serde_json::Value;

#[derive(Service, Default)]
#[service(name = "test-items")]
struct Items {
    registry: Registry<String>,
}

impl RegistryService for Items {
    type Item = String;

    fn registry(&self) -> &Registry<String> {
        &self.registry
    }

    fn admit(existing: &[&String], candidate: &String) -> Result<(), RegistrationError> {
        if existing.contains(&candidate) {
            return Err(RegistrationError::Duplicate {
                registry: Self::NAME,
                identity: candidate.clone(),
            });
        }
        Ok(())
    }
}

fn items_plugin() -> PluginDef {
    PluginDef::new(|ctx: Context, _| async move {
        ctx.provide(Items::default())?;
        Ok(())
    })
    .name("items")
    .provides([Items::NAME])
}

fn contributor(name: &'static str, items: &'static [&'static str]) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| async move {
        let registry = ctx.get::<Items>()?;
        for item in items {
            registry.register((*item).to_string())?;
        }
        Ok(())
    })
    .name(name)
    .inject(Inject::new().with(Items::NAME))
}

async fn root_with_items() -> (Context, Arc<Items>) {
    let root = Context::new();
    root.plugin_def(&items_plugin(), Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let items = root.get::<Items>().unwrap().arc().clone();
    (root, items)
}

/// Sorted: plugins start concurrently, so the registration order across
/// plugins is not defined. Consumers impose their own order.
fn names(items: &Items) -> Vec<String> {
    let mut names = items
        .registry
        .snapshot()
        .items()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[tokio::test]
async fn registrations_belong_to_the_registering_plugin() {
    let (root, items) = root_with_items().await;
    let first = root
        .plugin_def(&contributor("first", &["a", "b"]), Value::Null)
        .unwrap();
    let second = root
        .plugin_def(&contributor("second", &["c"]), Value::Null)
        .unwrap();
    first.settled().await.unwrap();
    second.settled().await.unwrap();
    assert_eq!(names(&items), ["a", "b", "c"]);
    let snapshot = items.registry.snapshot();
    let owner_of_c = snapshot
        .entries()
        .iter()
        .find(|entry| entry.item() == "c")
        .unwrap()
        .owner();
    assert_eq!(owner_of_c, "second");

    first.dispose().await.unwrap();
    assert_eq!(names(&items), ["c"]);
    assert!(items.registry.snapshot().revision() > snapshot.revision());

    root.fiber().dispose().await.unwrap();
    assert!(names(&items).is_empty());
}

#[tokio::test]
async fn a_refused_registration_fails_its_plugin_and_keeps_the_rest() {
    let (root, items) = root_with_items().await;
    let first = root
        .plugin_def(&contributor("first", &["a"]), Value::Null)
        .unwrap();
    first.settled().await.unwrap();
    let clash = root
        .plugin_def(&contributor("clash", &["b", "a"]), Value::Null)
        .unwrap();
    let _ = clash.settled().await;

    assert_eq!(clash.state(), FiberState::Failed);
    assert!(clash
        .error()
        .unwrap()
        .to_string()
        .contains("already contains a"));
    // The failed plugin's partial registration ("b") is reverted with it.
    assert_eq!(names(&items), ["a"]);
    root.fiber().dispose().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stopping_plugin_waits_for_leased_calls_to_finish() {
    let (root, items) = root_with_items().await;
    let plugin = root
        .plugin_def(&contributor("tools", &["a"]), Value::Null)
        .unwrap();
    plugin.settled().await.unwrap();
    let snapshot = items.registry.snapshot();
    let call = CancellationToken::default();
    let lease = snapshot.entries()[0].acquire(&call).unwrap();

    let disposal = tokio::spawn(async move { plugin.dispose().await });
    tokio::time::sleep(DRAIN_GRACE / 2).await;
    assert!(!disposal.is_finished(), "retirement waits for the lease");
    assert!(snapshot.entries()[0].is_retired());
    assert!(snapshot.entries()[0].acquire(&call).is_none());
    assert!(names(&items).is_empty(), "new snapshots omit the entry");

    drop(lease);
    disposal.await.unwrap().unwrap();
    assert!(
        !call.is_cancelled(),
        "a call that finished in time is not cancelled"
    );
    root.fiber().dispose().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_call_outliving_the_grace_period_is_cancelled_then_abandoned() {
    let (root, items) = root_with_items().await;
    let plugin = root
        .plugin_def(&contributor("tools", &["a"]), Value::Null)
        .unwrap();
    plugin.settled().await.unwrap();
    let call = CancellationToken::default();
    let lease = items.registry.snapshot().entries()[0]
        .acquire(&call)
        .unwrap();
    let started = tokio::time::Instant::now();

    plugin.dispose().await.unwrap();

    assert!(call.is_cancelled());
    assert!(started.elapsed() >= DRAIN_GRACE + RETIRE_CANCEL_BOUND - Duration::from_millis(1));
    drop(lease);
    root.fiber().dispose().await.unwrap();
}
