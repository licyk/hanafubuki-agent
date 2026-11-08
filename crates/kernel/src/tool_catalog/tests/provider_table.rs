//! The request's wire names: native namespaces where the protocol takes
//! them, flattened names fitted to its limit otherwise, and calls resolved
//! by lookup only.

use super::*;

fn mcp_tool(server: &str, name: &str) -> AgentPlannedTool {
    let namespace = vec!["mcp".to_string(), server.to_string()];
    plan_sourced(
        sourced_tool(
            &format!("mcp.{server}"),
            &format!("{server}.{}", &name[..name.len().min(16)]),
            [7; 32],
            descriptor(
                ToolName::namespaced(namespace, name).unwrap(),
                "an MCP tool",
                ToolExposure::Direct,
            ),
        ),
        ToolAvailability::Function,
    )
}

async fn plan_for(
    protocol: hanafubuki_agent_model_api::ModelApiProtocol,
    tools: Vec<AgentPlannedTool>,
) -> AgentToolPlan {
    let mut context = plan_context(AgentSessionMode::Default, false);
    context.provider_capabilities = protocol.tool_capabilities();
    AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: tools,
    })])
    .build(context, CancellationToken::default())
    .await
    .unwrap()
}

/// `(namespace, name)` of each planned tool's wire spec, by its own name.
fn wire_name(plan: &AgentToolPlan, model_name: &str) -> (Option<String>, String) {
    let tool = plan
        .registrations()
        .iter()
        .find(|tool| tool.model_spec.name == model_name)
        .unwrap_or_else(|| panic!("{model_name} is not planned"));
    match &tool.provider_spec {
        ToolSpec::Namespace(namespace) => (
            Some(namespace.name.local_name().to_string()),
            namespace.tools[0].name.local_name().to_string(),
        ),
        ToolSpec::Function(function) => (None, function.name.local_name().to_string()),
        other => panic!("unexpected spec {other:?}"),
    }
}

fn is_identity_suffix(name: &str) -> bool {
    let Some((_, suffix)) = name.rsplit_once('_') else {
        return false;
    };
    suffix.len() == 12 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn call(namespace: Option<&str>, name: &str) -> hanafubuki_agent_model_api::ModelToolCall {
    hanafubuki_agent_model_api::ModelToolCall {
        item_id: "item".into(),
        call_id: "call".into(),
        namespace: namespace.map(str::to_string),
        name: name.into(),
        arguments: json!({}),
        argument_error: None,
    }
}

/// A namespaced tool keeps its namespace on Responses and is flattened on
/// the protocols without one; a call resolves only by the name the request
/// carried, never by joining or splitting.
#[tokio::test]
async fn a_call_resolves_by_the_wire_name_of_its_protocol_only() {
    use hanafubuki_agent_model_api::ModelApiProtocol::{
        AnthropicMessages, ChatCompletions, Responses,
    };
    let responses = plan_for(Responses, vec![mcp_tool("fixture", "echo")]).await;
    assert_eq!(
        wire_name(&responses, "mcp__fixture__echo"),
        (Some("mcp__fixture".to_string()), "echo".to_string())
    );
    let echo = responses.resolve_call(&call(Some("mcp__fixture"), "echo"));
    assert_eq!(echo.unwrap().tool_id.as_str(), "fixture.echo");
    assert!(responses
        .resolve_call(&call(None, "mcp__fixture__echo"))
        .is_err());
    assert!(responses.resolve_call(&call(None, "echo")).is_err());
    assert!(responses
        .resolve_call(&call(Some("mcp__other"), "echo"))
        .is_err());
    assert!(responses
        .unselected_deferred_call(&call(Some("mcp__other"), "echo"))
        .is_none());

    for protocol in [ChatCompletions, AnthropicMessages] {
        let flat = plan_for(protocol, vec![mcp_tool("fixture", "echo")]).await;
        assert_eq!(
            wire_name(&flat, "mcp__fixture__echo"),
            (None, "mcp__fixture__echo".to_string())
        );
        assert!(flat.resolve_call(&call(None, "mcp__fixture__echo")).is_ok());
        assert!(flat
            .resolve_call(&call(Some("mcp__fixture"), "echo"))
            .is_err());
    }
}

/// A name over the protocol's limit is cut and ends with a hash of the
/// tool's identity: the same on every plan, within the limit, and not
/// reported, since nothing collided.
#[tokio::test]
async fn a_long_name_is_fitted_to_the_protocol_limit() {
    use hanafubuki_agent_model_api::ModelApiProtocol::{ChatCompletions, Responses};
    let long = format!("long_{}", "x".repeat(59));
    let longer = "y".repeat(120);
    let tools = || vec![mcp_tool("fixture", &long), mcp_tool("fixture", &longer)];

    let responses = plan_for(Responses, tools()).await;
    assert_eq!(
        wire_name(&responses, &format!("mcp__fixture__{long}")),
        (Some("mcp__fixture".to_string()), long.clone())
    );
    let (namespace, fitted) = wire_name(&responses, &format!("mcp__fixture__{longer}"));
    assert_eq!(namespace.as_deref(), Some("mcp__fixture"));
    assert_eq!("mcp__fixture__".len() + fitted.len(), 128);
    assert!(
        fitted.starts_with("yyyy") && is_identity_suffix(&fitted),
        "{fitted}"
    );
    assert!(responses
        .resolve_call(&call(Some("mcp__fixture"), &fitted))
        .is_ok());

    let chat = plan_for(ChatCompletions, tools()).await;
    for model_name in [
        format!("mcp__fixture__{long}"),
        format!("mcp__fixture__{longer}"),
    ] {
        let (namespace, fitted) = wire_name(&chat, &model_name);
        assert_eq!(namespace, None);
        assert_eq!(fitted.len(), 64);
        assert!(fitted.starts_with("mcp__fixture__") && is_identity_suffix(&fitted));
        assert!(chat.resolve_call(&call(None, &fitted)).is_ok());
        let again = plan_for(ChatCompletions, tools()).await;
        assert_eq!(wire_name(&again, &model_name), (None, fitted));
    }
    assert!(responses.omissions().is_empty());
    assert!(chat.omissions().is_empty());
}

/// Two tools whose names flatten alike: the first in planning order keeps
/// its name, the other ends with its identity hash and is reported once in
/// the plan's omissions, whatever order the source listed them in.
#[tokio::test]
async fn a_flattened_collision_is_suffixed_deterministically_and_reported() {
    use hanafubuki_agent_model_api::ModelApiProtocol::{ChatCompletions, Responses};
    let tools = |reversed: bool| {
        let mut tools = vec![mcp_tool("fixture", "b__c"), mcp_tool("fixture__b", "c")];
        if reversed {
            tools.reverse();
        }
        tools
    };
    for protocol in [Responses, ChatCompletions] {
        let plan = plan_for(protocol, tools(false)).await;
        let kept = plan
            .registrations()
            .iter()
            .find(|tool| tool.provider_id.as_str() == "mcp.fixture")
            .unwrap();
        let renamed = plan
            .registrations()
            .iter()
            .find(|tool| tool.provider_id.as_str() == "mcp.fixture__b")
            .unwrap();
        let wire = |tool: &AgentPlannedTool| match &tool.provider_spec {
            ToolSpec::Namespace(namespace) => (
                Some(namespace.name.local_name().to_string()),
                namespace.tools[0].name.local_name().to_string(),
            ),
            ToolSpec::Function(function) => (None, function.name.local_name().to_string()),
            _ => unreachable!(),
        };
        let (kept_namespace, kept_name) = wire(kept);
        let (renamed_namespace, renamed_name) = wire(renamed);
        match protocol {
            Responses => {
                assert_eq!(kept_namespace.as_deref(), Some("mcp__fixture"));
                assert_eq!(kept_name, "b__c");
                assert_eq!(renamed_namespace.as_deref(), Some("mcp__fixture__b"));
                assert!(renamed_name.starts_with("c_"));
            }
            _ => {
                assert_eq!(
                    (kept_namespace, kept_name.as_str()),
                    (None, "mcp__fixture__b__c")
                );
                assert_eq!(renamed_namespace, None);
                assert!(renamed_name.starts_with("mcp__fixture__b__c_"));
            }
        }
        assert!(is_identity_suffix(&renamed_name));
        assert_eq!(plan.omissions().len(), 1);
        assert_eq!(plan.omissions()[0].contributor, "mcp.fixture__b");
        assert!(plan.omissions()[0].reason.starts_with("renamed to `"));
        let specs = |plan: &AgentToolPlan| {
            plan.registrations()
                .iter()
                .map(|tool| tool.provider_spec.clone())
                .collect::<Vec<_>>()
        };
        let reversed = plan_for(protocol, tools(true)).await;
        assert_eq!(specs(&reversed), specs(&plan));
    }
}

/// A namespace belongs to the provider that first uses it; another
/// provider's tool in it goes on the wire flattened, so the two never merge
/// into one namespace.
#[tokio::test]
async fn a_namespace_has_one_provider() {
    let mut intruder = mcp_tool("fixture", "other");
    intruder.provider_id = AgentToolProviderId::parse("zz-other").unwrap();
    let plan = plan_for(
        hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        vec![mcp_tool("fixture", "echo"), intruder],
    )
    .await;
    assert_eq!(
        wire_name(&plan, "mcp__fixture__echo"),
        (Some("mcp__fixture".to_string()), "echo".to_string())
    );
    assert_eq!(
        wire_name(&plan, "mcp__fixture__other"),
        (None, "mcp__fixture__other".to_string())
    );
}
