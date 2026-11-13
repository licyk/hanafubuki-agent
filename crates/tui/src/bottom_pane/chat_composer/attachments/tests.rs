// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The draft's images (Codex's attachment, paste-path and
//! `apply_external_edit` image tests): placeholders, their deletion and
//! renumbering, pasted paths, the limits, and the draft's images through
//! submission, recall, Ctrl+R, a message put back and Ctrl+G.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{ChatComposer, InputResult};
use crate::app_event::{AppEvent, AppEventSender};
use crate::keymap::{self, Keymap};
use crate::user_message::{ImageSource, MessageImage, UserMessage};

fn new_composer() -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    let composer = ChatComposer::new(true, AppEventSender::new(tx), false, String::new());
    (composer, rx)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn asked(events: &Receiver<AppEvent>) -> bool {
    events
        .try_iter()
        .any(|event| matches!(event, AppEvent::PasteImage))
}

fn errors(events: &Receiver<AppEvent>) -> Vec<String> {
    events
        .try_iter()
        .filter_map(|event| match event {
            AppEvent::InsertHistoryCell(cell) => Some(
                cell.display_lines(200)
                    .iter()
                    .map(|line| line.to_string())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect()
}

fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// A clipboard image's copy at `directory/name`.
fn pasted(directory: &Path, name: &str) -> MessageImage {
    let path = directory.join(name);
    std::fs::write(&path, png()).unwrap();
    MessageImage {
        source: ImageSource::Pasted(path),
        name: name.to_string(),
    }
}

fn names(images: &[MessageImage]) -> Vec<&str> {
    images.iter().map(|image| image.name.as_str()).collect()
}

fn submit(composer: &mut ChatComposer) -> UserMessage {
    match composer.handle_key_event(key(KeyCode::Enter)).0 {
        InputResult::Submitted(message) => message,
        other => panic!("not sent: {other:?}"),
    }
}

#[test]
fn ctrl_v_and_ctrl_alt_v_ask_for_the_clipboards_image() {
    let (mut composer, events) = new_composer();
    composer.set_text_content("a draft".to_string());
    composer.handle_key_event(ctrl('v'));
    assert!(asked(&events));
    assert_eq!(composer.textarea.text(), "a draft");

    // Ctrl+Alt+V too, but on Windows, where Ctrl+Alt is AltGr: there it is
    // the composer's to type, and asks for nothing.
    let (mut typed, typed_events) = new_composer();
    typed.handle_key_event(KeyEvent::new(
        KeyCode::Char('v'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ));
    assert_eq!(asked(&typed_events), cfg!(not(windows)));

    // Not with a popup or the history search open, nor without the keys.
    composer.set_text_content("/mo".to_string());
    composer.handle_key_event(ctrl('v'));
    assert!(!asked(&events));
    assert_eq!(composer.textarea.text(), "/mo");
    composer.set_text_content(String::new());
    composer.handle_key_event(ctrl('r'));
    composer.handle_key_event(ctrl('v'));
    assert!(!asked(&events));
    composer.cancel_history_search();
    composer.set_has_focus(false);
    composer.handle_key_event(ctrl('v'));
    assert!(!asked(&events));
}

#[test]
fn the_paste_image_key_can_be_rebound() {
    let config = BTreeMap::from([(
        "composer".to_string(),
        BTreeMap::from([("paste_image".to_string(), vec!["f9".to_string()])]),
    )]);
    keymap::install(Keymap::resolve(&config).0);
    let (mut composer, events) = new_composer();
    composer.handle_key_event(key(KeyCode::F(9)));
    assert!(asked(&events));
    composer.handle_key_event(ctrl('v'));
    assert!(!asked(&events));
    keymap::install(Keymap::default());
}

#[test]
fn images_are_numbered_and_a_deleted_one_renumbers_the_rest() {
    let directory = tempfile::tempdir().unwrap();
    let (mut composer, _events) = new_composer();
    composer.set_text_content("compare ".to_string());
    composer.attach_image(pasted(directory.path(), "one.png"));
    composer.textarea.insert_str(" with ");
    composer.attach_image(pasted(directory.path(), "two.png"));
    composer.textarea.insert_str(" and ");
    composer.attach_image(pasted(directory.path(), "three.png"));
    assert_eq!(
        composer.textarea.text(),
        "compare [Image #1] with [Image #2] and [Image #3]"
    );

    // Backspace at the end of the first placeholder deletes it whole.
    composer.textarea.set_cursor("compare [Image #1]".len());
    composer.handle_key_event(key(KeyCode::Backspace));
    assert_eq!(
        composer.textarea.text(),
        "compare  with [Image #1] and [Image #2]"
    );
    assert_eq!(names(&composer.images), ["two.png", "three.png"]);
    // Its clipboard copy goes with it.
    assert!(!directory.path().join("one.png").exists());
    assert!(directory.path().join("two.png").exists());
    assert_eq!(
        composer.textarea.element_payloads(),
        ["[Image #1]", "[Image #2]"]
    );

    let message = submit(&mut composer);
    assert_eq!(message.text, "compare  with [Image #1] and [Image #2]");
    assert_eq!(names(&message.images), ["two.png", "three.png"]);
    assert!(composer.images.is_empty());
    assert!(composer.is_empty());
}

#[test]
fn deleting_a_persons_file_leaves_it_and_an_image_alone_is_a_message() {
    let directory = tempfile::tempdir().unwrap();
    let mine = directory.path().join("mine.png");
    std::fs::write(&mine, png()).unwrap();
    let (mut composer, _events) = new_composer();
    composer.attach_image(MessageImage {
        source: ImageSource::File(mine.clone()),
        name: "mine.png".to_string(),
    });
    composer.handle_key_event(key(KeyCode::Backspace));
    assert!(composer.is_empty());
    assert!(composer.images.is_empty());
    assert!(mine.exists());

    composer.attach_image(pasted(directory.path(), "only.png"));
    let message = submit(&mut composer);
    assert_eq!(message.text, "[Image #1]");
    assert_eq!(names(&message.images), ["only.png"]);
}

#[test]
fn a_pasted_image_path_attaches_the_file_in_each_form() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("my shot.png");
    std::fs::write(&path, png()).unwrap();
    let plain = path.to_string_lossy().into_owned();
    let mut forms = vec![format!("\"{plain}\""), format!("'{plain}'")];
    if cfg!(not(windows)) {
        forms.push(plain.replace(' ', "\\ "));
        forms.push(url::Url::from_file_path(&path).unwrap().to_string());
    }
    for form in forms {
        let (mut composer, events) = new_composer();
        composer.set_text_content("see ".to_string());
        composer.handle_paste(form.clone());
        assert_eq!(composer.textarea.text(), "see [Image #1] ", "{form}");
        assert_eq!(names(&composer.images), ["my shot.png"]);
        assert_eq!(composer.images[0].source, ImageSource::File(path.clone()));
        assert!(errors(&events).is_empty());
    }
}

#[test]
fn a_path_that_is_no_image_or_cannot_be_attached_is_pasted_as_text() {
    let directory = tempfile::tempdir().unwrap();
    let notes = directory.path().join("notes.txt");
    std::fs::write(&notes, "text").unwrap();
    let notes = notes.to_string_lossy().into_owned();
    let (mut composer, events) = new_composer();
    composer.handle_paste(notes.clone());
    assert_eq!(composer.textarea.text(), notes);
    assert!(composer.images.is_empty());
    assert!(errors(&events).is_empty());

    // An image of a format the Agent does not send: said why, and text.
    let bitmap = directory.path().join("scan.bmp");
    let mut bytes = b"BM".to_vec();
    bytes.resize(64, 0);
    std::fs::write(&bitmap, bytes).unwrap();
    let bitmap = bitmap.to_string_lossy().into_owned();
    let (mut composer, events) = new_composer();
    composer.handle_paste(bitmap.clone());
    assert_eq!(composer.textarea.text(), bitmap);
    let said = errors(&events);
    assert!(
        said[0].contains("scan.bmp is not a PNG, JPEG, GIF or WebP image"),
        "{said:?}"
    );

    // A `!` command's argument stays its own.
    let image = directory.path().join("a.png");
    std::fs::write(&image, png()).unwrap();
    let image = image.to_string_lossy().into_owned();
    let (mut composer, _events) = new_composer();
    composer.set_text_content("!file ".to_string());
    composer.handle_paste(image.clone());
    assert_eq!(composer.textarea.text(), format!("!file {image}"));
    assert!(composer.images.is_empty());
}

#[test]
fn a_message_carries_at_most_eight_images_and_a_command_none() {
    let directory = tempfile::tempdir().unwrap();
    let (mut composer, events) = new_composer();
    for index in 0..8 {
        assert_eq!(composer.image_attachable(), Ok(()));
        composer.attach_image(pasted(directory.path(), &format!("{index}.png")));
    }
    assert_eq!(
        composer.image_attachable(),
        Err("A message carries at most 8 images; remove one first.".to_string())
    );
    // A ninth pasted path is refused, and stays text.
    let ninth = directory.path().join("ninth.png");
    std::fs::write(&ninth, png()).unwrap();
    composer.handle_paste(ninth.to_string_lossy().into_owned());
    assert_eq!(composer.images.len(), 8);
    assert!(errors(&events)[0].contains("at most 8 images"));

    let (mut composer, _events) = new_composer();
    composer.set_text_content("!ls".to_string());
    assert_eq!(
        composer.image_attachable(),
        Err("A `!` command takes no images.".to_string())
    );
}

#[test]
fn a_message_put_back_goes_before_the_draft_its_images_first() {
    let directory = tempfile::tempdir().unwrap();
    let (mut composer, _events) = new_composer();
    composer.set_text_content("draft ".to_string());
    composer.attach_image(pasted(directory.path(), "draft.png"));
    composer.restore_draft(UserMessage {
        text: "waiting [Image #1] [Image #2]".to_string(),
        images: vec![
            pasted(directory.path(), "a.png"),
            pasted(directory.path(), "b.png"),
        ],
    });
    assert_eq!(
        composer.textarea.text(),
        "waiting [Image #1] [Image #2]\ndraft [Image #3]"
    );
    assert_eq!(names(&composer.images), ["a.png", "b.png", "draft.png"]);
    assert_eq!(
        composer.textarea.element_payloads(),
        ["[Image #1]", "[Image #2]", "[Image #3]"]
    );

    // An image its text does not name (another host's message) still
    // shows.
    composer.set_message(UserMessage {
        text: "edited".to_string(),
        images: vec![pasted(directory.path(), "c.png")],
    });
    assert_eq!(composer.textarea.text(), "edited [Image #1]");
    assert_eq!(composer.textarea.element_payloads(), ["[Image #1]"]);
}

#[test]
fn a_recalled_message_brings_its_images_back_but_not_a_copy_gone() {
    let directory = tempfile::tempdir().unwrap();
    let (mut composer, _events) = new_composer();
    composer.attach_image(pasted(directory.path(), "kept.png"));
    composer.textarea.insert_str(" and ");
    composer.attach_image(pasted(directory.path(), "sent.png"));
    let sent = submit(&mut composer);

    composer.handle_key_event(key(KeyCode::Up));
    assert_eq!(composer.textarea.text(), "[Image #1] and [Image #2]");
    assert_eq!(names(&composer.images), ["kept.png", "sent.png"]);
    composer.handle_key_event(key(KeyCode::Down));
    assert!(composer.is_empty());

    // Its copy removed once the conversation took it: left out on recall.
    let ImageSource::Pasted(path) = &sent.images[1].source else {
        panic!("a clipboard image");
    };
    std::fs::remove_file(path).unwrap();
    composer.handle_key_event(key(KeyCode::Up));
    assert_eq!(composer.textarea.text(), "[Image #1] and ");
    assert_eq!(names(&composer.images), ["kept.png"]);
}

#[test]
fn ctrl_r_shows_a_match_with_its_images_and_esc_gives_the_draft_back() {
    let directory = tempfile::tempdir().unwrap();
    let (mut composer, _events) = new_composer();
    composer.set_text_content("look ".to_string());
    composer.attach_image(pasted(directory.path(), "old.png"));
    submit(&mut composer);

    composer.set_text_content("mine ".to_string());
    composer.attach_image(pasted(directory.path(), "draft.png"));
    composer.handle_key_event(ctrl('r'));
    composer.on_recent_messages(vec!["look again [Image #1]".to_string()]);
    for c in "look".chars() {
        composer.handle_key_event(key(KeyCode::Char(c)));
    }
    assert_eq!(composer.textarea.text(), "look [Image #1]");
    assert_eq!(names(&composer.images), ["old.png"]);
    composer.handle_key_event(key(KeyCode::Esc));
    assert_eq!(composer.textarea.text(), "mine [Image #1]");
    assert_eq!(names(&composer.images), ["draft.png"]);

    // Taken: the match is the draft, its image with it; a stored one has
    // its labels dropped, its images not being read back.
    composer.handle_key_event(ctrl('r'));
    for c in "look".chars() {
        composer.handle_key_event(key(KeyCode::Char(c)));
    }
    composer.handle_key_event(key(KeyCode::Enter));
    assert_eq!(names(&composer.images), ["old.png"]);
    composer.handle_key_event(ctrl('r'));
    for c in "again".chars() {
        composer.handle_key_event(key(KeyCode::Char(c)));
    }
    composer.handle_key_event(key(KeyCode::Enter));
    assert_eq!(composer.textarea.text(), "look again ");
    assert!(composer.images.is_empty());
}

#[test]
fn ctrl_g_keeps_an_unchanged_placeholder_and_drops_a_deleted_ones_image() {
    let directory = tempfile::tempdir().unwrap();
    let (mut composer, _events) = new_composer();
    composer.set_text_content("a ".to_string());
    composer.attach_image(pasted(directory.path(), "one.png"));
    composer.textarea.insert_str(" b ");
    composer.attach_image(pasted(directory.path(), "two.png"));
    let seed = composer.current_text_with_pending();
    assert_eq!(seed, "a [Image #1] b [Image #2]");

    // Unchanged: both stay.
    composer.apply_external_edit(format!("{seed}\n"));
    assert_eq!(composer.textarea.text(), seed);
    assert_eq!(names(&composer.images), ["one.png", "two.png"]);
    assert_eq!(composer.textarea.element_payloads().len(), 2);

    // Moved: both stay where they now are.
    composer.apply_external_edit("[Image #2] before [Image #1]".to_string());
    assert_eq!(composer.textarea.text(), "[Image #2] before [Image #1]");
    assert_eq!(names(&composer.images), ["one.png", "two.png"]);

    // The first deleted, the second written twice: its image once, the
    // copy of the first removed, and the rest numbered again.
    composer.apply_external_edit("only [Image #2], [Image #2]".to_string());
    assert_eq!(composer.textarea.text(), "only [Image #1], [Image #2]");
    assert_eq!(composer.textarea.element_payloads(), ["[Image #1]"]);
    assert_eq!(names(&composer.images), ["two.png"]);
    assert!(!directory.path().join("one.png").exists());
    assert!(directory.path().join("two.png").exists());
    let message = submit(&mut composer);
    assert_eq!(names(&message.images), ["two.png"]);
}

#[test]
fn a_placeholder_typed_is_text_and_attaches_nothing() {
    let (mut composer, _events) = new_composer();
    composer.handle_paste("[Image #1]".to_string());
    let message = submit(&mut composer);
    assert_eq!(message.text, "[Image #1]");
    assert!(message.images.is_empty());
}
