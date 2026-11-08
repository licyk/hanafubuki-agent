//! Observers receive every notification while their plugin runs, one entry
//! per observer id, and none once their plugin has stopped.

use std::sync::{Arc, Mutex};

use hanafubuki_agent_contracts::{
    AgentGeneration, AgentNotification, AgentObserver, AgentSessionId, AgentTurnId,
    ObserverRegistry, Register, RegistrationError, RegistryService,
};
use kardia_core::{Context, Fiber, FiberState, Inject, PluginDef, Service};
use serde_json::Value;

#[derive(Default)]
struct Recording {
    id: &'static str,
    seen: Mutex<Vec<AgentNotification>>,
}

impl AgentObserver for Recording {
    fn id(&self) -> &str {
        self.id
    }

    fn observe(&self, notification: &AgentNotification) {
        self.seen.lock().unwrap().push(notification.clone());
    }
}

fn finished(turn: u64) -> AgentNotification {
    AgentNotification::TurnFinished {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(turn),
        generation: AgentGeneration::new(turn),
    }
}

async fn registry_root() -> (Context, Arc<ObserverRegistry>) {
    let root = Context::new();
    root.plugin_def(
        &PluginDef::new(|ctx: Context, _| async move {
            ctx.provide(ObserverRegistry::default())?;
            Ok(())
        })
        .provides([ObserverRegistry::NAME]),
        Value::Null,
    )
    .unwrap()
    .settled()
    .await
    .unwrap();
    let registry = root.get::<ObserverRegistry>().unwrap().arc().clone();
    (root, registry)
}

async fn observing(root: &Context, observer: Arc<Recording>) -> Fiber {
    let name = observer.id;
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let observer = observer.clone();
        async move {
            let observers = ctx.get::<ObserverRegistry>()?;
            observers.register(observer as Arc<dyn AgentObserver>)?;
            Ok(())
        }
    })
    .name(name)
    .inject(Inject::new().with(ObserverRegistry::NAME));
    let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    fiber
}

#[tokio::test]
async fn every_running_observer_sees_each_notification_in_order() {
    let (root, registry) = registry_root().await;
    let first = Arc::new(Recording {
        id: "first",
        ..Recording::default()
    });
    let second = Arc::new(Recording {
        id: "second",
        ..Recording::default()
    });
    observing(&root, first.clone()).await;
    observing(&root, second.clone()).await;

    registry.notify(&finished(1));
    registry.notify(&finished(2));

    for observer in [&first, &second] {
        assert_eq!(*observer.seen.lock().unwrap(), [finished(1), finished(2)]);
    }
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_stopped_plugin_is_no_longer_notified() {
    let (root, registry) = registry_root().await;
    let stopping = Arc::new(Recording {
        id: "stopping",
        ..Recording::default()
    });
    let staying = Arc::new(Recording {
        id: "staying",
        ..Recording::default()
    });
    let fiber = observing(&root, stopping.clone()).await;
    observing(&root, staying.clone()).await;
    registry.notify(&finished(1));

    fiber.dispose().await.unwrap();
    registry.notify(&finished(2));

    assert_eq!(*stopping.seen.lock().unwrap(), [finished(1)]);
    assert_eq!(*staying.seen.lock().unwrap(), [finished(1), finished(2)]);
    root.fiber().dispose().await.unwrap();
    assert!(registry.registry().snapshot().is_empty());
}

#[test]
fn the_registry_admits_one_observer_per_id() {
    let existing: Arc<dyn AgentObserver> = Arc::new(Recording {
        id: "memory",
        ..Recording::default()
    });
    let other: Arc<dyn AgentObserver> = Arc::new(Recording {
        id: "titles",
        ..Recording::default()
    });
    let same: Arc<dyn AgentObserver> = Arc::new(Recording {
        id: "memory",
        ..Recording::default()
    });
    assert!(ObserverRegistry::admit(&[&existing], &other).is_ok());
    assert!(matches!(
        ObserverRegistry::admit(&[&existing], &same),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "memory"
    ));
}
