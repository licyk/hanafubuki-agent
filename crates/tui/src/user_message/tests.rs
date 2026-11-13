use std::path::PathBuf;

use super::*;

fn image(source: ImageSource, name: &str) -> MessageImage {
    MessageImage {
        source,
        name: name.to_string(),
    }
}

fn file(name: &str) -> MessageImage {
    image(ImageSource::File(PathBuf::from(format!("/a/{name}"))), name)
}

#[test]
fn only_each_labels_first_appearance_within_the_images_is_its_image() {
    let text = "[Image #2] then [Image #1], [Image #2] again, [Image #3], [Image #x] [Image #1";
    let ranges = label_ranges(text, 2);
    let found: Vec<(&str, usize)> = ranges
        .iter()
        .map(|(range, number)| (&text[range.clone()], *number))
        .collect();
    assert_eq!(found, [("[Image #2]", 2), ("[Image #1]", 1)]);
    assert_eq!(ranges[1].0.start, "[Image #2] then ".len());
    assert!(label_ranges(text, 0).is_empty());
}

#[test]
fn joined_messages_number_their_images_after_the_ones_before() {
    let first = UserMessage {
        text: "see [Image #1]".to_string(),
        images: vec![file("a.png")],
    };
    let plain = UserMessage::from("no image");
    let second = UserMessage {
        text: "[Image #2] beside [Image #1]; [Image #9] is text".to_string(),
        images: vec![file("b.png"), file("c.png")],
    };
    let joined = UserMessage::join([first, plain, second]);
    assert_eq!(
        joined.text,
        "see [Image #1]\nno image\n[Image #3] beside [Image #2]; [Image #9] is text"
    );
    let names: Vec<&str> = joined
        .images
        .iter()
        .map(|image| image.name.as_str())
        .collect();
    assert_eq!(names, ["a.png", "b.png", "c.png"]);
}

#[test]
fn a_clipboard_image_whose_copy_is_gone_is_left_out_with_its_label() {
    let directory = tempfile::tempdir().unwrap();
    let kept = directory.path().join("kept.png");
    std::fs::write(&kept, b"png").unwrap();
    let gone = directory.path().join("gone.png");
    let message = UserMessage {
        text: "[Image #1] [Image #2] and [Image #3]".to_string(),
        images: vec![
            image(ImageSource::Pasted(gone), "gone.png"),
            image(ImageSource::Pasted(kept.clone()), "kept.png"),
            file("mine.png"),
        ],
    };
    let available = message.available();
    assert_eq!(available.text, " [Image #1] and [Image #2]");
    assert_eq!(available.images[0].source, ImageSource::Pasted(kept));
    assert_eq!(available.images[1].name, "mine.png");
    // A file of the person's is never checked, nor dropped.
    let mine = UserMessage {
        text: "[Image #1]".to_string(),
        images: vec![file("missing.png")],
    };
    assert_eq!(mine.clone().available(), mine);
}

#[test]
fn a_stored_message_drops_every_image_label() {
    assert_eq!(
        UserMessage::stored("[Image #1] what is [Image #12]?").text,
        " what is ?"
    );
    assert!(UserMessage::stored("x").images.is_empty());
}

#[test]
fn the_agent_reads_paths_and_attachments_and_the_transcript_names_them() {
    let message = UserMessage {
        text: "[Image #1] [Image #2]".to_string(),
        images: vec![
            image(
                ImageSource::Pasted(PathBuf::from("/tmp/clipboard-1.png")),
                "clipboard-1.png",
            ),
            file("shot.jpg"),
        ],
    };
    let inputs = message.request_images();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].path.as_deref(), Some("/tmp/clipboard-1.png"));
    assert_eq!(inputs[0].display_name, "clipboard-1.png");
    assert!(inputs[0].attachment.is_none());
    assert_eq!(inputs[1].path.as_deref(), Some("/a/shot.jpg"));
    assert_eq!(
        message.image_rows(),
        ["[Image #1: clipboard-1.png]", "[Image #2: shot.jpg]"]
    );
}

#[test]
fn removing_a_messages_copies_leaves_the_persons_files() {
    let directory = tempfile::tempdir().unwrap();
    let pasted = directory.path().join("clipboard-1.png");
    let mine = directory.path().join("mine.png");
    std::fs::write(&pasted, b"png").unwrap();
    std::fs::write(&mine, b"png").unwrap();
    let message = UserMessage {
        text: "[Image #1] [Image #2]".to_string(),
        images: vec![
            image(ImageSource::Pasted(pasted.clone()), "clipboard-1.png"),
            image(ImageSource::File(mine.clone()), "mine.png"),
        ],
    };
    message.remove_pasted_files();
    assert!(!pasted.exists());
    assert!(mine.exists());
}
