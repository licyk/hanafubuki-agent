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

/// The `sandbox` section says what the turn's commands may reach and how
/// one may leave the sandbox, which depends on the permission mode.
#[tokio::test]
async fn the_sandbox_section_states_the_sandbox_and_how_to_leave_it() {
    use hanafubuki_agent_kernel::AgentPermissionMode;
    use hanafubuki_agent_sandbox::SandboxPolicy;

    let mounted = mount(permission_mode_plugin()).await;
    let root = std::env::temp_dir().join("sandbox-section-root");
    let workspace_write = SandboxPolicy::WorkspaceWrite {
        writable_roots: vec![root.clone()],
        network_access: false,
    };
    let sandbox = |policy: &SandboxPolicy, mode: AgentPermissionMode| {
        let mut turn = TurnExtensions::default();
        turn.insert(std::sync::Arc::new(policy.clone()));
        let request = ContextRequest {
            permission_mode: mode,
            ..request(turn)
        };
        let context = mounted.context.clone();
        async move {
            let snapshot = hanafubuki_agent_kernel::collect_world_state(&context, &request)
                .await
                .unwrap();
            let section = snapshot
                .sections
                .into_iter()
                .find(|section| section.kind == ContextSectionKind::SANDBOX)
                .expect("a sandbox section");
            serde_json::from_str::<serde_json::Value>(&section.content).unwrap()
        }
    };

    let manual = sandbox(&workspace_write, AgentPermissionMode::Manual).await;
    assert_eq!(manual["mode"], "workspace-write");
    assert_eq!(manual["networkAccess"], false);
    assert_eq!(
        manual["writableRoots"],
        serde_json::json!([root.to_string_lossy()])
    );
    assert_eq!(
        manual["escalation"],
        "A command that needs more than the sandbox allows runs outside it with sandbox_permissions \"require_escalated\" and a justification question; the user approves or denies it."
    );
    let reviewed = sandbox(&workspace_write, AgentPermissionMode::AutoReview).await;
    assert!(reviewed["escalation"]
        .as_str()
        .unwrap()
        .contains("automatic review decides it"));
    let full_access = sandbox(&workspace_write, AgentPermissionMode::FullAccess).await;
    assert!(full_access["escalation"]
        .as_str()
        .unwrap()
        .contains("do not set sandbox_permissions, a command that does is refused"));
    let read_only = sandbox(&SandboxPolicy::ReadOnly, AgentPermissionMode::Manual).await;
    assert_eq!(
        read_only,
        serde_json::json!({
            "mode": "read-only",
            "networkAccess": false,
            "escalation": manual["escalation"],
        })
    );
    let unsandboxed = sandbox(&SandboxPolicy::FullAccess, AgentPermissionMode::Manual).await;
    assert_eq!(
        unsandboxed,
        serde_json::json!({
            "mode": "full-access",
            "networkAccess": true,
            "escalation": "There is no sandbox: commands run with the user's rights. Do not set sandbox_permissions.",
        })
    );
    mounted.fiber.dispose().await.unwrap();
    mounted.root.fiber().dispose().await.unwrap();
}
