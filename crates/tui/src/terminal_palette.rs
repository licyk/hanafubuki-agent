// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The colours the terminal can show: its colour depth, read from the
//! environment (`supports-color`; the diff promotes Windows Terminal,
//! `diff_render::style`), and the xterm palette's fixed colours a 24-bit colour is reduced to.
//! Codex also queries the terminal's default colours (OSC 10/11); the
//! interface does not (`style.rs`), so nothing here knows them.

use ratatui::style::Color;

use crate::color::perceptual_distance;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StdoutColorLevel {
    TrueColor,
    Ansi256,
    Ansi16,
    Unknown,
}

/// What `supports-color` says of stdout.
pub(crate) fn stdout_color_level() -> StdoutColorLevel {
    match supports_color::on_cached(supports_color::Stream::Stdout) {
        Some(level) if level.has_16m => StdoutColorLevel::TrueColor,
        Some(level) if level.has_256 => StdoutColorLevel::Ansi256,
        Some(_) => StdoutColorLevel::Ansi16,
        None => StdoutColorLevel::Unknown,
    }
}

pub(crate) fn rgb_color((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

pub(crate) fn indexed_color(index: u8) -> Color {
    Color::Indexed(index)
}

/// The closest colour to `target` a terminal of `color_level` shows: itself
/// in 24-bit, the nearest fixed xterm colour in 256, the default colour
/// otherwise.
pub(crate) fn best_color_for_level(target: (u8, u8, u8), color_level: StdoutColorLevel) -> Color {
    match color_level {
        StdoutColorLevel::TrueColor => rgb_color(target),
        StdoutColorLevel::Ansi256 => nearest_xterm_color(target),
        StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown => Color::default(),
    }
}

/// The fixed xterm colour (16–255) perceptually closest to `target`. The
/// first 16 are the terminal's theme and not known.
pub(crate) fn nearest_xterm_color(target: (u8, u8, u8)) -> Color {
    let index = (16..=255u8)
        .min_by(|a, b| {
            perceptual_distance(xterm_color(*a), target)
                .total_cmp(&perceptual_distance(xterm_color(*b), target))
        })
        .unwrap_or(16);
    indexed_color(index)
}

/// The colour of a fixed xterm index (16–255): the 6×6×6 cube, then the
/// grey ramp. Lower indices are the theme's; they read as black.
pub(crate) fn xterm_color(index: u8) -> (u8, u8, u8) {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => (0, 0, 0),
        16..=231 => {
            let cube = usize::from(index - 16);
            (LEVELS[cube / 36], LEVELS[(cube / 6) % 6], LEVELS[cube % 6])
        }
        232..=255 => {
            let grey = 8 + 10 * (index - 232);
            (grey, grey, grey)
        }
    }
}

#[cfg(test)]
mod tests;
