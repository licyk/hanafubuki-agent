// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A message the person sends: its text and the images it carries. Each
//! image stands in the text as its label, `[Image #N]`, numbered from 1 in
//! the order of `images` (Codex's `local_image_label_text`): the model
//! reads the text with its labels, then the images in that order, so
//! "compare [Image #2] with [Image #1]" names them. Only a label's first
//! appearance is its image's; a later one, or one past the images, is text.

use std::ops::Range;
use std::path::PathBuf;

use hanafubuki_agent::{AgentImageAttachmentRef, DesktopAgentImageInput};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UserMessage {
    pub(crate) text: String,
    pub(crate) images: Vec<MessageImage>,
}

/// An image a message carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MessageImage {
    pub(crate) source: ImageSource,
    /// What the transcript calls it: its file's name.
    pub(crate) name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImageSource {
    /// A file of the person's, attached where it is: a pasted path, or a
    /// file copied in a file manager. Never removed.
    File(PathBuf),
    /// A copy of a clipboard image in the run's directory
    /// (`clipboard::image_file::ImageDirectory`). Removed once its message
    /// reached the conversation or its placeholder was deleted, and with the
    /// directory when the interface exits.
    Pasted(PathBuf),
    /// One of the session's attachments: an image of an earlier message
    /// being edited.
    Attachment(AgentImageAttachmentRef),
}

/// The label image `number` (from 1) has in a message's text.
pub(crate) fn image_label(number: usize) -> String {
    format!("[Image #{number}]")
}

/// Where the labels of a message with `count` images are in `text`, each
/// label's first appearance, in the text's order, with its number.
pub(crate) fn label_ranges(text: &str, count: usize) -> Vec<(Range<usize>, usize)> {
    let mut seen = vec![false; count];
    labels(text)
        .filter(|(_, number)| {
            (1..=count).contains(number) && !std::mem::replace(&mut seen[number - 1], true)
        })
        .collect()
}

/// Every `[Image #N]` in `text`.
fn labels(text: &str) -> impl Iterator<Item = (Range<usize>, usize)> + '_ {
    const OPEN: &str = "[Image #";
    text.match_indices(OPEN).filter_map(move |(start, _)| {
        let rest = &text[start + OPEN.len()..];
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 || !rest[digits..].starts_with(']') {
            return None;
        }
        let number = rest[..digits].parse().ok()?;
        Some((start..start + OPEN.len() + digits + 1, number))
    })
}

impl From<&str> for UserMessage {
    fn from(text: &str) -> Self {
        Self::from_text(text)
    }
}

impl UserMessage {
    /// A message with no image.
    pub(crate) fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }

    /// A message read back from the history store, which keeps its text
    /// only: the labels of the images it had are dropped, so none is sent
    /// as text standing for an image that is not there.
    pub(crate) fn stored(text: &str) -> Self {
        Self::from_text(replace_labels(text, labels(text).collect(), |_| None))
    }

    /// Messages sent as one, a line each: each one's images follow the
    /// ones before, its labels renumbered after theirs.
    pub(crate) fn join(messages: impl IntoIterator<Item = UserMessage>) -> Self {
        let mut joined = Self::default();
        for (index, message) in messages.into_iter().enumerate() {
            if index > 0 {
                joined.text.push('\n');
            }
            let offset = joined.images.len();
            joined
                .text
                .push_str(&message.renumbered(|number| Some(number + offset)));
            joined.images.extend(message.images);
        }
        joined
    }

    /// The message with the clipboard images whose copy is gone (sent
    /// before, or deleted) left out, their labels with them, and the rest
    /// renumbered.
    pub(crate) fn available(self) -> Self {
        let kept: Vec<bool> = self
            .images
            .iter()
            .map(|image| match &image.source {
                ImageSource::Pasted(path) => path.is_file(),
                ImageSource::File(_) | ImageSource::Attachment(_) => true,
            })
            .collect();
        if kept.iter().all(|kept| *kept) {
            return self;
        }
        let mut numbers = Vec::with_capacity(kept.len());
        let mut next = 0;
        for kept in &kept {
            numbers.push(kept.then(|| {
                next += 1;
                next
            }));
        }
        let text = self.renumbered(|number| numbers[number - 1]);
        let images = self
            .images
            .into_iter()
            .zip(kept)
            .filter_map(|(image, kept)| kept.then_some(image))
            .collect();
        Self { text, images }
    }

    /// The text with each image's label renumbered by `number`, or dropped
    /// where it gives none.
    fn renumbered(&self, number: impl Fn(usize) -> Option<usize>) -> String {
        replace_labels(
            &self.text,
            label_ranges(&self.text, self.images.len()),
            number,
        )
    }

    /// The images as the Agent reads them.
    pub(crate) fn request_images(&self) -> Vec<DesktopAgentImageInput> {
        self.images
            .iter()
            .map(|image| {
                let (path, attachment) = match &image.source {
                    // Attached only when it is UTF-8 (`image_file::inspect`).
                    ImageSource::File(path) | ImageSource::Pasted(path) => {
                        (Some(path.to_string_lossy().into_owned()), None)
                    }
                    ImageSource::Attachment(attachment) => (None, Some(attachment.clone())),
                };
                DesktopAgentImageInput {
                    path,
                    attachment,
                    display_name: image.name.clone(),
                    detail: Default::default(),
                }
            })
            .collect()
    }

    /// Removes the copies of its clipboard images: the message reached the
    /// conversation, which keeps its own.
    pub(crate) fn remove_pasted_files(&self) {
        for image in &self.images {
            image.remove_pasted_file();
        }
    }

    /// What the transcript shows of its images, `[Image #N: name]` each.
    pub(crate) fn image_rows(&self) -> Vec<String> {
        image_rows(self.images.iter().map(|image| image.name.as_str()))
    }
}

impl MessageImage {
    /// Removes its copy when it is a clipboard image's; a file of the
    /// person's is never touched.
    pub(crate) fn remove_pasted_file(&self) {
        if let ImageSource::Pasted(path) = &self.source {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `[Image #N: name]` for each of `names`, numbered from 1.
pub(crate) fn image_rows<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| format!("[Image #{}: {name}]", index + 1))
        .collect()
}

/// `text` with each of `ranges`' labels replaced by `number`'s, or removed
/// where it gives none.
fn replace_labels(
    text: &str,
    ranges: Vec<(Range<usize>, usize)>,
    number: impl Fn(usize) -> Option<usize>,
) -> String {
    let mut result = String::with_capacity(text.len());
    let mut copied = 0;
    for (range, old) in ranges {
        result.push_str(&text[copied..range.start]);
        if let Some(new) = number(old) {
            result.push_str(&image_label(new));
        }
        copied = range.end;
    }
    result.push_str(&text[copied..]);
    result
}

#[cfg(test)]
mod tests;
