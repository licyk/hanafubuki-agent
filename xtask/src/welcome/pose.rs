use std::{fs, path::Path};

use image::{imageops, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use super::Metadata;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Layout {
    horizontal: f32,
    vertical: f32,
    width_scale: f32,
    min_rows: u16,
    max_rows: u16,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            horizontal: 0.5,
            vertical: 1.0,
            width_scale: 1.2,
            min_rows: 18,
            max_rows: 40,
        }
    }
}

pub(super) fn metadata(source: &Path) -> Result<Metadata> {
    let path = source.join("pose.json");
    if !path.exists() {
        return Ok(Metadata::default());
    }
    let meta: Metadata = serde_json::from_slice(&fs::read(path)?)?;
    let layout = &meta.layout;
    if !(32..=512).contains(&meta.width)
        || !(32..=512).contains(&meta.height)
        || meta.fps != super::FPS
        || meta.frames != super::FRAMES
        || meta.columns != super::COLUMNS
        || !(0.0..=1.0).contains(&layout.horizontal)
        || !(0.0..=1.0).contains(&layout.vertical)
        || !(0.5..=2.0).contains(&layout.width_scale)
        || layout.min_rows == 0
        || layout.min_rows > layout.max_rows
        || layout.max_rows > 60
    {
        return Err("invalid pose dimensions, timing or normalized placement in pose.json".into());
    }
    Ok(meta)
}

pub(super) fn prepare(mut source: RgbaImage, meta: &Metadata) -> Result<RgbaImage> {
    let (mut left, mut top) = source.dimensions();
    let (mut right, mut bottom) = (0, 0);
    for (x, y, pixel) in source.enumerate_pixels_mut() {
        if pixel[3] < 16 {
            *pixel = Rgba([0; 4]);
        } else {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if left >= right || top >= bottom {
        return Err("base.png contains no visible pixels".into());
    }
    let cropped = imageops::crop_imm(&source, left, top, right - left, bottom - top).to_image();
    let (width, height) = (meta.width * 2, meta.height * 2);
    let scale = (f64::from(width - 12) / f64::from(cropped.width()))
        .min(f64::from(height - 12) / f64::from(cropped.height()));
    let scaled = imageops::resize(
        &cropped,
        (f64::from(cropped.width()) * scale).round() as u32,
        (f64::from(cropped.height()) * scale).round() as u32,
        imageops::FilterType::Lanczos3,
    );
    let mut canvas = RgbaImage::new(width, height);
    let x = (f64::from(width - scaled.width()) * f64::from(meta.layout.horizontal)) as i64;
    imageops::overlay(&mut canvas, &scaled, x, i64::from(height - scaled.height()));
    Ok(canvas)
}
