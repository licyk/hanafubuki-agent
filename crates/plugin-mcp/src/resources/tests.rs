use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentToolError, AgentToolHandler, NoopAgentToolProgressSink, ToolCallContext,
    ToolCallValidationError, ToolInvocation, ToolPlanLease,
};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_mcp::McpManager;
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::{json, Value};

use super::{parse, McpResourceCall, McpResourceTool, McpResourceToolKind};

fn manager() -> Arc<McpManager> {
    Arc::new(McpManager::new(
        Arc::new(ExecutorManager::default()),
        BackgroundWorkerGroup::default(),
    ))
}

fn invocation(arguments: Value, plan_leases: Vec<Arc<dyn ToolPlanLease>>) -> ToolInvocation {
    ToolInvocation {
        context: ToolCallContext {
            session_id: serde_json::from_value(json!(1)).unwrap(),
            turn_id: serde_json::from_value(json!(1)).unwrap(),
            generation: serde_json::from_value(json!(1)).unwrap(),
            mode: hanafubuki_agent_contracts::AgentSessionMode::Default,
            session_revision: serde_json::from_value(json!(1)).unwrap(),
            call_id: "mcp-resource-call".to_string(),
            image_input: false,
            origin: Default::default(),
        },
        arguments,
        cancellation: CancellationToken::default(),
        progress: Arc::new(NoopAgentToolProgressSink),
        turn: Default::default(),
        plan_leases,
    }
}

#[test]
fn resource_lists_use_protocol_neutral_optional_server_and_cursor() {
    for kind in [
        McpResourceToolKind::ListResources,
        McpResourceToolKind::ListResourceTemplates,
    ] {
        assert!(parse(kind, &json!({})).is_ok());
        assert!(parse(kind, &json!({"server": "files", "cursor": "next"})).is_ok());
        // An unknown key is ignored: `serverId` names no server.
        assert_eq!(
            parse(kind, &json!({"serverId": "files"})).unwrap(),
            parse(kind, &json!({})).unwrap()
        );
    }
    assert_eq!(
        parse(McpResourceToolKind::ListResources, &json!({"server": null})).unwrap(),
        McpResourceCall::ListResources {
            server: None,
            cursor: None
        }
    );
    assert_eq!(
        parse(
            McpResourceToolKind::ReadResource,
            &json!({"server": "files"})
        ),
        Err(ToolCallValidationError::MissingArgument("uri"))
    );
    assert_eq!(
        parse(McpResourceToolKind::ReadResource, &json!("files")),
        Err(ToolCallValidationError::ArgumentsMustBeObject)
    );
}

#[tokio::test]
async fn resource_tools_read_only_through_the_step_binding_among_the_plan_leases() {
    let list = McpResourceTool::new(McpResourceToolKind::ListResources).unwrap();
    let read = McpResourceTool::new(McpResourceToolKind::ReadResource).unwrap();

    assert_eq!(
        list.invoke(invocation(json!({}), Vec::new()), None)
            .await
            .unwrap_err(),
        AgentToolError::Failed("MCP step binding is unavailable".to_string())
    );

    let leases = || -> Vec<Arc<dyn ToolPlanLease>> {
        vec![
            Arc::new("another source's lease".to_string()),
            Arc::new(manager().step_binding()),
        ]
    };
    let listed = list
        .invoke(invocation(json!({}), leases()), None)
        .await
        .unwrap();
    assert_eq!(
        listed.output.structured_content,
        Some(json!({
            "resources": [],
            "truncated": false,
            "omittedItems": 0,
            "failures": [],
            "warning": null,
        }))
    );
    assert_eq!(
        list.invoke(invocation(json!({"cursor": "next"}), leases()), None)
            .await
            .unwrap_err(),
        AgentToolError::Failed("an MCP resource cursor requires an exact server".to_string())
    );
    assert_eq!(
        read.invoke(
            invocation(json!({"server": "files", "uri": "file:///a"}), leases()),
            None
        )
        .await
        .unwrap_err(),
        AgentToolError::Failed("MCP server is unavailable in this sampling step".to_string())
    );
}
