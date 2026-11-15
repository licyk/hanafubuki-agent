//! Names-only discovery over real MCP and model transports, including
//! Responses' native search path and both function-search fallbacks.

use super::*;
use serde_json::Value;

#[tokio::test]
async fn listing_names_is_visible_without_admitting_mcp_tools_on_every_protocol() {
    for protocol in [
        AgentModelProtocol::Responses,
        AgentModelProtocol::ChatCompletions,
        AgentModelProtocol::AnthropicMessages,
    ] {
        let mcp = ScriptedMcpServers::start(BTreeMap::from([(
            "fixture",
            std::iter::once(scripted_tool("echo", "SCHEMA_ONLY_DESCRIPTION"))
                .chain((0..206).map(|index| {
                    scripted_tool(&format!("listed_{index:03}"), "SCHEMA_ONLY_DESCRIPTION")
                }))
                .collect(),
        )]))
        .await;
        let list_arguments =
            json!({"source":"mcp.fixture", "offset":"0", "limit":"999", "future_field":true});
        let search = if protocol == AgentModelProtocol::Responses {
            tool_search_events("mcp__fixture__echo")
        } else {
            call(
                protocol,
                "search",
                "tool_search",
                json!({"query":"mcp__fixture__echo"}),
            )
        };
        let answer = match protocol {
            AgentModelProtocol::Responses => assistant_events("Listed, discovered and called."),
            AgentModelProtocol::ChatCompletions => chat_reasoning_answer_events(
                "reasoning_content",
                "",
                "Listed, discovered and called.",
            ),
            AgentModelProtocol::AnthropicMessages => {
                anthropic_assistant_events("Listed, discovered and called.")
            }
        };
        let model = FakeServer::start(vec![
            (
                call(protocol, "list", "tool_list", list_arguments),
                Duration::ZERO,
            ),
            (
                call(
                    protocol,
                    "denied",
                    "mcp__fixture__echo",
                    json!({"value":"42"}),
                ),
                Duration::ZERO,
            ),
            (search, Duration::ZERO),
            (
                call(
                    protocol,
                    "invoke",
                    "mcp__fixture__echo",
                    json!({"value":"42"}),
                ),
                Duration::ZERO,
            ),
            (answer, Duration::ZERO),
        ])
        .await;
        let (service, paths) = service_with("tool-list-protocol", |_, config| {
            configure_test_provider(config, &model.base_url, None);
            config
                .mutate_providers(1, |registry| {
                    registry.providers[0].protocol = protocol;
                    Ok(())
                })
                .unwrap();
        });
        service
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: service.mcp_snapshot().await.unwrap().settings.revision,
                server: mcp.config("fixture", McpExposurePolicy::Deferred),
            })
            .await
            .unwrap();
        let session = service
            .create_session(AgentSessionMode::Default, None)
            .await
            .unwrap();
        let outcome = service
            .start_turn(
                session.id,
                "Browse, discover and use the echo tool.".to_string().into(),
            )
            .await
            .unwrap();
        assert_eq!(outcome.final_answer, "Listed, discovered and called.");
        let requests = model.requests();
        assert_eq!(requests.len(), 5, "{protocol:?}");
        let bodies = requests
            .iter()
            .map(|request| serde_json::from_slice::<Value>(&request.body).unwrap())
            .collect::<Vec<_>>();
        let listed = output(protocol, &bodies[1], "list");
        let listed_text = match &listed {
            Value::String(text) => text.as_str(),
            Value::Array(content) => content[0]["text"].as_str().unwrap(),
            _ => panic!("unexpected output: {listed}"),
        };
        // Notes can follow the JSON in a plain-text protocol output.
        let page: Value = serde_json::Deserializer::from_str(listed_text)
            .into_iter::<Value>()
            .next()
            .unwrap()
            .unwrap();
        // Chat Completions preserves the structured tool-result envelope.
        let page = page.get("structuredContent").unwrap_or(&page);
        let expected_names = std::iter::once("mcp__fixture__echo".to_string())
            .chain((0..49).map(|index| format!("mcp__fixture__listed_{index:03}")))
            .collect::<Vec<_>>();
        assert_eq!(
            page,
            &json!({"tools":expected_names,"total":207,"next_offset":50}),
            "{protocol:?}"
        );
        assert!(
            listed.to_string().contains("999"),
            "the clamped limit is reported: {listed}"
        );
        assert!(!bodies[1].to_string().contains("SCHEMA_ONLY_DESCRIPTION"));
        assert!(output(protocol, &bodies[2], "denied")
            .to_string()
            .contains("tool requires discovery before use"));
        assert!(bodies[3].to_string().contains("SCHEMA_ONLY_DESCRIPTION"));
        assert!(output(protocol, &bodies[4], "invoke")
            .to_string()
            .contains("echo called with"));
        let deadline = Instant::now() + TEST_SHUTDOWN_DEADLINE;
        let stopping = service.clone();
        assert_eq!(
            tokio::task::spawn_blocking(move || {
                stopping.stop_accepting_and_cancel_turns(deadline)
            })
            .await
            .unwrap(),
            []
        );
        service.shutdown_mcp(deadline).await.unwrap();
        assert_eq!(service.shutdown_plugins(deadline).await, []);
        assert_eq!(
            tokio::task::spawn_blocking(move || service.shutdown_workers_after_mcp(deadline))
                .await
                .unwrap(),
            []
        );
        std::fs::remove_dir_all(paths.app_data_dir).unwrap();
    }
}

fn call(protocol: AgentModelProtocol, id: &str, name: &str, arguments: Value) -> Vec<u8> {
    match protocol {
        AgentModelProtocol::Responses => {
            let events = if name == "mcp__fixture__echo" {
                namespace_tool_events("mcp__fixture", "echo", &arguments.to_string())
            } else {
                tool_events(name, &arguments.to_string())
            };
            String::from_utf8(events).unwrap().replace("call-tool", id).replace("mcp-call-1", id).into_bytes()
        }
        AgentModelProtocol::ChatCompletions => format!("data: {}\n\ndata: [DONE]\n\n", json!({"id":format!("response-{id}"),"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]})).into_bytes(),
        AgentModelProtocol::AnthropicMessages => String::from_utf8(anthropic_tool_events(name, &arguments.to_string())).unwrap().replace("toolu-1", id).into_bytes(),
    }
}

fn output(protocol: AgentModelProtocol, request: &Value, id: &str) -> Value {
    match protocol {
        AgentModelProtocol::Responses => request["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call_output" && item["call_id"] == id)
            .unwrap()["output"]
            .clone(),
        AgentModelProtocol::ChatCompletions => request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["role"] == "tool" && item["tool_call_id"] == id)
            .unwrap()["content"]
            .clone(),
        AgentModelProtocol::AnthropicMessages => request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["content"].as_array())
            .flatten()
            .find(|item| item["type"] == "tool_result" && item["tool_use_id"] == id)
            .unwrap()["content"]
            .clone(),
    }
}
