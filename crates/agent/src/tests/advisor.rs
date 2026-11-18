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
    assert!(!String::from_utf8_lossy(&executor.requests()[0].body)
        .contains("Advisor review preferences:"));
    assert!(String::from_utf8_lossy(&executor.requests()[1].body)
        .contains("Advisor review preferences:"));
    service
        .write_setting_values(SettingsWrite {
            changes: vec![SettingChange {
                key: "pluginOptions.advisor.consultationLimit".into(),
                op: SettingWriteOp::Set(2.into()),
            }],
        })
        .await
        .unwrap();
    let error = service
        .consult_advisor(id, "again".into(), Default::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("budget exhausted"), "{error}");
    assert_eq!(advisor.requests().len(), 2);

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

#[tokio::test(flavor = "multi_thread")]
async fn advisor_tool_history_applies_disclosure_policy_before_sending() {
    let executor = FakeServer::start(vec![
        (tool_events("list_agents", "{}"), Duration::ZERO),
        (assistant_events("first turn complete"), Duration::ZERO),
    ])
    .await;
    let advisor = FakeServer::start(vec![(assistant_events("reviewed"), Duration::ZERO); 2]).await;
    let (service, paths, _) = service_with_config("advisor-disclosure", |_, config| {
        configure_test_provider(config, &executor.base_url, None);
        add_test_provider(config, "second", "Second", &advisor.base_url);
    });
    service.start_plugins().await;
    let id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(id, "Review token=secret-marker".to_owned().into())
        .await
        .unwrap();
    let mut changes = vec![];
    for (key, value) in [
        ("provider", json!("second")),
        ("model", json!("advisor")),
        ("redactSecrets", json!(true)),
        ("summaryTools", json!(["list_agents"])),
    ] {
        changes.push(SettingChange {
            key: format!("pluginOptions.advisor.{key}"),
            op: SettingWriteOp::Set(value),
        });
    }
    service
        .write_setting_values(SettingsWrite { changes })
        .await
        .unwrap();
    service
        .consult_advisor(id, "password=private-question".into(), Default::default())
        .await
        .unwrap();
    let request = String::from_utf8(advisor.requests()[0].body.clone()).unwrap();
    assert!(
        !request.contains("secret-marker") && !request.contains("private-question"),
        "{request}"
    );
    assert!(request.contains("result parts"), "{request}");
    service
        .write_setting_values(SettingsWrite {
            changes: vec![SettingChange {
                key: "pluginOptions.advisor.excludedTools".into(),
                op: SettingWriteOp::Set(json!(["list_agents"])),
            }],
        })
        .await
        .unwrap();
    service
        .consult_advisor(id, "check again".into(), Default::default())
        .await
        .unwrap();
    let request = String::from_utf8(advisor.requests()[1].body.clone()).unwrap();
    assert!(!request.contains("list_agents"), "{request}");
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follow_up_reuses_evidence_only_until_new_input() {
    let server = FakeServer::start(vec![
        (assistant_events("primary evidence"), Duration::ZERO),
        (assistant_events("first opinion"), Duration::ZERO),
        (assistant_events("second opinion"), Duration::ZERO),
        (assistant_events("new input acknowledged"), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("advisor-follow-up", &server.base_url);
    let id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .write_setting_values(SettingsWrite {
            changes: [("provider", "provider-1"), ("model", "second-model")]
                .map(|(k, v)| SettingChange {
                    key: format!("pluginOptions.advisor.{k}"),
                    op: SettingWriteOp::Set(v.into()),
                })
                .into(),
        })
        .await
        .unwrap();
    service
        .start_turn(id, "unique evidence".to_owned().into())
        .await
        .unwrap();
    let answer = service
        .consult_advisor(id, "review".into(), Default::default())
        .await
        .unwrap();
    let follow = |key: String| {
        let mut args = AdvisorConsultation::default();
        args.follow_up_to = Some(key);
        args.question = "Which assumption?".into();
        service.consult_advisor_with(id, args, Default::default())
    };
    let second = follow(answer.advice_id).await.unwrap();
    let body = String::from_utf8(server.requests()[2].body.clone()).unwrap();
    for part in ["unique evidence", "first opinion", "Which assumption?"] {
        assert!(body.contains(part), "{body}")
    }
    service
        .start_turn(id, "another question".to_string().into())
        .await
        .unwrap();
    assert!(follow(second.advice_id).await.is_err());
    assert_eq!(server.requests().len(), 4);
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enabled_gate_reviews_repeated_calls_and_returns_revision_to_the_executor() {
    let executor = FakeServer::start(vec![
        (tool_events("list_agents", "{}"), Duration::ZERO),
        (tool_events("list_agents", "{}"), Duration::ZERO),
        (assistant_events("done"), Duration::ZERO),
    ])
    .await;
    let advisor = FakeServer::start(vec![(
        assistant_events(r#"{"decision":"revise","reason":"Use the first inventory."}"#),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _) = service_with_config("advisor-gate", |_, config| {
        configure_test_provider(config, &executor.base_url, None);
        add_test_provider(config, "review", "Review", &advisor.base_url);
    });
    service.start_plugins().await;
    let id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .write_setting_values(SettingsWrite {
            changes: [
                ("provider", json!("review")),
                ("model", json!("reviewer")),
                ("repeatThreshold", json!(2)),
            ]
            .map(|(key, value)| SettingChange {
                key: format!("pluginOptions.advisor.{key}"),
                op: SettingWriteOp::Set(value),
            })
            .into(),
        })
        .await
        .unwrap();
    service.set_advisor_active(id, true).await.unwrap();
    service
        .start_turn(id, "check inventory".to_string().into())
        .await
        .unwrap();
    assert_eq!(advisor.requests().len(), 1);
    assert!(
        String::from_utf8_lossy(&executor.requests()[2].body).contains("Use the first inventory.")
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_settings_proposals_need_user_confirmation_and_scout_only_forwards_selected_evidence()
{
    let executor = FakeServer::start(vec![
        (
            tool_events("advisor_config", r#"{"changes":{"scout":true}}"#),
            Duration::ZERO,
        ),
        (assistant_events("candidate review"), Duration::ZERO),
        (assistant_events("[0]"), Duration::ZERO),
    ])
    .await;
    let advisor = FakeServer::start(vec![(assistant_events("feedback"), Duration::ZERO)]).await;
    let (service, paths, _) = service_with_config("advisor-config-scout", |_, config| {
        configure_test_provider(config, &executor.base_url, None);
        add_test_provider(config, "review", "Review", &advisor.base_url);
    });
    service.start_plugins().await;
    let id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .write_setting_values(SettingsWrite {
            changes: [("provider", "review"), ("model", "reviewer")]
                .map(|(key, value)| SettingChange {
                    key: format!("pluginOptions.advisor.{key}"),
                    op: SettingWriteOp::Set(value.into()),
                })
                .into(),
        })
        .await
        .unwrap();
    service.set_advisor_active(id, true).await.unwrap();
    service
        .start_turn(id, "selected evidence".to_string().into())
        .await
        .unwrap();
    let proposal = service.advisor_settings_proposal(id).await.unwrap();
    assert_eq!(proposal.changes[0].before, json!(false));
    assert_eq!(proposal.changes[0].after, json!(true));
    assert!(service
        .confirm_advisor_settings(id, "different".into(), true)
        .await
        .is_err());
    service
        .confirm_advisor_settings(id, proposal.id, true)
        .await
        .unwrap();
    service
        .consult_advisor(id, "review".into(), Default::default())
        .await
        .unwrap();
    let requests = executor.requests();
    let scout = String::from_utf8_lossy(&requests[2].body);
    assert!(scout.contains("selected evidence") && scout.contains("candidate review"));
    let requests = advisor.requests();
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body.contains("selected evidence") && !body.contains("candidate review"),
        "{body}"
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_handoff_requires_a_named_tracked_file_and_transmits_only_selected_pixels() {
    use hanafubuki_agent_config::{AgentModelImageInput, AgentModelOverrideSettings};
    let reviewer = FakeServer::start(vec![
        (assistant_events("Read `tracked.txt`."), Duration::ZERO),
        (assistant_events("Reviewed."), Duration::ZERO),
    ])
    .await;
    let (service, paths, _) = service_with_config("advisor-workspace", |_, config| {
        configure_test_provider(config, &reviewer.base_url, None);
        config
            .mutate_providers(config.providers().unwrap().revision, |reg| {
                reg.providers[0].models.insert(
                    "review".into(),
                    AgentModelOverrideSettings {
                        image_input: AgentModelImageInput::Supported,
                        ..Default::default()
                    },
                );
                Ok(())
            })
            .unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path();
    for (name, body) in [
        ("tracked.txt", "tracked-body"),
        ("secret.txt", "excluded-body"),
    ] {
        std::fs::write(workspace.join(name), body).unwrap();
    }
    image::DynamicImage::new_rgb8(3, 2)
        .save(workspace.join("sample.png"))
        .unwrap();
    for args in [vec!["init", "-q"], vec!["add", "tracked.txt"]] {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(workspace)
            .status()
            .unwrap()
            .success());
    }
    service.start_plugins().await;
    let id = service
        .create_session_with_workspace(
            AgentSessionMode::Default,
            None,
            workspace.to_string_lossy().into_owned(),
        )
        .await
        .unwrap()
        .id;
    service
        .write_setting_values(SettingsWrite {
            changes: [
                ("provider", json!("provider-1")),
                ("model", json!("review")),
                ("gitContext", json!("summary")),
                ("untrackedFiles", json!(true)),
                ("trackedFiles", json!(true)),
                ("shareImages", json!(true)),
            ]
            .map(|(key, value)| SettingChange {
                key: format!("pluginOptions.advisor.{key}"),
                op: SettingWriteOp::Set(value),
            })
            .into(),
        })
        .await
        .unwrap();
    let mut request = AdvisorConsultation::default();
    request.include_tracked_files.push("tracked.txt".into());
    assert!(service
        .consult_advisor_with(id, request.clone(), Default::default())
        .await
        .is_err());
    service
        .consult_advisor(id, "inspect".into(), Default::default())
        .await
        .unwrap();
    request.include_untracked.push("sample.png".into());
    service
        .consult_advisor_with(id, request, Default::default())
        .await
        .unwrap();
    let requests = reviewer.requests();
    let summary = String::from_utf8_lossy(&requests[0].body);
    assert!(summary.contains("tracked.txt") && !summary.contains("tracked-body"));
    let body = String::from_utf8_lossy(&requests[1].body);
    assert!(
        body.contains("tracked-body")
            && body.contains("data:image/png;base64,")
            && !body.contains("excluded-body"),
        "{body}"
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}
