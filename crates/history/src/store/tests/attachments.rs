use super::*;
use std::io::Cursor;

pub(super) fn pixel_png() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

#[test]
fn attachment_blob_is_published_before_reference_use_and_deleted_with_session() {
    let (_temporary, paths, _workers, store) = open_store();
    let session_id = AgentSessionId::new(1);
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 1),
            history_mutations: Vec::new(),
        })
        .unwrap();
    let attachment = store
        .stage_image_attachment(
            session_id,
            hanafubuki_agent_kernel::PreparedAgentImage {
                bytes: pixel_png(),
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "pixel.png".to_string(),
                detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
            },
        )
        .unwrap();
    assert_eq!(
        store.load_image_attachment(&attachment).unwrap(),
        pixel_png()
    );
    let mut forged_mime = attachment.clone();
    forged_mime.mime_type = "image/jpeg".to_string();
    assert!(store.load_image_attachment(&forged_mime).is_err());
    let mut forged_dimensions = attachment.clone();
    forged_dimensions.width = 2;
    assert!(store.load_image_attachment(&forged_dimensions).is_err());
    let mut referenced_item = item(1, 0, "");
    referenced_item.content = ConversationItem::UserMessage {
        text: String::new(),
        images: vec![attachment.clone()],
    };
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(session_id, 1, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: referenced_item,
            }],
        })
        .unwrap();
    assert_eq!(
        store
            .load_visible_image_attachment(session_id, &attachment)
            .unwrap(),
        pixel_png()
    );
    assert!(store
        .load_visible_image_attachment(AgentSessionId::new(999), &attachment)
        .is_err());
    let blob = paths
        .attachments
        .join(session_id.value().to_string())
        .join("blobs")
        .join(format!("{}.blob", attachment.attachment_id.as_str()));
    assert!(blob.is_file());

    store.delete_sessions_confirmed(&[session_id]).unwrap();
    assert!(!paths
        .attachments
        .join(session_id.value().to_string())
        .exists());
}

/// A queued load reads on the session's writer and answers each attachment
/// in order, a forged reference with its own error.
#[test]
fn a_queued_attachment_load_answers_each_attachment_in_order() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = AgentSessionId::new(1);
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 1),
            history_mutations: Vec::new(),
        })
        .unwrap();
    let attachment = store
        .stage_image_attachment(
            session_id,
            hanafubuki_agent_kernel::PreparedAgentImage {
                bytes: pixel_png(),
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "pixel.png".to_string(),
                detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
            },
        )
        .unwrap();
    let mut forged = attachment.clone();
    forged.width = 2;
    let (sender, receiver) = std::sync::mpsc::channel();

    store.enqueue_image_attachment_load(
        session_id,
        vec![forged, attachment],
        hanafubuki_agent_kernel::AgentImageAttachmentLoadAck::new(move |loaded| {
            let _ = sender.send((std::thread::current().name().map(str::to_owned), loaded));
        }),
    );

    let (thread, loaded) = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        thread.as_deref(),
        Some(format!("agent-history-writer-{session_id}").as_str())
    );
    assert!(loaded[0].is_err());
    assert_eq!(loaded[1].as_ref().unwrap(), &pixel_png());
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
