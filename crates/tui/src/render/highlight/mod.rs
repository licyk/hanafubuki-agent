// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Syntax highlighting: [syntect] with the grammars and themes two-face
//! bundles (about 250 languages), for the fenced code of the Markdown
//! renderer and the lines of a diff.
//!
//! The active theme is one process-wide value (`themes` resolves it from
//! `tui.theme` at launch); `/theme` swaps it to preview one and puts the
//! saved one back on cancel. What was rendered before a swap keeps its
//! colours. Tests read a theme of their own thread, so parallel tests do not
//! change each other's rendering.
//!
//! Inputs over 512 KiB, 10,000 lines, or with a line over 4 KiB are not
//! highlighted: they come back as plain text.
//!
//! A theme's revision counts the swaps, so a stream's open code fence
//! (`streaming`) knows its retained parser state is of another theme.

mod streaming;
mod themes;

#[cfg(not(test))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use parking_lot::RwLock;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SyntectColor, FontStyle, Highlighter, Style as SyntectStyle, Theme,
};
use syntect::parsing::{Scope, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

pub(crate) use streaming::StreamingCodeHighlighter;
pub(crate) use themes::{
    configure_theme, list_available_themes, resolve_theme_by_name, themes_dir, DEFAULT_THEME,
};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
#[cfg(not(test))]
static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();
#[cfg(not(test))]
static THEME_REVISION: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static TEST_THEME: std::cell::OnceCell<std::sync::Arc<RwLock<Theme>>> =
        const { std::cell::OnceCell::new() };
    static TEST_THEME_REVISION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

// bat's `ansi`, `base16` and `base16-256` themes say in the alpha channel
// that a colour is the terminal's: `a = 0` makes `r` an index of its
// palette, `a = 1` its default colour.
const ANSI_ALPHA_INDEX: u8 = 0x00;
const ANSI_ALPHA_DEFAULT: u8 = 0x01;

const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;
pub(crate) const MAX_HIGHLIGHT_LINE_BYTES: usize = 4 * 1024;

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
}

fn theme_lock() -> impl std::ops::Deref<Target = RwLock<Theme>> {
    #[cfg(not(test))]
    {
        THEME.get_or_init(|| RwLock::new(themes::default_theme()))
    }
    #[cfg(test)]
    TEST_THEME.with(|theme| {
        theme
            .get_or_init(|| std::sync::Arc::new(RwLock::new(themes::default_theme())))
            .clone()
    })
}

/// Makes `theme` the one everything is highlighted with from now on.
pub(crate) fn set_syntax_theme(theme: Theme) {
    *theme_lock().write() = theme;
    #[cfg(not(test))]
    THEME_REVISION.fetch_add(1, Ordering::Release);
    #[cfg(test)]
    TEST_THEME_REVISION.with(|revision| revision.set(revision.get().wrapping_add(1)));
}

/// How many times the theme was swapped, for what keeps highlighting state.
pub(crate) fn syntax_theme_revision() -> u64 {
    #[cfg(not(test))]
    {
        THEME_REVISION.load(Ordering::Acquire)
    }
    #[cfg(test)]
    TEST_THEME_REVISION.with(std::cell::Cell::get)
}

/// The active theme, to put back after a preview.
pub(crate) fn current_syntax_theme() -> Theme {
    theme_lock().read().clone()
}

/// `code` in `lang` (a name, an alias or a file extension), one line per
/// source line; plain when the language is unknown or the input too large.
/// The theme's foregrounds only: the caller's background stays.
pub(crate) fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
    match highlight_to_line_spans(code, lang) {
        Some(lines) => lines.into_iter().map(Line::from).collect(),
        None => {
            // `lines()`, not `split('\n')`: the code of a fenced block ends
            // with a newline that is not a line of its own.
            let mut lines: Vec<Line<'static>> = code
                .lines()
                .map(|line| Line::from(line.to_string()))
                .collect();
            if lines.is_empty() {
                lines.push(Line::default());
            }
            lines
        }
    }
}

/// The spans of each line of `code`, for a diff to place after its sign;
/// `None` when it is not highlighted, and the diff colours it itself.
pub(crate) fn highlight_code_to_styled_spans(
    code: &str,
    lang: &str,
) -> Option<Vec<Vec<Span<'static>>>> {
    highlight_to_line_spans(code, lang)
}

/// Whether a diff of this size is highlighted at all.
pub(crate) fn exceeds_highlight_limits(total_bytes: usize, total_lines: usize) -> bool {
    total_bytes > MAX_HIGHLIGHT_BYTES || total_lines > MAX_HIGHLIGHT_LINES
}

/// The foreground the active theme gives the first of `scopes` it styles
/// (the status line's item colours).
pub(crate) fn foreground_for_scopes(scopes: &[&str]) -> Option<Color> {
    let theme = theme_lock();
    let theme = theme.read();
    foreground_for_scopes_with_theme(&theme, scopes)
}

/// [`foreground_for_scopes`] in `theme` (a Mermaid diagram's colours).
pub(crate) fn foreground_for_scopes_with_theme(theme: &Theme, scopes: &[&str]) -> Option<Color> {
    let highlighter = Highlighter::new(theme);
    scopes.iter().find_map(|scope| {
        let scope = Scope::new(scope).ok()?;
        convert_syntect_color(highlighter.style_mod_for_stack(&[scope]).foreground?)
    })
}

fn highlight_to_line_spans(code: &str, lang: &str) -> Option<Vec<Vec<Span<'static>>>> {
    let theme = theme_lock();
    let theme = theme.read();
    highlight_with_theme(code, lang, &theme)
}

fn highlight_with_theme(code: &str, lang: &str, theme: &Theme) -> Option<Vec<Vec<Span<'static>>>> {
    if code.is_empty()
        || code.len() > MAX_HIGHLIGHT_BYTES
        || code.lines().count() > MAX_HIGHLIGHT_LINES
        || code
            .lines()
            .any(|line| line.len() > MAX_HIGHLIGHT_LINE_BYTES)
    {
        return None;
    }
    let syntax = find_syntax(lang)?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    LinesWithEndings::from(code)
        .map(|line| {
            let ranges = highlighter.highlight_line(line, syntax_set()).ok()?;
            Some(line_spans(ranges))
        })
        .collect()
}

/// One highlighted line; its line ending is the `Line`'s, not a span's.
fn line_spans(ranges: Vec<(SyntectStyle, &str)>) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = ranges
        .into_iter()
        .filter_map(|(style, text)| {
            let text = text.trim_end_matches(['\n', '\r']);
            (!text.is_empty()).then(|| Span::styled(text.to_string(), convert_style(style)))
        })
        .collect();
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

/// The grammar for `lang`, by token (an extension or alias), name, or
/// extension; two-face resolves most names, a few aliases are added.
fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let set = syntax_set();
    let lower = lang.to_ascii_lowercase();
    let patched = match lower.as_str() {
        "csharp" | "c-sharp" => "c#",
        "cu" | "cuh" | "cppm" | "cxxm" | "ixx" => "cpp",
        "golang" => "go",
        "python3" => "python",
        "shell" => "bash",
        _ => lang,
    };
    set.find_syntax_by_token(patched)
        .or_else(|| set.find_syntax_by_name(patched))
        .or_else(|| {
            let lower = patched.to_ascii_lowercase();
            set.syntaxes()
                .iter()
                .find(|syntax| syntax.name.to_ascii_lowercase() == lower)
        })
        .or_else(|| set.find_syntax_by_extension(lang))
}

/// A theme's colour as the terminal's; `None` for its default colour.
fn convert_syntect_color(color: SyntectColor) -> Option<Color> {
    match color.a {
        ANSI_ALPHA_INDEX => Some(ansi_palette_color(color.r)),
        ANSI_ALPHA_DEFAULT => None,
        _ => Some(Color::Rgb(color.r, color.g, color.b)),
    }
}

/// The named colours for the low indices: terminals brighten them for bold
/// text the way the `ansi` themes expect.
fn ansi_palette_color(index: u8) -> Color {
    match index {
        0x00 => Color::Black,
        0x01 => Color::Red,
        0x02 => Color::Green,
        0x03 => Color::Yellow,
        0x04 => Color::Blue,
        0x05 => Color::Magenta,
        0x06 => Color::Cyan,
        0x07 => Color::Gray,
        index => Color::Indexed(index),
    }
}

/// The foreground and bold only: a background would cover the terminal's,
/// and italics and underlines (Dracula underlines type names) read poorly
/// in a terminal.
fn convert_style(style: SyntectStyle) -> Style {
    let mut converted = Style::default();
    if let Some(color) = convert_syntect_color(style.foreground) {
        converted = converted.fg(color);
    }
    if style.font_style.contains(FontStyle::BOLD) {
        converted = converted.add_modifier(Modifier::BOLD);
    }
    converted
}

#[cfg(test)]
mod tests;
