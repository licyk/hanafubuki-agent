//! A message's images through the chat: a steer carries them, and its
//! clipboard copies go once the turn read it; steers sent together are
//! renumbered; a message put back keeps them; one a model is known not to
//! read, or a `!` command with images, stays in the composer.

use std::path::Path;

use super::*;
use crate::agent_bridge::SessionStatus;
use crate::user_message::{ImageSource, MessageImage, UserMessage};

/// A clipboard image's copy at `directory/name`.
fn pasted(directory: &Path, name: &str) -> MessageImage {
    let path = directory.join(name);
    std::fs::write(&path, b"png").unwrap();
    MessageImage {
        source: ImageSource::Pasted(path),
        name: name.to_string(),
    }
}

fn with_image(text: &str, image: MessageImage) -> UserMessage {
    UserMessage {
        text: text.to_string(),
        images: vec![image],
    }
}

fn copy_exists(image: &MessageImage) -> bool {
    match &image.source {
        ImageSource::Pasted(path) => path.exists(),
        _ => panic!("a clipboard image"),
    }
}

#[test]
fn a_steer_carries_its_images_whose_copies_go_once_the_turn_read_it() {
    let directory = tempfile::tempdir().unwrap();
    let image = pasted(directory.path(), "clipboard-1.png");
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let message = with_image("[Image #1] look", image.clone());
    chat.submit_user_message(message.clone());
    assert_eq!(drain(&events).1, vec![Op::Steer { id: 0, message }]);

    // Taken but not read yet: the copy stays.
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 0,
        outcome: SteerOutcome::Accepted(1),
    });
    assert!(copy_exists(&image));
    chat.handle_turn_update(TurnUpdate::SteeringConsumed(1));
    let cells = trimmed(drain(&events).0);
    assert_eq!(cells, ["› [Image #1] look\n  [Image #1: clipboard-1.png]"]);
    assert!(!copy_exists(&image));
}

#[test]
fn steers_sent_together_are_renumbered_and_an_interrupt_puts_them_back_whole() {
    let directory = tempfile::tempdir().unwrap();
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    let first = pasted(directory.path(), "a.png");
    let second = pasted(directory.path(), "b.png");
    chat.submit_user_message(with_image("[Image #1] first", first.clone()));
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 0,
        outcome: SteerOutcome::Accepted(1),
    });
    chat.submit_user_message(with_image("second [Image #1]", second.clone()));
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 1,
        outcome: SteerOutcome::Accepted(2),
    });
    drain(&events);

    // Ctrl-C: the steers the turn did not read come back, copies kept.
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
    assert_eq!(
        chat.bottom_pane.composer_text(),
        "[Image #1] first\nsecond [Image #2]"
    );
    assert!(copy_exists(&first) && copy_exists(&second));

    // Sent again, they are one message with both images.
    chat.handle_key_event(key(KeyCode::Enter));
    let (_, ops) = drain(&events);
    let [Op::UserInput(sent)] = ops.as_slice() else {
        panic!("{ops:?}");
    };
    assert_eq!(sent.text, "[Image #1] first\nsecond [Image #2]");
    assert_eq!(sent.images, [first, second]);
}

#[test]
fn a_message_with_images_a_model_does_not_read_stays_in_the_composer() {
    let directory = tempfile::tempdir().unwrap();
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::SessionStatus(Box::new(SessionStatus {
        session_id: AgentSessionId::new(1),
        title: None,
        model: "text-only".to_string(),
        reasoning_effort: None,
        usage: None,
        task_progress: None,
        permission_mode: None,
        goal: None,
        git_branch: None,
        compression: None,
        role: None,
        images_refused: Some("reported".to_string()),
        background: None,
    })));
    let image = pasted(directory.path(), "a.png");
    chat.submit_user_message(with_image("[Image #1] what is it?", image.clone()));
    let (cells, ops) = drain(&events);
    assert!(ops.is_empty(), "{ops:?}");
    assert_eq!(
        trimmed(cells),
        ["■ This model does not read images (reported); remove the image or switch with /model."]
    );
    assert_eq!(chat.bottom_pane.composer_text(), "[Image #1] what is it?");
    assert!(copy_exists(&image));

    // Without its image it goes.
    chat.submit_user_message("what is it?".into());
    assert_eq!(drain(&events).1, vec![Op::UserInput("what is it?".into())]);
}

#[test]
fn a_command_with_images_stays_in_the_composer() {
    let directory = tempfile::tempdir().unwrap();
    let (mut chat, events) = chat();
    chat.submit_user_message(with_image(
        "!ls [Image #1]",
        pasted(directory.path(), "a.png"),
    ));
    let (cells, ops) = drain(&events);
    assert!(ops.is_empty(), "{ops:?}");
    assert_eq!(
        trimmed(cells),
        ["■ A `!` command takes no images; remove them first."]
    );
    assert_eq!(chat.bottom_pane.composer_text(), "!ls [Image #1]");
}
