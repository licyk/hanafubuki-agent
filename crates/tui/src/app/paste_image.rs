// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Ctrl+V (`composer.paste_image`, Codex's `App::handle_clipboard_key_action`
//! and `ChatWidget::paste_image`): the clipboard's image attached to the
//! composer's draft. An image's pixels are encoded to PNG into the run's
//! directory (`ImageDirectory`); an image file copied in a file manager is
//! attached where it is. A clipboard with text and no image pastes the
//! text, as the terminal's paste would have (Codex's fallback where the
//! console delivers Ctrl+V as a key); one with neither, or none to read,
//! says so. A key held down repeats: a press within 250 ms of the last read
//! is ignored (Codex's window).
//!
//! An empty bracketed paste, which some terminals send for a clipboard
//! holding an image, tries the clipboard's image too, saying nothing when
//! there is none (Codex tried this and reverted it, 749b58366c).

use std::time::{Duration, Instant};

use super::App;
use crate::clipboard::image_file::{self, file_image, ImageDirectory, Inspection};
use crate::clipboard::paste::{read_clipboard, ClipboardContent, ClipboardError, ReadClipboard};
use crate::user_message::{ImageSource, MessageImage};

const REPEAT_WINDOW: Duration = Duration::from_millis(250);

/// The app's side of pasting images.
pub(super) struct ImagePaste {
    /// How the clipboard is read; the tests give their own.
    pub(super) read: ReadClipboard,
    /// Where the clipboard images' copies are kept for the run.
    pub(super) directory: ImageDirectory,
    /// When the clipboard was last read for an image.
    pub(super) last: Option<Instant>,
}

impl Default for ImagePaste {
    fn default() -> Self {
        Self {
            read: read_clipboard,
            directory: ImageDirectory::default(),
            last: None,
        }
    }
}

/// What asked for the clipboard's image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PasteTrigger {
    Key,
    EmptyPaste,
}

impl App {
    pub(super) fn paste_image(&mut self, trigger: PasteTrigger) {
        let now = Instant::now();
        let repeated = self
            .image_paste
            .last
            .is_some_and(|last| now.duration_since(last) < REPEAT_WINDOW);
        if (trigger == PasteTrigger::Key && repeated) || !self.chat_widget.composer_takes_images() {
            return;
        }
        let read = (self.image_paste.read)();
        self.image_paste.last = Some(Instant::now());
        if matches!(
            read,
            Ok(ClipboardContent::Pixels(_) | ClipboardContent::ImageFile(_))
        ) {
            if let Err(why) = self.chat_widget.image_attachable() {
                return self.chat_widget.add_error_message(why);
            }
        }
        let attached = match read {
            Ok(ClipboardContent::Pixels(pixels)) => self.save_pasted_image(&pixels),
            Ok(ClipboardContent::ImageFile(path)) => match image_file::inspect(&path) {
                Inspection::Image => Ok(file_image(&path)),
                Inspection::Refused(why) => Err(why),
                Inspection::NotAnImage => Err(format!(
                    "{} on the clipboard is not an image.",
                    image_file::file_name(&path)
                )),
            },
            Ok(ClipboardContent::Text(text)) => {
                if trigger == PasteTrigger::Key {
                    self.chat_widget.handle_paste(text);
                }
                return;
            }
            Err(_) if trigger == PasteTrigger::EmptyPaste => return,
            Err(ClipboardError::Empty) => Err("There is no image on the clipboard.".to_string()),
            Err(ClipboardError::Unavailable(why)) => Err(format!(
                "The clipboard cannot be read ({why}); save the image to a file and paste its \
                 path instead."
            )),
        };
        match attached {
            Ok(image) => self.chat_widget.attach_image(image),
            Err(why) => self.chat_widget.add_error_message(why),
        }
    }

    /// The clipboard's image as a PNG in the run's directory.
    fn save_pasted_image(&mut self, pixels: &image::RgbaImage) -> Result<MessageImage, String> {
        let png = image_file::encode_png(pixels)?;
        let path = self.image_paste.directory.save(&png).map_err(|error| {
            format!("The clipboard's image could not be saved to a temporary file: {error}.")
        })?;
        Ok(MessageImage {
            name: image_file::file_name(&path),
            source: ImageSource::Pasted(path),
        })
    }
}
