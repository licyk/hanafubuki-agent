//! Opt-in comparison of the real view_image tool and pasted image input.

use super::*;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_kernel::{AgentImageDetail, ConversationItem};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires HOSHIMI_VISION_TEST_URL, HOSHIMI_VISION_TEST_IMAGE and logged-in qodercli-cn"]
async fn live_hoshimi_view_image_and_paste_deliver_the_same_image() {
    let url = std::env::var("HOSHIMI_VISION_TEST_URL").unwrap();
    let source =
        ResolvedPath::resolve(std::env::var("HOSHIMI_VISION_TEST_IMAGE").unwrap()).unwrap();
    let bytes = std::fs::read(source.system()).unwrap();
    let (service, paths) = service_with("vision-tool-live", |_, config| {
        configure_test_provider(config, &url, Some(ModelReasoningEffort::Low));
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.model = "qodercli-cn/DeepSeek-Flash".into();
                Ok(())
            })
            .unwrap();
    });
    service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();
    let prompt = "请客观描述图片中的人物、主要颜色、表情、动作和手中物品，用中文回答。看不到图片就明确说明，不要猜测。";
    let mut attachments = Vec::new();
    for pasted in [false, true] {
        let session = service
            .create_session(AgentSessionMode::Default, None)
            .await
            .unwrap();
        let input = if pasted {
            let image = service
                .import_image_bytes(
                    session.id,
                    bytes.clone(),
                    "vision-probe.png".into(),
                    AgentImageDetail::Auto,
                )
                .unwrap();
            attachments.push(image.attachment_id.clone());
            AgentTurnInput {
                images: vec![image],
                ..AgentTurnInput::from(format!("{prompt} 直接看附图，不要调用工具。"))
            }
        } else {
            AgentTurnInput::from(format!(
                "请先调用 view_image 查看 {}，然后{prompt} 不要使用其他工具。",
                source.native().display()
            ))
        };
        let result = tokio::time::timeout(
            Duration::from_secs(180),
            Box::pin(service.start_turn(session.id, input)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!result.final_answer.is_empty());
        let context = service.core.test_model_context(session.id).unwrap();
        if !pasted {
            assert!(context.iter().any(|item| matches!(
                item, ConversationItem::ToolCall { name, .. } if name == "view_image"
            )));
            for item in &context {
                if let ConversationItem::ToolOutput { images, .. } = item {
                    attachments.extend(images.iter().map(|image| image.attachment_id.clone()));
                }
            }
            assert_eq!(attachments.len(), 1);
        }
        println!(
            "LIVE_VISION {}",
            json!({"pasted": pasted, "answer": result.final_answer})
        );
    }
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0], attachments[1]);
    let stopping = service.clone();
    let diagnostics = tokio::task::spawn_blocking(move || {
        stopping.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
    })
    .await
    .unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}
