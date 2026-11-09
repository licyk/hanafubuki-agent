use crate::AgentConfig;
use hanafubuki_agent_approval::ApprovalService;
use hanafubuki_agent_kernel::{AgentKernelRegistries, AgentService, EphemeralAgentHistoryStore};

use super::*;
use crate::test_support::mount;

fn conversations() -> AgentConversationReader {
    let core = AgentService::without_default_model_with_history_store_and_approval(
        AgentKernelRegistries::default(),
        Arc::new(EphemeralAgentHistoryStore::default()),
        ApprovalService::manual(),
    );
    AgentConversationReader::new(
        core,
        AgentConfig::new(&std::env::temp_dir().join("hanafubuki-agent-recap-plugin")),
    )
}

/// The plugin provides its service under its name while it runs, and
/// stopping it withdraws the service.
#[tokio::test]
async fn the_recap_plugin_provides_its_service_until_it_stops() {
    let mounted = mount(recap_plugin(conversations())).await;
    assert!(mounted.root.live_as::<RecapService>(RECAP_PLUGIN).is_some());

    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.root.live_as::<RecapService>(RECAP_PLUGIN).is_none());
    mounted.root.fiber().dispose().await.unwrap();
}
