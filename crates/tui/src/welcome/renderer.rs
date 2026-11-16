use std::sync::OnceLock;
use std::time::Duration;

use hanafubuki_agent::AgentWelcomeTheme;
use image::{imageops, RgbaImage};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use serde::Deserialize;

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
    #[serde(default)]
    layout: Layout,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(default)]
struct Layout {
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

struct Atlas {
    image: RgbaImage,
    meta: Metadata,
}

fn atlas(theme: AgentWelcomeTheme) -> Option<&'static Atlas> {
    type Entry = (OnceLock<Option<Atlas>>, &'static [u8], &'static [u8]);
    static ATLASES: [Entry; 5] = [
        (
            OnceLock::new(),
            include_bytes!("../../assets/welcome/atlas.png"),
            include_bytes!("../../assets/welcome/atlas.json"),
        ),
        (
            OnceLock::new(),
            include_bytes!("../../assets/welcome/left/atlas.png"),
            include_bytes!("../../assets/welcome/left/atlas.json"),
        ),
        (
            OnceLock::new(),
            include_bytes!("../../assets/welcome/sleep/atlas.png"),
            include_bytes!("../../assets/welcome/sleep/atlas.json"),
        ),
        (
            OnceLock::new(),
            include_bytes!("../../assets/welcome/read/atlas.png"),
            include_bytes!("../../assets/welcome/read/atlas.json"),
        ),
        (
            OnceLock::new(),
            include_bytes!("../../assets/welcome/corner/atlas.png"),
            include_bytes!("../../assets/welcome/corner/atlas.json"),
        ),
    ];
    let index = match theme {
        AgentWelcomeTheme::Random | AgentWelcomeTheme::Center => 0,
        AgentWelcomeTheme::LeftPeek => 1,
        AgentWelcomeTheme::RightSleep => 2,
        AgentWelcomeTheme::RightRead => 3,
        AgentWelcomeTheme::LowerLeft => 4,
    };
    let (cache, png, json) = &ATLASES[index];
    cache
        .get_or_init(|| {
            Some(Atlas {
                image: image::load_from_memory(png).ok()?.into_rgba8(),
                meta: serde_json::from_slice(json).ok()?,
            })
        })
        .as_ref()
}

fn stage(available: Rect, buf: &Buffer, meta: &Metadata) -> Option<Rect> {
    let layout = meta.layout;
    if available.width < 64 || available.height < layout.min_rows {
        return None;
    }
    let mut top = available.y;
    for y in available.y..available.bottom() {
        if (available.x..available.right()).any(|x| !buf[(x, y)].symbol().trim().is_empty()) {
            top = y + 1;
        }
    }
    let aspect = meta.width as f32 * 2.0 * layout.width_scale / meta.height as f32;
    let rows_by_width = (f32::from(available.width) / aspect).floor() as u16;
    let rows = available
        .bottom()
        .saturating_sub(top)
        .min(layout.max_rows)
        .min(rows_by_width);
    if rows < layout.min_rows {
        return None;
    }
    let columns = (f32::from(rows) * aspect).floor() as u16;
    Some(Rect::new(
        available.x + (f32::from(available.width - columns) * layout.horizontal) as u16,
        top + (f32::from(available.bottom() - top - rows) * layout.vertical) as u16,
        columns,
        rows,
    ))
}

pub(super) struct Renderer {
    theme: AgentWelcomeTheme,
    cached: Option<(u32, u16, u16, RgbaImage)>,
}

impl Renderer {
    pub(super) fn new(theme: AgentWelcomeTheme) -> Self {
        Self {
            theme,
            cached: None,
        }
    }

    pub(super) fn stage(&self, available: Rect, buf: &Buffer) -> Option<Rect> {
        stage(available, buf, &atlas(self.theme)?.meta)
    }

    pub(super) fn paint(&mut self, area: Rect, buf: &mut Buffer, elapsed: Duration, opacity: f32) {
        let Some(atlas) = atlas(self.theme) else {
            return;
        };
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
