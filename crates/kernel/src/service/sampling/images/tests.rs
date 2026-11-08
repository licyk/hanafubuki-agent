use super::*;

fn attachment(id: &str) -> AgentImageAttachmentRef {
    AgentImageAttachmentRef {
        attachment_id: crate::AgentAttachmentId::from_sha256(format!("{id:0>64}")),
        source_session_id: AgentSessionId::new(1),
        mime_type: "image/png".to_string(),
        byte_length: 1,
        width: 1,
        height: 1,
        display_name: format!("{id}.png"),
        detail: crate::AgentImageDetail::High,
    }
}

#[test]
fn an_attachment_is_answered_by_its_load_and_a_missing_one_is_stale() {
    let (loaded, failed, missing) = (attachment("a"), attachment("b"), attachment("c"));
    let images = LoadedImages {
        loaded: vec![
            (loaded.clone(), Ok(vec![7])),
            (
                failed.clone(),
                Err(AgentHistoryStoreError::CorruptHistory(
                    "unreadable".to_string(),
                )),
            ),
        ],
    };

    assert_eq!(images.bytes(&loaded), Ok(vec![7]));
    assert!(
        matches!(images.bytes(&failed), Err(AgentError::Persistence(message)) if message.contains("unreadable"))
    );
    assert_eq!(images.bytes(&missing), Err(AgentError::StaleRevision));
}

#[test]
fn the_referenced_images_are_the_ones_the_request_resolves_each_once() {
    let (first, second) = (attachment("a"), attachment("b"));
    let items = [
        ConversationItem::UserMessage {
            text: "look".to_string(),
            images: vec![first.clone(), second.clone()],
        },
        ConversationItem::SteeringMessage {
            sequence: AgentSteeringSequence::new(1),
            text: String::new(),
            images: vec![second.clone()],
        },
    ];

    assert_eq!(referenced_images(&items), vec![first, second]);
}
