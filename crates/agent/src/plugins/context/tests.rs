//! The permission-mode contributor registers while its plugin runs and
//! produces its section from the turn's request. The workspace contributor
//! is exercised end to end through the application's world-state tests,
//! which need configured application paths.

use hanafubuki_agent_contracts::{
    ContextRequest, ContextSectionKind, RegistryService, TurnExtensions,
};

use super::*;
use crate::test_support::mount;

fn request(turn: TurnExtensions) -> ContextRequest {
    ContextRequest {
        session_id: hanafubuki_agent_kernel::AgentSessionId::new(1),
        mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
        permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::FullAccess,
        user_message: None,
        turn,
    }
}

#[tokio::test]
async fn permission_mode_renders_the_turns_mode_while_its_plugin_runs() {
    let mounted = mount(permission_mode_plugin()).await;

    let snapshot = hanafubuki_agent_kernel::collect_world_state(
        &mounted.context,
        &request(TurnExtensions::default()),
    )
    .await
    .unwrap();

    assert_eq!(snapshot.sections.len(), 1);
    assert_eq!(
        snapshot.sections[0].kind,
        ContextSectionKind::PERMISSION_MODE
    );
    assert_eq!(snapshot.sections[0].content, "{\"mode\":\"full_access\"}");
    assert!(mounted
        .renderers
        .snapshot()
        .find(&ContextSectionKind::PERMISSION_MODE)
        .is_some());
    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.context.registry().snapshot().is_empty());
    assert!(mounted.renderers.registry().snapshot().is_empty());
    mounted.root.fiber().dispose().await.unwrap();
}
