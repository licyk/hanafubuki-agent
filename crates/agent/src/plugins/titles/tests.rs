use std::time::{Duration, Instant};

use crate::AgentConfig;
use hanafubuki_agent_approval::ApprovalService;
use hanafubuki_agent_contracts::{
    AgentGeneration, AgentNotification, AgentObserver, RegistryService,
};
use hanafubuki_agent_kernel::{AgentKernelRegistries, AgentService, EphemeralAgentHistoryStore};

use super::*;
use crate::test_support::mount;

fn titles() -> AgentTitleApplication {
    let core = AgentService::without_default_model_with_history_store_and_approval(
        AgentKernelRegistries::default(),
        Arc::new(EphemeralAgentHistoryStore::default()),
        ApprovalService::manual(),
    );
    AgentTitleApplication::new(
        core,
        AgentConfig::new(&std::env::temp_dir().join("hanafubuki-agent-titles-plugin")),
    )
}

fn admitted(origin: AgentTurnOrigin, automatic_title: Option<&str>) -> AgentNotification {
    AgentNotification::TurnAdmitted {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(1),
        generation: AgentGeneration::new(1),
        origin,
        automatic_title: automatic_title.map(str::to_string),
    }
}

/// The plugin registers its observer, and stopping it removes the observer
/// and stops the worker well within the worker bound.
#[tokio::test]
async fn the_titles_plugin_registers_its_observer_until_it_stops() {
    let mounted = mount(titles_plugin(titles())).await;
    let owners = |mounted: &crate::test_support::MountedPlugin| {
        mounted
            .observers
            .registry()
            .snapshot()
            .entries()
            .iter()
            .map(|entry| (entry.owner().to_string(), entry.item().id().to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        owners(&mounted),
        [("titles".to_string(), "titles".to_string())]
    );

    let started = Instant::now();
    mounted.fiber.dispose().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(owners(&mounted).is_empty());
    mounted.root.fiber().dispose().await.unwrap();
}

/// Only a user's turn of a session whose title the Agent gave is queued.
#[test]
fn only_a_user_turn_with_an_automatic_title_is_queued() {
    let (sender, mut receiver) = mpsc::channel(TITLE_QUEUE_CAPACITY);
    let observer = TitleObserver { sender };

    observer.observe(&admitted(AgentTurnOrigin::Programmatic, Some("Title")));
    observer.observe(&admitted(AgentTurnOrigin::User, None));
    assert!(receiver.try_recv().is_err());

    observer.observe(&admitted(AgentTurnOrigin::User, Some("Title")));
    let work = receiver.try_recv().unwrap();
    assert_eq!(work.title, "Title");
    assert_eq!(work.turn_id, AgentTurnId::new(1));
    assert!(receiver.try_recv().is_err());
}

/// A full queue drops the work instead of waiting: the next user turn
/// tries again.
#[test]
fn a_full_queue_drops_the_title_instead_of_waiting() {
    let (sender, mut receiver) = mpsc::channel(1);
    let observer = TitleObserver { sender };

    observer.observe(&admitted(AgentTurnOrigin::User, Some("First")));
    observer.observe(&admitted(AgentTurnOrigin::User, Some("Second")));

    assert_eq!(receiver.try_recv().unwrap().title, "First");
    assert!(receiver.try_recv().is_err());
}
