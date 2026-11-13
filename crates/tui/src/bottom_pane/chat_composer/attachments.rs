// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The draft's images (Codex's `chat_composer/attachment_state.rs`): each
//! an atomic `[Image #N]` element of the text, numbered in the order of
//! `images`. Ctrl+V (`composer.paste_image`) asks the app for the
//! clipboard's image while the composer has the keys and no popup or
//! history search is open; a paste that names one image file attaches the
//! file. Deleting a placeholder drops its image, and a clipboard image's
//! copy with it, and renumbers the rest. A message sent, recalled or put
//! back brings its images; one whose clipboard copy is gone comes back
//! without it (`UserMessage::available`). Codex's remote image rows are not
//! ported.

use hanafubuki_agent::MAXIMUM_USER_IMAGES_PER_MESSAGE;
use ratatui::crossterm::event::{KeyEvent, KeyEventKind};

use super::ChatComposer;
use crate::app_event::AppEvent;
use crate::clipboard::image_file::{self, file_image, Inspection};
use crate::clipboard::paste::normalize_pasted_path;
use crate::history_cell;
use crate::key_hint::is_altgr;
use crate::keymap::{self, Action};
use crate::user_message::{image_label, label_ranges, MessageImage, UserMessage};

impl ChatComposer {
    /// Takes Ctrl+V's key, whether or not it asks for the clipboard now.
    pub(super) fn handle_paste_image_key(&mut self, key: &KeyEvent) -> bool {
        // On Windows Ctrl+Alt is AltGr, which types a character (`@` is
        // AltGr+V on a Hungarian keyboard).
        if key.kind != KeyEventKind::Press
            || is_altgr(key.modifiers)
            || !keymap::matches(Action::PasteImage, key)
        {
            return false;
        }
        if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
            self.handle_paste(pasted);
        }
        if self.takes_images() {
            self.app_event_tx.send(AppEvent::PasteImage);
        }
        true
    }

    /// Whether the clipboard's image goes into this draft: the composer has
    /// the keys, with no popup or history search open.
    pub(crate) fn takes_images(&self) -> bool {
        self.has_focus && !self.popup_active()
    }

    /// Why no image can be attached to the draft now; `Ok` when one can.
    pub(crate) fn image_attachable(&self) -> Result<(), String> {
        if self.images.len() >= MAXIMUM_USER_IMAGES_PER_MESSAGE {
            return Err(format!(
                "A message carries at most {MAXIMUM_USER_IMAGES_PER_MESSAGE} images; remove one \
                 first."
            ));
        }
        if self.textarea.text().starts_with('!') {
            return Err("A `!` command takes no images.".to_string());
        }
        Ok(())
    }

    /// Attaches `image` at the cursor as the next `[Image #N]`.
    pub(crate) fn attach_image(&mut self, image: MessageImage) {
        self.textarea
            .insert_element(&image_label(self.images.len() + 1));
        self.images.push(image);
        self.sync_popups();
    }

    /// A paste that names one image file attaches the file, followed by a
    /// space (Codex); `false` when it names none, and it is text. An image
    /// that cannot be attached is said why, and pasted as text.
    pub(super) fn paste_image_path(&mut self, pasted: &str) -> bool {
        // A `!` command's argument is its own.
        if self.textarea.text().starts_with('!') {
            return false;
        }
        let Some(path) = normalize_pasted_path(pasted, self.wsl) else {
            return false;
        };
        let refusal = match image_file::inspect(&path) {
            Inspection::NotAnImage => return false,
            Inspection::Refused(why) => why,
            Inspection::Image => match self.image_attachable() {
                Ok(()) => {
                    self.attach_image(file_image(&path));
                    self.textarea.insert_str(" ");
                    return true;
                }
                Err(why) => why,
            },
        };
        self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_error_event(refusal),
        )));
        false
    }

    /// Drops the images whose placeholders were deleted, each a removed
    /// element's text, and renumbers the rest.
    pub(super) fn drop_deleted_images(&mut self, removed: &[String]) {
        let mut kept = Vec::with_capacity(self.images.len());
        for (index, image) in std::mem::take(&mut self.images).into_iter().enumerate() {
            if removed.contains(&image_label(index + 1)) {
                image.remove_pasted_file();
            } else {
                kept.push((index + 1, image));
            }
        }
        // Each number only falls, onto one freed before it.
        for (index, (number, _)) in kept.iter().enumerate() {
            if *number != index + 1 {
                self.textarea
                    .replace_element_payload(&image_label(*number), &image_label(index + 1));
            }
        }
        self.images = kept.into_iter().map(|(_, image)| image).collect();
    }

    /// The draft as a message: its text with the large pastes put back,
    /// and its images.
    pub(super) fn draft_message(&self) -> UserMessage {
        UserMessage {
            text: self.current_text_with_pending(),
            images: self.images.clone(),
        }
    }

    /// Replaces the draft with `message`, its images attached, with the
    /// cursor at the end. The draft's own images are let go, not removed:
    /// the history may hold them.
    pub(crate) fn set_message(&mut self, message: UserMessage) {
        self.textarea.set_text_clearing_elements("");
        self.pending_pastes.clear();
        self.images.clear();
        self.prepend_message(message);
        self.textarea.set_cursor(self.textarea.text().len());
        self.sync_popups();
    }

    /// Puts `message` before the draft, on lines of its own; its images
    /// come first, the draft's renumbered after them.
    pub(super) fn prepend_message(&mut self, message: UserMessage) {
        let message = message.available();
        let count = message.images.len();
        // Highest first: each new number is above every old one.
        for number in (1..=self.images.len()).rev() {
            self.textarea
                .replace_element_payload(&image_label(number), &image_label(number + count));
        }
        let mut text = message.text;
        // An image its text does not name (sent by another host) still
        // shows.
        let named = label_ranges(&text, count);
        for number in (1..=count).filter(|number| !named.iter().any(|(_, n)| n == number)) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&image_label(number));
        }
        if !self.textarea.is_empty() {
            text.push('\n');
        }
        self.textarea.insert_str_at(0, &text);
        for (range, _) in label_ranges(&text, count) {
            self.textarea.mark_element(range);
        }
        self.images.splice(0..0, message.images);
    }
}

#[cfg(test)]
mod tests;
