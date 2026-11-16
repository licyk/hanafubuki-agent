use std::sync::OnceLock;
use std::time::Duration;

use image::{imageops, RgbaImage};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use serde::Deserialize;

const MIN_ROWS: u16 = 18;
const MAX_ROWS: u16 = 40;
const DOTS: [(u32, u32, u8); 8] = [
    (0, 0, 1),
    (0, 1, 2),
    (0, 2, 4),
    (1, 0, 8),
    (1, 1, 16),
    (1, 2, 32),
    (0, 3, 64),
    (1, 3, 128),
];
const DITHER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

#[derive(Deserialize)]
struct Metadata {
    width: u32,
    height: u32,
    columns: u32,
    frames: u32,
    fps: u32,
}

struct Atlas {
    image: RgbaImage,
    meta: Metadata,
}

fn atlas() -> Option<&'static Atlas> {
    static ATLAS: OnceLock<Option<Atlas>> = OnceLock::new();
    ATLAS
        .get_or_init(|| {
            Some(Atlas {
                image: image::load_from_memory(include_bytes!("../../assets/welcome/atlas.png"))
                    .ok()?
                    .into_rgba8(),
                meta: serde_json::from_slice(include_bytes!("../../assets/welcome/atlas.json"))
                    .ok()?,
            })
        })
        .as_ref()
}

pub(super) fn stage(available: Rect, buf: &Buffer) -> Option<Rect> {
    if available.width < 64 || available.height < MIN_ROWS {
        return None;
    }
    let mut top = available.y;
    for y in available.y..available.bottom() {
        if (available.x..available.right()).any(|x| !buf[(x, y)].symbol().trim().is_empty()) {
            top = y + 1;
        }
    }
    let meta = &atlas()?.meta;
    let rows_by_width =
        u32::from(available.width.saturating_sub(4)) * meta.height / (meta.width * 2);
    let rows = available
        .bottom()
        .saturating_sub(top)
        .min(MAX_ROWS)
        .min(rows_by_width as u16);
    if rows < MIN_ROWS {
        return None;
    }
    let columns = (u32::from(rows) * meta.width * 2 / meta.height) as u16;
    Some(Rect::new(
        available.x + (available.width - columns) / 2,
        available.bottom() - rows,
        columns,
        rows,
    ))
}

#[derive(Default)]
pub(super) struct Renderer {
    cached: Option<(u32, u16, u16, RgbaImage)>,
}

impl Renderer {
    pub(super) fn paint(&mut self, area: Rect, buf: &mut Buffer, elapsed: Duration, opacity: f32) {
        let Some(atlas) = atlas() else { return };
        let meta = &atlas.meta;
        let frame =
            ((elapsed.as_secs_f64() * f64::from(meta.fps)) as u64 % u64::from(meta.frames)) as u32;
        if self.cached.as_ref().is_none_or(|(old, width, height, _)| {
            (*old, *width, *height) != (frame, area.width, area.height)
        }) {
            let source = imageops::crop_imm(
                &atlas.image,
                (frame % meta.columns) * meta.width,
                (frame / meta.columns) * meta.height,
                meta.width,
                meta.height,
            )
            .to_image();
            let scaled = imageops::resize(
                &source,
                u32::from(area.width) * 2,
                u32::from(area.height) * 4,
                imageops::FilterType::Lanczos3,
            );
            self.cached = Some((frame, area.width, area.height, scaled));
        }
        let Some((_, _, _, pixels)) = &self.cached else {
            return;
        };
        for row in 0..area.height {
            for column in 0..area.width {
                let mut mask = 0u8;
                let mut color = [0u32; 3];
                let mut weight = 0u32;
                for (dx, dy, bit) in DOTS {
                    let x = u32::from(column) * 2 + dx;
                    let y = u32::from(row) * 4 + dy;
                    let pixel = pixels.get_pixel(x, y).0;
                    let threshold =
                        (f32::from(DITHER[y as usize % 4][x as usize % 4]) + 0.5) / 16.0;
                    if f32::from(pixel[3]) / 255.0 * opacity <= threshold {
                        continue;
                    }
                    mask |= bit;
                    for channel in 0..3 {
                        color[channel] += u32::from(pixel[channel]) * u32::from(pixel[3]);
                    }
                    weight += u32::from(pixel[3]);
                }
                if mask != 0 {
                    let cell = &mut buf[(area.x + column, area.y + row)];
                    cell.set_char(char::from_u32(0x2800 + u32::from(mask)).unwrap_or(' '));
                    cell.set_fg(Color::Rgb(
                        (color[0] / weight) as u8,
                        (color[1] / weight) as u8,
                        (color[2] / weight) as u8,
                    ));
                }
            }
        }
    }
}
