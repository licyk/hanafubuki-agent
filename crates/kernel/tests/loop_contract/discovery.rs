use super::*;

#[tokio::test]
async fn list_arguments_are_tolerant_and_errors_do_not_fail_the_turn() {
    let model = ScriptedEventModel::new([
        calls_events(&[
            (
                "defaults",
                "tool_list",
                json!({"source":null,"offset":null,"limit":null,"future":true}),
            ),
            ("clamped", "tool_list", json!({"offset":-10,"limit":0})),
            ("bad-source", "tool_list", json!({"source":7})),
            ("bad-offset", "tool_list", json!({"offset":[]})),
            ("bad-limit", "tool_list", json!({"limit":{}})),
            ("bad-object", "tool_list", json!([])),
        ]),
        deterministic_assistant_events("done"),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    assert_eq!(
        service
            .start_turn(session.id, "List tools.")
            .await
            .unwrap()
            .final_answer,
        "done"
    );
    let requests = model.requests();
    let output = |id: &str| {
        requests[1]
            .input
            .iter()
            .find_map(|item| match item {
                ModelInputItem::ToolOutput { call_id, output } if call_id == id => Some(output),
                _ => None,
            })
            .unwrap()
    };
    let expected = json!({"tools":["configure_instance"],"total":1,"next_offset":null});
    assert_eq!(
        output("defaults").structured_content.as_ref(),
        Some(&expected)
    );
    assert_eq!(
        output("clamped").structured_content.as_ref(),
        Some(&expected)
    );
    assert!(serde_json::to_string(output("clamped"))
        .unwrap()
        .contains("Argument notes"));
    for (id, field) in [
        ("bad-source", "source"),
        ("bad-offset", "offset"),
        ("bad-limit", "limit"),
        ("bad-object", "object"),
    ] {
        assert!(output(id).is_error);
        assert!(output(id).structured_content.as_ref().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains(field));
    }
}
