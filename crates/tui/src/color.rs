// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Colour arithmetic (Codex's `color.rs`) and readable text on a painted
//! surface (Codex's `style/contrast.rs`): a theme's colour is kept when it
//! contrasts enough with the background it is drawn on, else moved toward
//! black or white, as little as the surface allows. The terminal's own
//! colours (the 16 named ones, its default) are never changed.

use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::style::Color;

use crate::terminal_palette::{
    best_color_for_level, indexed_color, rgb_color, xterm_color, StdoutColorLevel,
};

/// WCAG's minimum contrast for text.
const MIN_TEXT_CONTRAST: f64 = 4.5;
const MAX_CACHED_FOREGROUNDS: usize = 512;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct ForegroundKey {
    preferred: (u8, u8, u8),
    background: Option<(u8, u8, u8)>,
    level: StdoutColorLevel,
}

thread_local! {
    // A diff's spans reuse a few colours; the bound holds for themes with
    // many.
    static FOREGROUNDS: RefCell<HashMap<ForegroundKey, Color>> = RefCell::default();
}

pub(crate) fn is_light(bg: (u8, u8, u8)) -> bool {
    let (r, g, b) = bg;
    let y = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    y > 128.0
}

pub(crate) fn blend(fg: (u8, u8, u8), bg: (u8, u8, u8), alpha: f32) -> (u8, u8, u8) {
    let r = (fg.0 as f32 * alpha + bg.0 as f32 * (1.0 - alpha)) as u8;
    let g = (fg.1 as f32 * alpha + bg.1 as f32 * (1.0 - alpha)) as u8;
    let b = (fg.2 as f32 * alpha + bg.2 as f32 * (1.0 - alpha)) as u8;
    (r, g, b)
}

/// The perceptual distance of two colours: CIE76, the Euclidean distance
/// in Lab space.
pub(crate) fn perceptual_distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    fn srgb_to_linear(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn rgb_to_xyz(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
        let r = srgb_to_linear(r);
        let g = srgb_to_linear(g);
        let b = srgb_to_linear(b);
        let x = r * 0.4124 + g * 0.3576 + b * 0.1805;
        let y = r * 0.2126 + g * 0.7152 + b * 0.0722;
        let z = r * 0.0193 + g * 0.1192 + b * 0.9505;
        (x, y, z)
    }

    fn xyz_to_lab(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        // D65 reference white.
        let xr = x / 0.95047;
        let yr = y / 1.00000;
        let zr = z / 1.08883;
        fn f(t: f32) -> f32 {
            if t > 0.008856 {
                t.powf(1.0 / 3.0)
            } else {
                7.787 * t + 16.0 / 116.0
            }
        }
        let fx = f(xr);
        let fy = f(yr);
        let fz = f(zr);
        (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
    }

    let (x1, y1, z1) = rgb_to_xyz(a.0, a.1, a.2);
    let (x2, y2, z2) = rgb_to_xyz(b.0, b.1, b.2);
    let (l1, a1, b1) = xyz_to_lab(x1, y1, z1);
    let (l2, a2, b2) = xyz_to_lab(x2, y2, z2);
    let (dl, da, db) = (l1 - l2, a1 - a2, b1 - b2);
    (dl * dl + da * da + db * db).sqrt()
}

/// `preferred` readable on `background` in a terminal of `level`. The
/// terminal's own colours stay; without a known background (the default
/// one is not queried) a theme colour is only reduced to the level.
pub(crate) fn readable_color_on(
    preferred: Color,
    background: Option<Color>,
    level: StdoutColorLevel,
) -> Color {
    let preferred = match preferred {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(index) if index >= 16 => xterm_color(index),
        _ => return preferred,
    };
    let background = match background {
        Some(Color::Rgb(r, g, b)) => Some((r, g, b)),
        Some(Color::Indexed(index)) if index >= 16 => Some(xterm_color(index)),
        _ => None,
    };
    foreground(preferred, background, level)
}

fn foreground(
    preferred: (u8, u8, u8),
    background: Option<(u8, u8, u8)>,
    level: StdoutColorLevel,
) -> Color {
    FOREGROUNDS.with(|cache| {
        let key = ForegroundKey {
            preferred,
            background,
            level,
        };
        let mut cache = cache.borrow_mut();
        if let Some(color) = cache.get(&key) {
            return *color;
        }
        let color = match (background, level) {
            (None, _) => best_color_for_level(preferred, level),
            (Some(background), StdoutColorLevel::TrueColor) => {
                if ratio(preferred, background) >= MIN_TEXT_CONTRAST {
                    rgb_color(preferred)
                } else {
                    let black = (0, 0, 0);
                    let white = (255, 255, 255);
                    let endpoint = if ratio(black, background) >= ratio(white, background) {
                        black
                    } else {
                        white
                    };
                    // Keeps as much of the hue as the surface allows.
                    (1..=255)
                        .map(|step| blend(endpoint, preferred, step as f32 / 255.0))
                        .find(|candidate| ratio(*candidate, background) >= MIN_TEXT_CONTRAST)
                        .map_or_else(|| rgb_color(endpoint), rgb_color)
                }
            }
            (Some(background), StdoutColorLevel::Ansi256) => (16..=255u8)
                .filter(|index| ratio(xterm_color(*index), background) >= MIN_TEXT_CONTRAST)
                .min_by(|a, b| {
                    perceptual_distance(xterm_color(*a), preferred)
                        .total_cmp(&perceptual_distance(xterm_color(*b), preferred))
                })
                .map_or(Color::Reset, indexed_color),
            (Some(_), StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown) => Color::Reset,
        };
        if cache.len() >= MAX_CACHED_FOREGROUNDS {
            cache.clear();
        }
        cache.insert(key, color);
        color
    })
}

/// The WCAG contrast ratio of two colours.
fn ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn luminance(rgb: (u8, u8, u8)) -> f64 {
    let [r, g, b] = [rgb.0, rgb.1, rgb.2].map(|channel| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[cfg(test)]
mod tests;
