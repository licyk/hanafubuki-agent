use std::sync::Arc;

use super::*;
use hanafubuki_agent_contracts::{ToolResult, TurnExtensions};
use hanafubuki_agent_kernel::AgentToolError;
use serde_json::Value;

/// The `skills` plugin over `skills`, with a call helper that hands each call
/// `catalog` as the turn's frozen Skill catalog.
pub(super) struct MountedSkills {
    plugin: MountedToolPlugin,
}

impl MountedSkills {
    pub(super) async fn mount(skills: &AgentSkillApplication) -> Self {
        Self {
            plugin: mount_tool_plugin(
                crate::plugins::skills_plugin(
                    skills.clone(),
                    crate::plugins::tests::fixtures::disconnected_mcp_application(
                        std::env::temp_dir().join("hanafubuki-skills-plugin-no-mcp-settings.json"),
                    ),
                ),
                Arc::new(crate::test_support::TestBlockingOperationPort),
            )
            .await,
        }
    }

    pub(super) async fn call(
        &self,
        name: &str,
        catalog: &hanafubuki_agent_skills::SkillTurnCatalog,
        arguments: Value,
    ) -> Result<ToolResult, String> {
        self.plugin
            .invoke_in_turn(
                name,
                arguments,
                None,
                TurnExtensions::default().with(Arc::new(catalog.clone())),
            )
            .await
            .map_err(|error| match error {
                AgentToolError::Failed(message) => message,
                AgentToolError::Cancelled => "cancelled".to_string(),
            })
    }
}
