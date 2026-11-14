//! The colours and attributes a terminal's SGR sequences (`ESC [ … m`) give
//! text, and the styles of a rendered text: runs of its bytes, each with the
//! style it was written in.
//!
//! Only what reads the same in every terminal is kept: the foreground and
//! background (the 16 palette colours, the 256-colour palette, 24-bit
//! colour), bold, dim, italic, underline and reverse video. Blink, conceal,
//! strike-through, overline, underline colours and fonts are dropped, as is
//! every sequence other than SGR.

use serde::{Deserialize, Serialize};

/// A colour of [`TextStyle`]: a palette index (0–15 are the terminal's own
/// 16 colours, as SGR 30–37 and 90–97 name them) or 24-bit colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextColor {
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The SGR state text was written in. The default is the terminal's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<TextColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<TextColor>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dim: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// The bytes `start..end` of a text, written in `style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleSpan {
    pub start: usize,
    pub end: usize,
    pub style: TextStyle,
}

/// How a plain text of `text_bytes` bytes was styled: its runs that are not
/// in the default style, in order, each within one line.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyles {
    pub text_bytes: usize,
    pub spans: Vec<StyleSpan>,
}

/// The runs a [`TextStyles`] keeps; text past the last one shows in the
/// default style, so output that changes colour at every character cannot
/// make its styles many times larger than its text.
pub const MAX_STYLE_SPANS: usize = 2048;

impl TextStyles {
    /// Whether these are the styles of `text`: its length, and runs in
    /// order, apart, non-empty and cut at character boundaries.
    pub fn fits(&self, text: &str) -> bool {
        let mut end = 0;
        self.text_bytes == text.len()
            && self.spans.iter().all(|span| {
                let fits = span.start >= end
                    && span.start < span.end
                    && span.end <= text.len()
                    && text.is_char_boundary(span.start)
                    && text.is_char_boundary(span.end);
                end = span.end;
                fits
            })
    }

    /// Adds `start..end` in `style`, joined to the last run when it goes on
    /// from it in the same style. Default-styled text adds nothing.
    pub fn push(&mut self, start: usize, end: usize, style: TextStyle) {
        if start >= end || style == TextStyle::default() {
            return;
        }
        if let Some(last) = self.spans.last_mut() {
            if last.end == start && last.style == style {
                last.end = end;
                return;
            }
        }
        if self.spans.len() < MAX_STYLE_SPANS {
            self.spans.push(StyleSpan { start, end, style });
        }
    }

    /// Appends `other`, the styles of a text that follows this one's.
    pub fn append(&mut self, other: &TextStyles) {
        let offset = self.text_bytes;
        for span in &other.spans {
            self.push(offset + span.start, offset + span.end, span.style);
        }
        self.text_bytes += other.text_bytes;
    }

    /// Styles nothing of `text_bytes` more bytes of text.
    pub fn extend_plain(&mut self, text_bytes: usize) {
        self.text_bytes += text_bytes;
    }
}

/// Applies an SGR sequence's parameters (what is between `ESC [` and `m`)
/// to `style`, in order. Parameters are separated by `;`, and an extended
/// colour's parts by `;` or `:` (`38;5;208`, `38:2::1:2:3`). A parameter
/// this does not know is skipped, with the parts of a colour it names.
pub(super) fn apply_sgr(style: &mut TextStyle, parameters: &str) {
    let mut groups = parameters.split(';');
    while let Some(group) = groups.next() {
        let mut parts = group.split(':');
        let code = parts.next().unwrap_or("");
        let Ok(code) = (if code.is_empty() {
            Ok(0)
        } else {
            code.parse::<u16>()
        }) else {
            continue;
        };
        match code {
            0 => *style = TextStyle::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            // `4:0` is "no underline"; `4:1`–`4:5` are its kinds.
            4 => style.underline = parts.next().is_none_or(|kind| kind != "0"),
            7 => style.reverse = true,
            22 => {
                style.bold = false;
                style.dim = false;
            }
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.reverse = false,
            30..=37 => style.fg = Some(TextColor::Indexed((code - 30) as u8)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(TextColor::Indexed((code - 40) as u8)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(TextColor::Indexed((code - 90 + 8) as u8)),
            100..=107 => style.bg = Some(TextColor::Indexed((code - 100 + 8) as u8)),
            38 | 48 | 58 => {
                let color = if group.contains(':') {
                    colon_color(parts)
                } else {
                    semicolon_color(&mut groups)
                };
                match (code, color) {
                    (38, Some(color)) => style.fg = Some(color),
                    (48, Some(color)) => style.bg = Some(color),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// `5;n` or `2;r;g;b` after `38;`, taken from the parameters that follow.
fn semicolon_color<'a>(groups: &mut impl Iterator<Item = &'a str>) -> Option<TextColor> {
    match groups.next()? {
        "5" => Some(TextColor::Indexed(groups.next()?.parse().ok()?)),
        "2" => {
            let r = groups.next()?.parse().ok();
            let g = groups.next()?.parse().ok();
            let b = groups.next()?.parse().ok();
            Some(TextColor::Rgb(r?, g?, b?))
        }
        _ => None,
    }
}

/// `5:n`, `2:r:g:b` or `2:colour-space:r:g:b` after `38:`.
fn colon_color<'a>(parts: impl Iterator<Item = &'a str>) -> Option<TextColor> {
    let parts = parts.collect::<Vec<_>>();
    match parts.as_slice() {
        ["5", index] => Some(TextColor::Indexed(index.parse().ok()?)),
        ["2", r, g, b] | ["2", _, r, g, b] => Some(TextColor::Rgb(
            r.parse().ok()?,
            g.parse().ok()?,
            b.parse().ok()?,
        )),
        _ => None,
    }
}

/// The SGR sequence that sets `style` from any state: a reset, then each of
/// its attributes.
pub fn sgr_sequence(style: &TextStyle) -> String {
    let mut parameters = String::from("0");
    for (on, code) in [
        (style.bold, "1"),
        (style.dim, "2"),
        (style.italic, "3"),
        (style.underline, "4"),
        (style.reverse, "7"),
    ] {
        if on {
            parameters.push(';');
            parameters.push_str(code);
        }
    }
    for (color, base, bright, extended) in [(style.fg, 30, 90, 38), (style.bg, 40, 100, 48)] {
        match color {
            None => {}
            Some(TextColor::Indexed(index @ 0..=7)) => {
                parameters.push_str(&format!(";{}", base + u16::from(index)));
            }
            Some(TextColor::Indexed(index @ 8..=15)) => {
                parameters.push_str(&format!(";{}", bright + u16::from(index) - 8));
            }
            Some(TextColor::Indexed(index)) => {
                parameters.push_str(&format!(";{extended};5;{index}"));
            }
            Some(TextColor::Rgb(r, g, b)) => {
                parameters.push_str(&format!(";{extended};2;{r};{g};{b}"));
            }
        }
    }
    format!("\x1b[{parameters}m")
}

/// `text` as terminal text written in `styles`: each run between the SGR
/// sequence of its style and a reset, so a renderer of it reads `text` and
/// `styles` back.
pub fn styled_terminal_text(text: &str, styles: &TextStyles) -> String {
    let mut out = String::with_capacity(text.len());
    let mut written = 0;
    for span in &styles.spans {
        let (Some(before), Some(styled)) = (
            text.get(written..span.start),
            text.get(span.start..span.end),
        ) else {
            break;
        };
        out.push_str(before);
        out.push_str(&sgr_sequence(&span.style));
        out.push_str(styled);
        out.push_str("\x1b[0m");
        written = span.end;
    }
    out.push_str(text.get(written..).unwrap_or_default());
    out
}
