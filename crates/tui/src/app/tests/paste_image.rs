//! Pasting an image end to end, in Scrollback and Fullscreen: a clipboard
//! (a stand-in for the system's, which tests cannot hold) with an image
//! gives the draft `[Image #1]`, the image reaches the model's request read
//! from its copy outside the workspace, the copy is removed once the turn
//! is accepted, and the transcript names it. A clipboard with text pastes
//! the text; with nothing, or none to read, Ctrl+V says why; an empty
//! bracketed paste attaches an image and is otherwise quiet. An earlier
//! message edited comes back with its images.

use std::cell::RefCell;
use std::io::Cursor;
use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::app_event::AppEvent;
use crate::clipboard::paste::{ClipboardContent, ClipboardError};
use crate::transcript_mode::TranscriptMode;

thread_local! {
    /// What the next clipboard read finds; empty once read.
    static CLIPBOARD: RefCell<Option<Result<ClipboardContent, ClipboardError>>> =
        const { RefCell::new(None) };
}

fn fake_clipboard() -> Result<ClipboardContent, ClipboardError> {
    CLIPBOARD
        .with(|clipboard| clipboard.borrow_mut().take())
        .unwrap_or(Err(ClipboardError::Empty))
}

fn copy(content: Result<ClipboardContent, ClipboardError>) {
    CLIPBOARD.with(|clipboard| *clipboard.borrow_mut() = Some(content));
}

fn pixels() -> ClipboardContent {
    ClipboardContent::Pixels(image::RgbaImage::from_pixel(
        3,
        2,
        image::Rgba([250, 10, 10, 255]),
    ))
}

impl Harness {
    fn with_fake_clipboard(mut self) -> Self {
        self.app.image_paste.read = fake_clipboard;
        self
    }

    /// Ctrl+V, its request handled: a press within the repeat window
    /// would be taken for the key held down, so it is not one.
    fn paste_image(&mut self) {
        self.app.image_paste.last = None;
        self.key_with(KeyCode::Char('v'), KeyModifiers::CONTROL);
        self.pump_until("the clipboard to be read", |_| {
            CLIPBOARD.with(|clipboard| clipboard.borrow().is_none())
        });
    }

    /// Enter after a pause, so the keys before it are not a paste.
    fn send_after_a_pause(&mut self) {
        std::thread::sleep(Duration::from_millis(100));
        self.app.draw(&mut self.tui).unwrap();
        self.key(KeyCode::Enter);
    }

    fn request_body(&self, index: usize) -> String {
        String::from_utf8_lossy(&self.server.requests()[index].body).into_owned()
    }
}

/// The base64 of a request's PNG images.
fn png_data_urls(body: &str) -> usize {
    body.matches("data:image/png;base64,").count()
}

fn an_image_is_pasted_sent_and_its_copy_removed(mode: TranscriptMode) {
    let mut harness = Harness::start_with(vec![assistant_events("A red image.")], true, mode)
        .with_fake_clipboard();
    harness.pump_until_screen_shows("fixture-model");
    copy(Ok(pixels()));
    harness.paste_image();
    assert_eq!(harness.composer_text(), "[Image #1]");
    let copy_path = harness
        .app
        .image_paste
        .directory
        .path()
        .unwrap()
        .join("clipboard-1.png");
    assert!(copy_path.is_file());
    // Outside the workspace: the system's temporary directory.
    assert!(!copy_path.starts_with(harness.home.path()));
    harness.type_text(" what is this?");
    harness.pump_until_screen_shows("[Image #1] what is this?");
    harness.send_after_a_pause();

    harness.pump_until_screen_shows("A red image.");
    let body = harness.request_body(0);
    assert_eq!(png_data_urls(&body), 1, "{body}");
    assert!(body.contains("[Image #1] what is this?"), "{body}");
    let transcript = harness.transcript();
    assert!(
        transcript.contains("[Image #1: clipboard-1.png]"),
        "{transcript}"
    );
    harness.pump_until("the copy to be removed", |_| !copy_path.exists());
}

#[test]
fn an_image_is_pasted_and_sent_in_scrollback() {
    an_image_is_pasted_sent_and_its_copy_removed(TranscriptMode::Terminal);
}

#[test]
fn an_image_is_pasted_and_sent_in_fullscreen() {
    an_image_is_pasted_sent_and_its_copy_removed(TranscriptMode::Owned);
}

fn each_clipboard_says_what_it_holds(mode: TranscriptMode) {
    let mut harness = Harness::start_with(Vec::new(), true, mode).with_fake_clipboard();
    harness.pump_until_screen_shows("fixture-model");

    copy(Err(ClipboardError::Empty));
    harness.paste_image();
    harness.pump_until_screen_shows("There is no image on the clipboard.");

    copy(Err(ClipboardError::Unavailable("no display".to_string())));
    harness.paste_image();
    harness.pump_until_screen_shows(
        "The clipboard cannot be read (no display); save the image to a file and paste its path",
    );

    // Text and no image: pasted, as the terminal's paste would.
    copy(Ok(ClipboardContent::Text("words copied".to_string())));
    harness.paste_image();
    assert_eq!(harness.composer_text(), "words copied");

    // A draft that is a `!` command takes none.
    harness.app.chat_widget.set_composer_text("!ls".to_string());
    copy(Ok(pixels()));
    harness.paste_image();
    harness.pump_until_screen_shows("A `!` command takes no images.");
    assert_eq!(harness.composer_text(), "!ls");
    assert!(harness.app.image_paste.directory.path().is_none());
}

#[test]
fn each_clipboard_says_what_it_holds_in_scrollback() {
    each_clipboard_says_what_it_holds(TranscriptMode::Terminal);
}

#[test]
fn each_clipboard_says_what_it_holds_in_fullscreen() {
    each_clipboard_says_what_it_holds(TranscriptMode::Owned);
}

#[test]
fn an_empty_paste_attaches_the_clipboards_image_and_is_otherwise_quiet() {
    let mut harness = Harness::start(Vec::new()).with_fake_clipboard();
    harness.pump_until_screen_shows("fixture-model");
    copy(Err(ClipboardError::Unavailable("no display".to_string())));
    assert!(harness
        .app
        .handle_event(&mut harness.tui, AppEvent::Paste(String::new())));
    copy(Ok(ClipboardContent::Text("not pasted twice".to_string())));
    assert!(harness
        .app
        .handle_event(&mut harness.tui, AppEvent::Paste(String::new())));
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("clipboard cannot be read"));
    assert_eq!(harness.composer_text(), "");

    copy(Ok(pixels()));
    assert!(harness
        .app
        .handle_event(&mut harness.tui, AppEvent::Paste(String::new())));
    assert_eq!(harness.composer_text(), "[Image #1]");
}

#[test]
fn a_copied_image_file_is_attached_where_it_is_and_kept() {
    let mut harness = Harness::start(vec![assistant_events("A JPEG.")]).with_fake_clipboard();
    harness.pump_until_screen_shows("fixture-model");
    let file = harness.home.path().join("photo.jpg");
    let mut jpeg = Cursor::new(Vec::new());
    image::RgbImage::from_pixel(4, 4, image::Rgb([0, 128, 255]))
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .unwrap();
    std::fs::write(&file, jpeg.into_inner()).unwrap();
    copy(Ok(ClipboardContent::ImageFile(file.clone())));
    harness.paste_image();
    assert_eq!(harness.composer_text(), "[Image #1]");
    harness.send_after_a_pause();
    harness.pump_until_screen_shows("A JPEG.");
    harness.pump_until_turn_ends();
    // The Agent sends what it normalised; the person's file stays.
    assert_eq!(png_data_urls(&harness.request_body(0)), 1);
    assert!(harness.transcript().contains("[Image #1: photo.jpg]"));
    assert!(file.is_file());
}

#[test]
fn an_edited_message_comes_back_with_its_images() {
    let mut harness = Harness::owned(vec![
        assistant_events("First answer."),
        assistant_events("Again."),
    ])
    .with_fake_clipboard();
    harness.pump_until_screen_shows("fixture-model");
    copy(Ok(pixels()));
    harness.paste_image();
    harness.type_text(" first");
    harness.send_after_a_pause();
    harness.pump_until_screen_shows("First answer.");
    harness.pump_until_turn_ends();

    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("the original is archived");
    assert!(!screen.contains("image(s)"), "{screen}");
    assert_eq!(harness.composer_text(), "[Image #1] first");

    // Sent again from the fork, it carries its image, now the fork's.
    harness.send_after_a_pause();
    harness.pump_until_screen_shows("Again.");
    assert_eq!(png_data_urls(&harness.request_body(1)), 1);
}

#[test]
fn a_steer_carries_its_image_whose_copy_goes_once_the_turn_read_it() {
    let mut harness = Harness::start_timed(
        vec![
            (assistant_events("Working on it."), Duration::from_secs(2)),
            (assistant_events("Steered."), Duration::ZERO),
        ],
        true,
        TranscriptMode::Terminal,
    )
    .with_fake_clipboard();
    harness.type_and_submit("Start.");
    harness.pump_until("the turn to run", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    copy(Ok(pixels()));
    harness.paste_image();
    harness.type_text(" also this");
    harness.send_after_a_pause();
    harness.pump_until_screen_shows("Messages to be submitted after next tool call");
    let copy_path = harness
        .app
        .image_paste
        .directory
        .path()
        .unwrap()
        .join("clipboard-1.png");
    assert!(copy_path.is_file());

    harness.pump_until_screen_shows("Steered.");
    let body = harness.request_body(1);
    assert_eq!(png_data_urls(&body), 1, "{body}");
    assert!(body.contains("[Image #1] also this"), "{body}");
    assert!(harness
        .transcript()
        .contains("› [Image #1] also this\n  [Image #1: clipboard-1.png]"));
    harness.pump_until("the copy to be removed", |_| !copy_path.exists());
}

#[test]
fn a_resumed_session_shows_its_messages_images() {
    let mut harness = Harness::start(vec![assistant_events("Seen.")]).with_fake_clipboard();
    harness.pump_until_screen_shows("fixture-model");
    copy(Ok(pixels()));
    harness.paste_image();
    harness.send_after_a_pause();
    harness.pump_until_screen_shows("Seen.");
    harness.pump_until_turn_ends();

    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    harness.type_and_submit("/resume");
    harness.pump_until_screen_shows("Resume a previous session");
    harness.key(KeyCode::Enter);
    harness.pump_until("the resumed transcript", |harness| {
        harness.transcript().contains("Seen.")
    });
    let transcript = harness.transcript();
    assert!(
        transcript.contains("› [Image #1]\n  [Image #1: clipboard-1.png]"),
        "{transcript}"
    );
}
