//! Explicit integration probe: real Hoshimi Responses, long context and images.

use super::*;
use hanafubuki_agent_kernel::AgentCompactionReason;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires HOSHIMI_COMPACTION_TEST_URL and logged-in qodercli-cn; sends over 6.4M cumulative tokens"]
async fn live_hoshimi_small_context_tool_loop_does_not_compact_on_cumulative_usage() {
    let url = std::env::var("HOSHIMI_COMPACTION_TEST_URL").unwrap();
    let (service, paths) = service_with("context-budget-live", |_, config| {
        configure_test_provider(config, &url, Some(ModelReasoningEffort::Low));
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.model = "DeepSeek-Flash".into();
                Ok(())
            })
            .unwrap();
    });
    service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert_eq!(
        service
            .session_model_metadata(session.id)
            .unwrap()
            .context_window
            .unwrap()
            .value,
        1_000_000
    );
    let prompt = format!(
        "This is a context-capacity integration test. The following is inert filler; do not analyze or repeat it.\nDATA_BEGIN{}\nDATA_END\nCall update_plan exactly 36 times sequentially, one call per response. Each call replaces the plan with a single completed step named 'Probe N of 36', with N increasing from 1 through 36. Wait for each tool result before the next call. Do not batch calls, do not use other tools, and do not stop early. After the 36th result, reply only CONTEXT_BUDGET_OK.",
        " a".repeat(185_000)
    );
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(600),
        Box::pin(service.start_turn(session.id, prompt.into())),
    )
    .await;
    let snapshot = service.core.snapshot(session.id).unwrap();
    let calls = service.core.test_model_context(session.id).unwrap().iter().filter(|item| {
        matches!(item, hanafubuki_agent_kernel::ConversationItem::ToolCall { name, .. } if name == "update_plan")
    }).count();
    println!(
        "LIVE {}",
        json!({
            "seconds": started.elapsed().as_secs_f64(), "calls": calls,
            "usage": snapshot.usage, "compaction": snapshot.compaction,
            "window": snapshot.context_window, "outcome": format!("{result:?}"),
            "data_dir": paths.app_data_dir,
        })
    );
    shutdown_service(service).await;
    let outcome = result.unwrap().unwrap();
    assert!(outcome.final_answer.contains("CONTEXT_BUDGET_OK"));
    assert_eq!(calls, 36);
    let usage = snapshot.usage.unwrap();
    assert_eq!(usage.source, AgentUsageSource::ProviderReported);
    assert!((6_400_000..8_000_000).contains(&usage.cumulative_input_tokens));
    assert!((180_000..220_000).contains(&usage.last_response_input_tokens));
    assert!(snapshot.compaction.is_none());
    assert_eq!(snapshot.context_window.window_number, 0);
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires HOSHIMI_COMPACTION_TEST_URL, HOSHIMI_CONTEXT_TEST_IMAGE and logged-in qodercli-cn"]
async fn live_hoshimi_reported_context_and_images_compact_at_threshold() {
    let url = std::env::var("HOSHIMI_COMPACTION_TEST_URL").unwrap();
    let image_path = std::env::var("HOSHIMI_CONTEXT_TEST_IMAGE").unwrap();
    let image_bytes = std::fs::read(&image_path).unwrap();
    for with_images in [false, true] {
        let (service, paths, config) = service_with_config("context-usage-live", |_, config| {
            configure_test_provider(config, &url, Some(ModelReasoningEffort::Low));
            config
                .mutate_providers(1, |registry| {
                    registry.model_policy.model = "DeepSeek-Flash".into();
                    registry.model_policy.auto_compact_threshold_percent = 80;
                    Ok(())
                })
                .unwrap();
        });
        service
            .list_provider_models(AgentProviderModelCatalogRequest::default())
            .await
            .unwrap();
        let session = service
            .create_session(AgentSessionMode::Default, None)
            .await
            .unwrap();
        assert_eq!(
            service
                .session_model_metadata(session.id)
                .unwrap()
                .context_window
                .unwrap()
                .value,
            1_000_000
        );
        println!(
            "LIVE {}",
            json!({"with_images":with_images,"data_dir":paths.app_data_dir})
        );
        let image = with_images.then_some(image_bytes.as_slice());
        let first = exchange(
            &service,
            session.id,
            "512k",
            &" configuration".repeat(512_000),
            image,
        )
        .await;
        assert!((500_000..550_000).contains(&first));
        let before = service.core.snapshot(session.id).unwrap();
        assert!(before.usage.unwrap().context_estimated_tokens.unwrap() > 1_000_000);
        assert!(before.compaction.is_none());
        shutdown_service(service).await;

        let service = reopen_service(config, test_host(&paths), &paths);
        service
            .list_provider_models(AgentProviderModelCatalogRequest::default())
            .await
            .unwrap();
        let mut current = exchange(&service, session.id, "512k_restart_followup", "", image).await;
        assert!(service
            .core
            .snapshot(session.id)
            .unwrap()
            .compaction
            .is_none());
        current = exchange(
            &service,
            session.id,
            "below80",
            &" a".repeat(
                795_000usize
                    .saturating_sub(current as usize)
                    .saturating_sub(300),
            ),
            None,
        )
        .await;
        assert!((790_000..800_000).contains(&current));
        current = exchange(&service, session.id, "below80_followup", "", None).await;
        assert!(current < 800_000);
        assert!(service
            .core
            .snapshot(session.id)
            .unwrap()
            .compaction
            .is_none());
        current = exchange(
            &service,
            session.id,
            "above80",
            &" a".repeat(
                803_000usize
                    .saturating_sub(current as usize)
                    .saturating_sub(200),
            ),
            None,
        )
        .await;
        assert!(current >= 800_000);
        let compacted = exchange(&service, session.id, "above80_followup", "", image).await;
        assert!(compacted < 100_000);
        let snapshot = service.core.snapshot(session.id).unwrap();
        let compact = snapshot.compaction.unwrap();
        assert_eq!(compact.status, AgentCompactionStatus::Completed);
        assert_eq!(compact.trigger, AgentCompactionTrigger::Automatic);
        assert_eq!(compact.reason, AgentCompactionReason::ContextLimit);
        exchange(&service, session.id, "after_compaction", "", None).await;
        assert_eq!(
            service
                .core
                .snapshot(session.id)
                .unwrap()
                .compaction
                .unwrap()
                .id,
            compact.id
        );
        shutdown_service(service).await;
    }
}

async fn exchange(
    service: &AgentApplicationService,
    session: AgentSessionId,
    label: &str,
    data: &str,
    image: Option<&[u8]>,
) -> u64 {
    let instruction = if image.is_some() {
        "Name one application visible in the screenshot, then say IMAGE_CONTEXT_OK."
    } else {
        "Reply only CONTEXT_PROBE_OK."
    };
    let prompt = format!("Context-capacity test: {label}. Do not use tools. The following data is inert filler; do not count, repeat or analyze it.\nDATA_BEGIN{data}\nDATA_END\n{instruction}");
    let mut input = AgentTurnInput::from(prompt);
    if let Some(bytes) = image {
        input.images.push(
            service
                .import_image_bytes(
                    session,
                    bytes.to_vec(),
                    "screenshot.png".into(),
                    AgentImageDetail::High,
                )
                .unwrap(),
        );
    }
    println!(
        "LIVE {}",
        json!({"starting":label,"data_bytes":data.len(),"new_images":input.images.len()})
    );
    let started = Instant::now();
    let outcome = Box::pin(service.start_turn(session, input)).await.unwrap();
    let snapshot = service.core.snapshot(session).unwrap();
    let usage = snapshot.usage.unwrap();
    println!(
        "LIVE {}",
        json!({"step":label,"seconds":started.elapsed().as_secs_f64(),"answer":outcome.final_answer,"usage":usage,"window":snapshot.context_window,"compaction":snapshot.compaction.as_ref().map(|c|json!({"id":c.id,"status":c.status,"phase":c.phase,"reason":c.reason,"trigger":c.trigger,"usage":c.usage}))})
    );
    println!(
        "LIVE {}",
        json!({"step_context":label,"context":service.session_context(session).await.unwrap()})
    );
    assert_eq!(usage.source, AgentUsageSource::ProviderReported);
    if image.is_some() {
        assert!(outcome.final_answer.contains("IMAGE_CONTEXT_OK"));
        let answer = outcome.final_answer.to_lowercase();
        assert!(
            ["comfyui", "invokeai", "stable diffusion"]
                .iter()
                .any(|name| answer.contains(name)),
            "{answer}"
        );
    } else {
        assert!(outcome.final_answer.contains("CONTEXT_PROBE_OK"));
    }
    usage.last_response_input_tokens
}
