use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

use color_quant::NeuQuant;
use image::codecs::gif::{GifEncoder, Repeat};
use image::{imageops, Delay, Frame, Rgba, RgbaImage};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{animation, Metadata, COLUMNS, FRAMES, HEIGHT, WIDTH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Serialize)]
struct Evidence {
    #[serde(flatten)]
    metadata: Metadata,
    loop_seconds: u32,
    generator: &'static str,
    method: &'static str,
    sha256: BTreeMap<String, String>,
}

pub(super) fn generate(source: &Path, output: &Path) -> Result<()> {
    let (opened, closed) = animation::source_frames(source)?;
    let mut hashes = BTreeMap::new();
    for name in ["base.png", "blink.png", "prompts.json"] {
        hashes.insert(name.to_string(), digest(&fs::read(source.join(name))?));
    }
    fs::create_dir_all(output)?;
    fs::create_dir_all(source.join("frames"))?;
    let mut atlas = RgbaImage::new(WIDTH * COLUMNS, HEIGHT * FRAMES.div_ceil(COLUMNS));
    let mut contact = RgbaImage::new(384 * 4, 432 * 2);
    let mut gif =
        GifEncoder::new_with_speed(BufWriter::new(File::create(source.join("loop.gif"))?), 30);
    gif.set_repeat(Repeat::Infinite)?;
    let first = animation::frame_at(&opened, &closed, 0);
    for index in 0..FRAMES {
        let rendered = animation::frame_at(&opened, &closed, index);
        if rendered.as_raw()[396 * 384 * 4..] != first.as_raw()[396 * 384 * 4..] {
            return Err(format!("frame {index} moved the resting forearm").into());
        }
        rendered.save(source.join("frames").join(format!("{index:03}.png")))?;
        let small = imageops::resize(&rendered, WIDTH, HEIGHT, imageops::FilterType::Lanczos3);
        imageops::replace(
            &mut atlas,
            &small,
            i64::from(index % COLUMNS * WIDTH),
            i64::from(index / COLUMNS * HEIGHT),
        );
        let mut preview = RgbaImage::from_pixel(384, 432, Rgba([32, 35, 43, 255]));
        imageops::overlay(&mut preview, &rendered, 0, 0);
        if let Some(slot) = [0, 12, 29, 31, 48, 54, 75, 95]
            .iter()
            .position(|frame| *frame == index)
        {
            imageops::replace(
                &mut contact,
                &preview,
                (slot % 4 * 384) as i64,
                (slot / 4 * 432) as i64,
            );
        }
        // GIF has centisecond precision; 80+80+90 ms preserves the 8-second loop.
        let delay = [80, 80, 90][index as usize % 3];
        gif.encode_frame(Frame::from_parts(
            preview,
            0,
            0,
            Delay::from_numer_denom_ms(delay, 1),
        ))?;
    }
    if first != animation::frame_at(&opened, &closed, FRAMES) {
        return Err("the animation loop does not close".into());
    }
    drop(gif);
    let atlas_bytes = indexed_png(&atlas)?;
    fs::write(output.join("atlas.png"), &atlas_bytes)?;
    fs::write(source.join("atlas.png"), &atlas_bytes)?;
    contact.save(source.join("contact-sheet.png"))?;
    write_json(&output.join("atlas.json"), &Metadata::default())?;
    hashes.insert("atlas.png".into(), digest(&atlas_bytes));
    write_json(&source.join("conversion.json"), &Evidence {
        metadata: Metadata::default(),
        loop_seconds: FRAMES / super::FPS,
        generator: "cargo xtask welcome-assets",
        method: "eye-only keyframe composite; anchored normalized mesh; bilinear warp; Lanczos resize; NeuQuant RGBA palette",
        sha256: hashes,
    })?;
    println!("Generated {FRAMES} frames, loop.gif, contact-sheet.png and conversion.json; atlas: {} ({} bytes)", output.join("atlas.png").display(), atlas_bytes.len());
    Ok(())
}

pub(super) fn indexed_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let quantizer = NeuQuant::new(30, 256, image.as_raw());
    let mut palette = quantizer.color_map_rgba();
    // Reserve the transparent color so empty canvas pixels never acquire dots.
    let transparent = quantizer.index_of(&[0; 4]);
    palette[transparent * 4..transparent * 4 + 4].fill(0);
    let indices: Vec<_> = image
        .pixels()
        .map(|pixel| {
            if pixel[3] == 0 {
                transparent as u8
            } else {
                quantizer.index_of(&pixel.0) as u8
            }
        })
        .collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder.set_palette(
            palette
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|color| color[..3].iter().copied())
                .collect::<Vec<_>>(),
        );
        encoder.set_trns(
            palette
                .as_chunks::<4>()
                .0
                .iter()
                .map(|color| color[3])
                .collect::<Vec<_>>(),
        );
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&indices)?;
        writer.finish()?;
    }
    Ok(bytes)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
