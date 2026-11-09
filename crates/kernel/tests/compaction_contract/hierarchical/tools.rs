use hanafubuki_agent_kernel::{AgentImageAttachmentRef, AgentImageDetail, PreparedAgentImage};

use super::*;

impl ScriptedModel {
    /// Queues `responses` after those already queued.
    fn push(&self, responses: impl IntoIterator<Item = ScriptedResponse>) {
        self.responses.lock().unwrap().extend(responses);
    }
}

/// The images committed with the output of `call_id`.
fn output_images(
    service: &AgentService,
    session: AgentSessionId,
    call_id: &str,
) -> Vec<AgentImageAttachmentRef> {
    service
        .test_model_context(session)
        .unwrap()
        .into_iter()
        .find_map(|item| match item {
            ConversationItem::ToolOutput {
                call_id: id,
                images,
                ..
            } if id == call_id => Some(images),
            _ => None,
        })
        .expect("the call's output is committed")
}

/// `startId`/`endId` restore only the block's messages in the range, read
/// from the history, and leave the block as it is; a half range, a range the
/// block does not cover and a block ref in place of a message ref are
/// refused.
#[tokio::test]
async fn a_range_restores_only_the_messages_it_names() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold", "m00002–m00004", "Both answers"),
        call(
            "decompress",
            "range",
            json!({ "blockId": "b1", "startId": "m00003", "endId": "m00004" }),
        ),
        call(
            "decompress",
            "half",
            json!({ "blockId": "b1", "startId": "m00003", "endId": "" }),
        ),
        call(
            "decompress",
            "outside",
            json!({ "blockId": "b1", "startId": "m00007", "endId": "m00009" }),
        ),
        call(
            "decompress",
            "block-ref",
            json!({ "blockId": "b1", "startId": "b1", "endId": "m00004" }),
        ),
        response("three", "Restored.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold and look again."))
        .await
        .unwrap();

    let range = tool_output(&service, session.id, "range");
    assert!(
        range.contains("[Block b1 content — m00003–m00004 — 2 item(s)]"),
        "{range}"
    );
    assert!(range.contains("[user]\\nsecond"), "{range}");
    assert!(range.contains("a long second answer"));
    assert!(!range.contains("a long first answer"));
    assert!(!range.contains("Re-fold"));
    assert!(tool_output(&service, session.id, "half").contains("must be given together"));
    let outside = tool_output(&service, session.id, "outside");
    assert!(
        outside.contains("b1 covers no messages in m00007–m00009 (its coverage is m00002–m00004)"),
        "{outside}"
    );
    assert!(tool_output(&service, session.id, "block-ref").contains("must be mNNNNN message refs"));
    let state = state(&service, session.id);
    assert!(!state.block("b1").unwrap().restored_inline);
    // The range is in the request after it, where the fold left its messages
    // out.
    let last = text(model.requests().last().unwrap());
    assert!(last.contains("a long second answer"));
}

/// `search_context` finds what no summary kept in the messages the blocks
/// folded, naming the message's ref and the block's span; a summary hit is
/// listed first and its block is not repeated; a query that matches nothing
/// says so.
#[tokio::test]
async fn search_finds_the_folded_originals() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        compress("fold-2", "m00004–m00004", "Second exchange"),
        call(
            "search_context",
            "original",
            json!({ "query": "long answer" }),
        ),
        call(
            "search_context",
            "summary",
            json!({ "query": "exchange answer" }),
        ),
        call("search_context", "nothing", json!({ "query": "zebra" })),
        response("three", "Searched.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold and search."))
        .await
        .unwrap();

    let original = tool_output(&service, session.id, "original");
    assert!(
        original.contains(
            "Found 0 summary block(s) for \\\"long answer\\\" — but 2 folded original(s) match:"
        ),
        "{original}"
    );
    for (block, message_ref, text) in [
        ("b1", "m00002", "a long first answer"),
        ("b2", "m00004", "a long second answer"),
    ] {
        assert!(
            original.contains(&format!(
                "{block} (original text, not in any summary, at {message_ref}) [{message_ref} · 1 msgs]\\n  {text}"
            )),
            "{original}"
        );
    }

    let summary = tool_output(&service, session.id, "summary");
    assert!(summary.contains("Found 2 block(s) for"), "{summary}");
    assert!(
        summary.contains("b1 (T1) \\\"First exchange\\\" [m00002 · 1 msgs]"),
        "{summary}"
    );
    // The summaries name "exchange": each block is listed once, by it.
    assert!(!summary.contains("Original-text hit(s)"), "{summary}");
    assert!(summary.contains("\u{3010}"), "{summary}");

    let nothing = tool_output(&service, session.id, "nothing");
    assert!(
        nothing.contains("[No blocks matched \\\"zebra\\\" among 2 block(s)."),
        "{nothing}"
    );
}

/// An image folded with its message is listed in the summary's MEDIA REFS,
/// and comes back attached to `decompress`'s output by its message's ref,
/// by that handle and with a `full` restore: the request after it carries
/// the pixels again.
#[tokio::test]
async fn a_folded_image_is_reached_again() {
    let model = ScriptedModel::new([]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    let image = service
        .stage_image_attachment(
            session.id,
            PreparedAgentImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".to_string(),
                width: 4,
                height: 3,
                display_name: "screenshot.png".to_string(),
                detail: AgentImageDetail::High,
            },
        )
        .unwrap();
    let handle = image.attachment_id.as_str().to_owned();
    model.push(two_exchanges().into_iter().chain([
        response(
            "three",
            &"the screenshot shows a failure ".repeat(100),
            2_000,
        ),
        compress("fold", "m00005–m00006", "The screenshot"),
        response("four", "Folded.", 3_000),
        call(
            "decompress",
            "list",
            json!({ "blockId": "", "imageRef": "list" }),
        ),
        call("decompress", "by-ref", json!({ "imageRef": "m00005" })),
        call("decompress", "by-handle", json!({ "imageRef": handle })),
        call("decompress", "missing", json!({ "imageRef": "m00002" })),
        call(
            "decompress",
            "full",
            json!({ "blockId": "b1", "full": true }),
        ),
        response("five", "Seen again.", 3_000),
    ]));
    seed(&service, session.id).await;
    let mut with_image = turn("Here is a screenshot of the failure, please look at it.");
    with_image.images = vec![image.clone()];
    service.start_turn(session.id, with_image).await.unwrap();
    service
        .start_turn(session.id, turn("Fold the screenshot."))
        .await
        .unwrap();
    let summary = state(&service, session.id)
        .block("b1")
        .unwrap()
        .summary
        .clone();
    assert!(
        summary.contains(&format!(
            "MEDIA REFS (verbatim re-access handles — preserve when distilling):\n- {handle}"
        )),
        "{summary}"
    );
    let folded = text(model.requests().last().unwrap());
    assert!(
        !folded.contains("AQID"),
        "the folded image left the request"
    );

    service
        .start_turn(session.id, turn("Look at the screenshot again."))
        .await
        .unwrap();

    let list = tool_output(&service, session.id, "list");
    assert!(
        list.contains("[Restorable images (1):]\\nm00005 [png 4x3 · 1KB]"),
        "{list}"
    );
    assert!(output_images(&service, session.id, "list").is_empty());
    for call_id in ["by-ref", "by-handle", "full"] {
        assert_eq!(
            output_images(&service, session.id, call_id),
            vec![image.clone()],
            "{call_id}"
        );
    }
    assert!(tool_output(&service, session.id, "by-ref").contains("[Restored 1 image(s) for m00005"));
    assert!(tool_output(&service, session.id, "missing").contains("no restorable image for ref"));
    let restored = text(model.requests().last().unwrap());
    assert!(
        restored.contains("AQID"),
        "the pixels are in the request again"
    );
}

/// `toFile` asks for the session's export file. A store that keeps no files
/// (an ephemeral one, as here) refuses: the call fails with the content
/// inline and the block is left as it was.
#[tokio::test]
async fn to_file_without_files_fails_with_the_content() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold", "m00002–m00002", "First exchange"),
        call(
            "decompress",
            "export",
            json!({ "blockId": "b1", "toFile": "notes/b1.md" }),
        ),
        response("three", "Tried.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold and export."))
        .await
        .unwrap();

    let export = tool_output(&service, session.id, "export");
    assert!(
        export.contains("[decompress FAILED: could not write export decompress-b1.txt:"),
        "{export}"
    );
    assert!(export.contains("The export did NOT complete — partial content follows (4000 of "));
    assert!(export.contains("a long first answer"));
    assert!(
        !state(&service, session.id)
            .block("b1")
            .unwrap()
            .restored_inline
    );
}
