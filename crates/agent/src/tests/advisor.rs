use super::*;
use hanafubuki_agent_config::{SettingChange, SettingWriteOp};
use hanafubuki_agent_kernel::AgentSessionMode;

#[tokio::test(flavor = "multi_thread")]
async fn advisor_manual_and_model_calls_share_a_provider_only_after_explicit_activation() {
    let executor = FakeServer::start(vec![
        (assistant_events("observed green"), Duration::ZERO),
        (
            tool_events("ask_advisor", r#"{"question":"Which colour?"}"#),
            Duration::ZERO,
        ),
        (assistant_events("verified green"), Duration::ZERO),
        (assistant_events("finished"), Duration::ZERO),
    ])
    .await;
    let advisor = FakeServer::start(vec![
        (assistant_events("manual green"), Duration::ZERO),
        (assistant_events("advisor green"), Duration::ZERO),
    ])
    .await;
    let (service, paths, _) = service_with_config("advisor-activation", |_, config| {
        configure_test_provider(config, &executor.base_url, None);
        add_test_provider(config, "second", "Second", &advisor.base_url);
    });
    service.start_plugins().await;
    let id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    assert!(service.set_advisor_active(id, true).await.is_err());
    service
        .write_setting_values(SettingsWrite {
            changes: [("provider", "second"), ("model", "visual/reviewer")]
                .map(|(key, value)| SettingChange {
                    key: format!("pluginOptions.advisor.{key}"),
                    op: SettingWriteOp::Set(value.into()),
                })
                .into(),
        })
        .await
        .unwrap();
    service
        .start_turn(id, "remember the colour green".to_owned().into())
        .await
        .unwrap();
    let advertised = |index: usize| {
        let body: serde_json::Value =
            serde_json::from_slice(&executor.requests()[index].body).unwrap();
        body["tools"].to_string().contains("ask_advisor")
    };
    assert!(!advertised(0));
    let answer = service
        .consult_advisor(id, "What colour?".into(), Default::default())
        .await
        .unwrap();
    assert_eq!(answer.text, "manual green");
    let body: serde_json::Value = serde_json::from_slice(&advisor.requests()[0].body).unwrap();
    assert_eq!(body["model"], "visual/reviewer");
    assert!(body["input"].to_string().contains("observed green"));
    assert!(body["tools"].as_array().is_none_or(Vec::is_empty));
    service.set_advisor_active(id, true).await.unwrap();
    service
        .start_turn(id, "consult".to_owned().into())
        .await
        .unwrap();
    assert!(advertised(1));
    assert_eq!(advisor.requests().len(), 2);
    service.set_advisor_active(id, false).await.unwrap();
    service
        .start_turn(id, "finish".to_owned().into())
        .await
        .unwrap();
    assert!(!advertised(3));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an authenticated Hoshimi instance with two configured models"]
async fn live_advisor_routes_a_manual_question_to_the_selected_model() {
    let url = std::env::var("HOSHIMI_ADVISOR_TEST_URL").unwrap();
    let executor = std::env::var("HOSHIMI_EXECUTOR_MODEL").unwrap();
    let reviewer = std::env::var("HOSHIMI_ADVISOR_MODEL").unwrap();
    let (service, paths, _) = service_with_config("live-advisor", |_, config| {
        configure_test_provider(config, &url, None);
        config
            .mutate_providers(1, |reg| {
                reg.model_policy.model = executor.clone();
                Ok(())
            })
            .unwrap();
    });
    service.start_plugins().await;
    let id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let marker = format!("SAPPHIRE-{}", std::process::id());
    service
        .start_turn(
            id,
            format!("Reply only with the marker {marker}. Do not use tools.").into(),
        )
        .await
        .unwrap();
    service
        .write_setting_values(SettingsWrite {
            changes: [("provider", "provider-1"), ("model", reviewer.as_str())]
                .map(|(key, value)| SettingChange {
                    key: format!("pluginOptions.advisor.{key}"),
                    op: SettingWriteOp::Set(value.into()),
                })
                .into(),
        })
        .await
        .unwrap();
    let answer = service
        .consult_advisor(
            id,
            "Repeat the exact marker mentioned by the user. No explanation.".into(),
            Default::default(),
        )
        .await
        .unwrap();
    assert!(answer.text.contains(&marker), "{answer:?}");
    assert_eq!(answer.model, reviewer);
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}
