// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a pasted image must be to be attached, and where a clipboard
//! image's copy is kept. The Agent stages a message's images under limits
//! (`prepare_and_stage_images`: PNG, JPEG, GIF or WebP, at most
//! `MAXIMUM_SOURCE_IMAGE_BYTES` and `MAXIMUM_IMAGE_PIXELS` each); checked
//! here at the paste, an image it would refuse is refused at once, and no
//! message is sent only to fail. A clipboard image is encoded to PNG into a
//! directory the run creates in the system's temporary directory when it
//! first needs it, removed with what it holds when the interface exits.

use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use hanafubuki_agent::{MAXIMUM_IMAGE_PIXELS, MAXIMUM_SOURCE_IMAGE_BYTES};
use image::{ImageFormat, ImageReader};

use crate::user_message::{ImageSource, MessageImage};

/// What a file is to an image paste.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Inspection {
    /// An image that can be attached.
    Image,
    /// Not an image (or not a file): a pasted path stays text.
    NotAnImage,
    /// An image that cannot be attached, and why.
    Refused(String),
}

/// Whether `path` is a file whose content reads as an image.
pub(crate) fn is_image(path: &Path) -> bool {
    path.is_file() && guessed_reader(path).is_some_and(|reader| reader.format().is_some())
}

/// Whether the file at `path` can be attached as an image.
pub(crate) fn inspect(path: &Path) -> Inspection {
    // A FIFO or a device would block the read.
    let Some(metadata) = path.metadata().ok().filter(|metadata| metadata.is_file()) else {
        return Inspection::NotAnImage;
    };
    let Some(reader) = guessed_reader(path) else {
        return Inspection::NotAnImage;
    };
    let name = file_name(path);
    match reader.format() {
        None => return Inspection::NotAnImage,
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP) => {}
        Some(_) => {
            return Inspection::Refused(format!(
                "{name} is not a PNG, JPEG, GIF or WebP image, the formats a model is sent."
            ))
        }
    }
    if path.to_str().is_none() {
        return Inspection::Refused(format!("The path of {name} is not UTF-8."));
    }
    if metadata.len() > MAXIMUM_SOURCE_IMAGE_BYTES as u64 {
        return Inspection::Refused(format!(
            "{name} is too large to attach: {}, at most {}.",
            megabytes(metadata.len()),
            megabytes(MAXIMUM_SOURCE_IMAGE_BYTES as u64)
        ));
    }
    match reader.into_dimensions() {
        Ok((width, height)) => match check_pixels(width, height) {
            Ok(()) => Inspection::Image,
            Err(why) => Inspection::Refused(format!("{name} {why}")),
        },
        Err(error) => Inspection::Refused(format!("{name} cannot be read as an image: {error}.")),
    }
}

/// `Err` with why when an image of `width` by `height` is too large.
pub(crate) fn check_pixels(width: u32, height: u32) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 {
        return Err("is empty (no pixels).".to_string());
    }
    if pixels > MAXIMUM_IMAGE_PIXELS {
        return Err(format!(
            "is too large to attach: {width}×{height} pixels, at most {} million.",
            MAXIMUM_IMAGE_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

/// `image` as PNG, refused when it is larger than an image the Agent reads.
pub(crate) fn encode_png(image: &image::RgbaImage) -> Result<Vec<u8>, String> {
    check_pixels(image.width(), image.height())
        .map_err(|why| format!("The clipboard's image {why}"))?;
    let mut png = io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| format!("The clipboard's image could not be encoded: {error}."))?;
    let png = png.into_inner();
    if png.len() > MAXIMUM_SOURCE_IMAGE_BYTES {
        return Err(format!(
            "The clipboard's image is too large to attach: {} as PNG, at most {}.",
            megabytes(png.len() as u64),
            megabytes(MAXIMUM_SOURCE_IMAGE_BYTES as u64)
        ));
    }
    Ok(png)
}

/// The run's directory of clipboard images.
#[derive(Default)]
pub(crate) struct ImageDirectory {
    directory: Option<tempfile::TempDir>,
    saved: usize,
}

impl ImageDirectory {
    /// Writes `png` as the next `clipboard-N.png`, creating the directory
    /// the first time.
    pub(crate) fn save(&mut self, png: &[u8]) -> io::Result<PathBuf> {
        let directory = match &mut self.directory {
            Some(directory) => directory,
            empty => empty.insert(
                tempfile::Builder::new()
                    .prefix("hanafubuki-agent-images-")
                    .tempdir()?,
            ),
        };
        self.saved += 1;
        let path = directory
            .path()
            .join(format!("clipboard-{}.png", self.saved));
        std::fs::write(&path, png)?;
        Ok(path)
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> Option<&Path> {
        self.directory.as_ref().map(tempfile::TempDir::path)
    }
}

/// A file of the person's, attached where it is, by its absolute path.
pub(crate) fn file_image(path: &Path) -> MessageImage {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    MessageImage {
        name: file_name(&path),
        source: ImageSource::File(path),
    }
}

/// What the transcript calls the image at `path`: its file's name.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string())
}

/// A reader of `path` whose format is the content's, never the name's.
fn guessed_reader(path: &Path) -> Option<ImageReader<BufReader<File>>> {
    let mut reader = ImageReader::open(path).ok()?;
    reader.clear_format();
    reader.with_guessed_format().ok()
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests;
