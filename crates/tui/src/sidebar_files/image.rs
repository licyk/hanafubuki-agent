// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use std::io::{self, Write};
use std::sync::Arc;

use base64::Engine;
use image::{
    imageops::{resize, FilterType},
    Rgba, RgbaImage,
};
use ratatui::{
    buffer::Buffer,
    crossterm::{
        cursor::{MoveTo, RestorePosition, SavePosition},
        queue,
        terminal::window_size,
    },
    layout::Rect,
    style::Color,
};

use crate::terminal_detection::{terminal_info, TerminalName};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
enum Protocol {
    Kitty,
    Sixel,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ImagePlacement {
    area: Rect,
    id: u64,
    encoded: Arc<[u8]>,
    protocol: Protocol,
}

impl ImagePlacement {
    pub(crate) fn same_image(&self, other: &Self) -> bool {
        self.id == other.id && self.area == other.area
    }

    // The terminal diff clears every row past its last styled cell. An invisible
    // style at the image's edge keeps that clear from erasing native pixels on
    // subsequent frames that do not retransmit them.
    pub(crate) fn reserve(&self, buffer: &mut Buffer) {
        let area = self.area.intersection(buffer.area);
        if !area.is_empty() {
            for row in area.y..area.bottom() {
                buffer[(area.right() - 1, row)]
                    .modifier
                    .insert(ratatui::style::Modifier::DIM);
            }
        }
    }

    pub(crate) fn paint(&self, writer: &mut impl Write) -> io::Result<()> {
        queue!(writer, SavePosition, MoveTo(self.area.x, self.area.y))?;
        writer.write_all(&self.encoded)?;
        queue!(writer, RestorePosition)
    }
    pub(crate) fn erase(&self, writer: &mut impl Write) -> io::Result<()> {
        match self.protocol {
            Protocol::Kitty => write!(writer, "\x1b_Ga=d,d=I,i={},q=2\x1b\\", self.id as u32),
            Protocol::Sixel => {
                queue!(writer, SavePosition)?;
                for y in self.area.y..self.area.bottom() {
                    queue!(writer, MoveTo(self.area.x, y))?;
                    write!(writer, "\x1b[{}X", self.area.width)?;
                }
                queue!(writer, RestorePosition)
            }
        }
    }
}

struct NativeFit {
    area: Rect,
    encoded: Arc<[u8]>,
    protocol: Protocol,
    id: u64,
}
pub(super) struct Preview {
    pixels: RgbaImage,
    fit: Option<(u16, u16, Fit)>,
}
enum Fit {
    Cells(RgbaImage),
    Native(NativeFit),
}

impl Preview {
    pub(super) fn new(pixels: RgbaImage) -> Self {
        Self { pixels, fit: None }
    }

    pub(super) fn paint(&mut self, area: Rect, buf: &mut Buffer) -> Option<ImagePlacement> {
        if area.is_empty() {
            return None;
        }
        if !self
            .fit
            .as_ref()
            .is_some_and(|(w, h, _)| (*w, *h) == (area.width, area.height))
        {
            let fit = self.native_fit(area).map_or_else(
                || {
                    let (w, h) = fit(
                        self.pixels.dimensions(),
                        (area.width.into(), u32::from(area.height) * 2),
                    );
                    Fit::Cells(resize(&self.pixels, w, h, FilterType::Triangle))
                },
                Fit::Native,
            );
            self.fit = Some((area.width, area.height, fit));
        }
        match &self.fit.as_ref()?.2 {
            Fit::Native(image) => {
                let mut rect = image.area;
                rect.x = area.x + (area.width - rect.width) / 2;
                rect.y = area.y + (area.height - rect.height) / 2;
                Some(ImagePlacement {
                    area: rect,
                    encoded: image.encoded.clone(),
                    protocol: image.protocol,
                    id: image.id,
                })
            }
            Fit::Cells(image) => {
                let color = |p: &Rgba<u8>| {
                    if p[3] < 128 {
                        Color::Reset
                    } else {
                        Color::Rgb(p[0], p[1], p[2])
                    }
                };
                let x0 = area.x + (area.width - image.width() as u16) / 2;
                let y0 = area.y + (area.height - image.height().div_ceil(2) as u16) / 2;
                for y in (0..image.height()).step_by(2) {
                    for x in 0..image.width() {
                        let top = color(image.get_pixel(x, y));
                        let bottom = image
                            .get_pixel_checked(x, y + 1)
                            .map_or(Color::Reset, color);
                        buf[(x0 + x as u16, y0 + (y / 2) as u16)]
                            .set_symbol("▀")
                            .set_fg(top)
                            .set_bg(bottom);
                    }
                }
                None
            }
        }
    }

    fn native_fit(&self, area: Rect) -> Option<NativeFit> {
        let terminal = terminal_info();
        if terminal.multiplexer.is_some() {
            return None;
        }
        let protocol = match terminal.name {
            TerminalName::Kitty
            | TerminalName::Ghostty
            | TerminalName::Rex
            | TerminalName::WezTerm => Protocol::Kitty,
            TerminalName::WindowsTerminal => Protocol::Sixel,
            _ if terminal.term.as_deref().is_some_and(|term| {
                ["sixel", "foot", "mlterm"]
                    .iter()
                    .any(|name| term.contains(name))
            }) =>
            {
                Protocol::Sixel
            }
            _ => return None,
        };
        let (cw, ch) = window_size()
            .ok()
            .and_then(|s| {
                Some((
                    s.width.checked_div(s.columns)?,
                    s.height.checked_div(s.rows)?,
                ))
            })
            .filter(|(w, h)| *w > 0 && *h > 0)
            .or_else(|| (protocol == Protocol::Kitty).then_some((8, 16)))?;
        let max_w = u32::from(area.width) * u32::from(cw);
        let max_h = u32::from(area.height) * u32::from(ch);
        let (max_w, max_h) = match protocol {
            // Same compact RGB332 transfer budget as the upstream Sixel preview.
            Protocol::Sixel => (max_w.min(160), max_h.min(80)),
            Protocol::Kitty => (max_w, max_h),
        };
        let (w, h) = fit(
            self.pixels.dimensions(),
            (
                max_w.min(self.pixels.width()),
                max_h.min(self.pixels.height()),
            ),
        );
        let image = resize(&self.pixels, w, h, FilterType::Triangle);
        let rect = Rect::new(
            0,
            0,
            w.div_ceil(u32::from(cw)) as u16,
            h.div_ceil(u32::from(ch)) as u16,
        );
        let id = super::next_request();
        let encoded = match protocol {
            Protocol::Kitty => kitty(&image, rect, id as u32),
            Protocol::Sixel => sixel(&image),
        };
        Some(NativeFit {
            area: rect,
            encoded: encoded.into(),
            protocol,
            id,
        })
    }
}

fn fit((w, h): (u32, u32), (mw, mh): (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(mw) / f64::from(w)).min(f64::from(mh) / f64::from(h));
    (
        (f64::from(w) * scale).round().max(1.) as u32,
        (f64::from(h) * scale).round().max(1.) as u32,
    )
}

fn kitty(image: &RgbaImage, area: Rect, id: u32) -> Vec<u8> {
    let data = base64::engine::general_purpose::STANDARD.encode(image.as_raw());
    let mut out = Vec::new();
    let mut chunks = data.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        out.extend_from_slice(b"\x1b_G");
        if first {
            write!(
                out,
                "a=T,f=32,s={},v={},c={},r={},i={id},C=1,q=2,",
                image.width(),
                image.height(),
                area.width,
                area.height
            )
            .unwrap();
            first = false;
        }
        out.extend_from_slice(if chunks.peek().is_some() {
            b"m=1;"
        } else {
            b"m=0;"
        });
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

fn palette(pixel: &Rgba<u8>) -> Option<u8> {
    (pixel[3] >= 128).then_some((pixel[0] & 0xe0) | ((pixel[1] >> 3) & 0x1c) | (pixel[2] >> 6))
}

fn sixel(image: &RgbaImage) -> Vec<u8> {
    use std::collections::BTreeMap;
    let (w, h) = image.dimensions();
    let mut out = format!("\x1bP9;1;0q\"1;1;{w};{h}").into_bytes();
    let mut defined = [false; 256];
    for top in (0..h).step_by(6) {
        let mut planes = BTreeMap::<u8, Vec<u8>>::new();
        for y in top..(top + 6).min(h) {
            for x in 0..w {
                if let Some(color) = palette(image.get_pixel(x, y)) {
                    planes.entry(color).or_insert_with(|| vec![0; w as usize])[x as usize] |=
                        1 << (y - top);
                }
            }
        }
        for (color, plane) in planes {
            write!(out, "#{color}").unwrap();
            if !defined[color as usize] {
                let rgb = u16::from(color);
                write!(
                    out,
                    ";2;{};{};{}",
                    (rgb >> 5) * 100 / 7,
                    ((rgb >> 2) & 7) * 100 / 7,
                    (rgb & 3) * 100 / 3
                )
                .unwrap();
                defined[color as usize] = true;
            }
            out.extend(plane.into_iter().map(|v| v + b'?'));
            out.push(b'$');
        }
        if top + 6 < h {
            out.push(b'-');
        }
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

#[cfg(test)]
mod tests;
