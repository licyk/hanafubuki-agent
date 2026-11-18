use super::*;

#[tokio::test]
async fn input_and_output_budget_pressure_are_not_context_pressure() {
    for window in [None, Some(1_000_000)] {
        for output_pressure in [false, true] {
            let ScriptedResponse::Events(mut events) = tool_response_with_usage(90) else {
                unreachable!()
            };
            if output_pressure {
                for event in &mut events {
                    if let Ok(ModelEvent::ResponseCompleted {
                        usage: Some(usage), ..
                    }) = event
                    {
                        usage.input_tokens = 10;
                        usage.output_tokens = 90;
                    }
                }
            }
            let model = ScriptedModel::new([
                ScriptedResponse::Events(events),
                response("answer", "Done", 2),
            ]);
            let tools = FakeTools::new([FakeRegisteredExchange::success(
                "list_instances",
                json!({}),
                json!({"instances": []}),
            )])
            .await;
            let service = AgentService::new(Arc::new(model.clone()), tools.registries());
            let session = service
                .create_session(AgentSessionConfig {
                    context_window_tokens: window,
                    ..config()
                })
                .unwrap();
            service
                .set_session_turn_budget_policy(
                    session.id,
                    AgentTurnBudgetPolicy {
                        maximum_cumulative_model_input_tokens: Some(100),
                        maximum_cumulative_model_output_tokens: Some(100),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(
                service
                    .start_turn(session.id, "List instances")
                    .await
                    .unwrap()
                    .final_answer,
                "Done"
            );
            assert!(service.snapshot(session.id).unwrap().compaction.is_none());
            assert_eq!(model.requests().len(), 2);
        }
    }
}

#[tokio::test]
async fn default_budget_allows_long_repeated_tool_loops_without_context_pressure() {
    for strategy in [
        hanafubuki_agent_kernel::AgentCompactionStrategy::Standard,
        hanafubuki_agent_kernel::AgentCompactionStrategy::Background,
        hanafubuki_agent_kernel::AgentCompactionStrategy::Hierarchical,
    ] {
        let responses = (0..520).map(|index| {
            let ScriptedResponse::Events(mut events) = tool_response_with_usage(184_999) else {
                unreachable!()
            };
            for event in &mut events {
                if let Ok(
                    ModelEvent::OutputItemAdded {
                        item: ModelOutputItem::ToolCall { call },
                    }
                    | ModelEvent::OutputItemCompleted {
                        item: ModelOutputItem::ToolCall { call },
                    },
                ) = event
                {
                    call.call_id = format!("call-{index}");
                    call.arguments = json!({});
                }
            }
            ScriptedResponse::Events(events)
        });
        let model = ScriptedModel::new(responses.chain([response("answer", "Done", 185_100)]));
        let tools = FakeTools::new((0..520).map(|index| {
            FakeRegisteredExchange::success(
                "list_instances",
                json!({}),
                json!({"instances": [index]}),
            )
        }))
        .await;
        let service = AgentService::new(Arc::new(model.clone()), tools.registries());
        let session = service
            .create_session(AgentSessionConfig {
                context_window_tokens: Some(1_000_000),
                compaction_strategy: strategy,
                ..config()
            })
            .unwrap();
        let outcome = service
            .start_turn(session.id, "List instances repeatedly")
            .await;
        let snapshot = service.snapshot(session.id).unwrap();
        assert!(snapshot.compaction.is_none(), "{strategy:?}");
        assert_eq!(outcome.unwrap().final_answer, "Done", "{strategy:?}");
        let usage = snapshot.usage.unwrap();
        assert_eq!(usage.cumulative_input_tokens, 96_384_580);
        assert_eq!(usage.context_consumed_tokens, Some(185_100));
        assert_eq!(snapshot.context_window.window_number, 0);
        assert_eq!(model.requests().len(), 521);
        assert!(service.test_model_context(session.id).unwrap().len() > 1_024);
    }
}
