// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A diff row's colours, Codex's palette: each row is a gutter (its line
//! number), a sign and its content, over a background tint that fills the
//! whole row.
//!
//! - **Theme.** Dark terminals get muted tints, light ones GitHub's pastels
//!   with a stronger gutter so the numbers stay readable. Codex picks by the
//!   terminal's queried background; the interface does not query it
//!   (`style.rs`), so the dark palette is used, as Codex's is when its query
//!   gets no answer.
//! - **Colour depth.** Fixed palettes for 24-bit and 256 colours; with 16
//!   colours there are no backgrounds at all (the 16 backgrounds are too
//!   saturated under syntax colours): the sign and plain content are green
//!   or red.
//! - **Syntax theme.** A theme that gives `markup.inserted` /
//!   `markup.deleted` (or `diff.*`) a background replaces the palette's
//!   tint, at a rich colour depth.

use ratatui::style::{Color, Modifier, Style};

use crate::color::is_light;
use crate::render::highlight::{diff_scope_backgrounds, DiffScopeBackground, DiffScopeBackgrounds};
use crate::terminal_detection::{terminal_info, TerminalName};
use crate::terminal_palette::{
    indexed_color, nearest_xterm_color, rgb_color, stdout_color_level, StdoutColorLevel,
};

// Truecolor palette.
const DARK_TC_ADD_LINE_BG_RGB: (u8, u8, u8) = (33, 58, 43); // #213A2B
const DARK_TC_DEL_LINE_BG_RGB: (u8, u8, u8) = (74, 34, 29); // #4A221D
const LIGHT_TC_ADD_LINE_BG_RGB: (u8, u8, u8) = (218, 251, 225); // #dafbe1
const LIGHT_TC_DEL_LINE_BG_RGB: (u8, u8, u8) = (255, 235, 233); // #ffebe9
const LIGHT_TC_ADD_NUM_BG_RGB: (u8, u8, u8) = (172, 238, 187); // #aceebb
const LIGHT_TC_DEL_NUM_BG_RGB: (u8, u8, u8) = (255, 206, 203); // #ffcecb
const LIGHT_TC_GUTTER_FG_RGB: (u8, u8, u8) = (31, 35, 40); // #1f2328

// 256-color palette.
const DARK_256_ADD_LINE_BG_IDX: u8 = 22;
const DARK_256_DEL_LINE_BG_IDX: u8 = 52;
const LIGHT_256_ADD_LINE_BG_IDX: u8 = 194;
const LIGHT_256_DEL_LINE_BG_IDX: u8 = 224;
const LIGHT_256_ADD_NUM_BG_IDX: u8 = 157;
const LIGHT_256_DEL_NUM_BG_IDX: u8 = 217;
const LIGHT_256_GUTTER_FG_IDX: u8 = 236;

/// What a diff row does: `Insert` has a `+` and green, `Delete` a `-` and
/// red, `Context` a space and the terminal's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLineType {
    Insert,
    Delete,
    Context,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffTheme {
    Dark,
    Light,
}

/// The renderer's colour depth: the terminal's, with Windows Terminal
/// promoted to 24-bit (`terminal_palette`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffColorLevel {
    TrueColor,
    Ansi256,
    Ansi16,
}

/// A depth with tinted backgrounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RichDiffColorLevel {
    TrueColor,
    Ansi256,
}

impl RichDiffColorLevel {
    fn from_diff_color_level(level: DiffColorLevel) -> Option<Self> {
        match level {
            DiffColorLevel::TrueColor => Some(Self::TrueColor),
            DiffColorLevel::Ansi256 => Some(Self::Ansi256),
            DiffColorLevel::Ansi16 => None,
        }
    }
}

/// The added and removed rows' backgrounds; none with 16 colours.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResolvedDiffBackgrounds {
    pub(crate) add: Option<Color>,
    pub(crate) del: Option<Color>,
}

/// The theme, depth and backgrounds of one rendering, read once for all of
/// its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiffRenderStyleContext {
    pub(crate) theme: DiffTheme,
    pub(crate) color_level: DiffColorLevel,
    pub(crate) diff_backgrounds: ResolvedDiffBackgrounds,
}

impl DiffRenderStyleContext {
    /// The context of `theme` at `color_level` with `scope_backgrounds`
    /// from a syntax theme.
    pub(crate) fn new(
        theme: DiffTheme,
        color_level: DiffColorLevel,
        scope_backgrounds: DiffScopeBackgrounds,
    ) -> Self {
        Self {
            theme,
            color_level,
            diff_backgrounds: resolve_diff_backgrounds_for(theme, color_level, scope_backgrounds),
        }
    }

    /// The depth the contrast of syntax colours is resolved at.
    pub(crate) fn stdout_level(self) -> StdoutColorLevel {
        match self.color_level {
            DiffColorLevel::TrueColor => StdoutColorLevel::TrueColor,
            DiffColorLevel::Ansi256 => StdoutColorLevel::Ansi256,
            DiffColorLevel::Ansi16 => StdoutColorLevel::Ansi16,
        }
    }
}

/// This terminal's context, with the active syntax theme's backgrounds.
/// A test's is its thread's ([`with_diff_style_context`]), 16 colours on a
/// dark terminal otherwise, whatever the environment it runs in.
pub(crate) fn current_diff_render_style_context() -> DiffRenderStyleContext {
    #[cfg(test)]
    if let Some(context) = TEST_CONTEXT.with(std::cell::Cell::get) {
        return context;
    }
    DiffRenderStyleContext::new(diff_theme(), diff_color_level(), diff_scope_backgrounds())
}

#[cfg(test)]
thread_local! {
    static TEST_CONTEXT: std::cell::Cell<Option<DiffRenderStyleContext>> =
        const { std::cell::Cell::new(None) };
}

/// Runs `render` with `context` as this thread's diff colours.
#[cfg(test)]
pub(crate) fn with_diff_style_context<T>(
    context: DiffRenderStyleContext,
    render: impl FnOnce() -> T,
) -> T {
    let previous = TEST_CONTEXT.with(|current| current.replace(Some(context)));
    let result = render();
    TEST_CONTEXT.with(|current| current.set(previous));
    result
}

/// The palette's backgrounds, replaced by the syntax theme's where it has
/// them at a rich depth; a theme's terminal-default marker leaves a row
/// unfilled.
fn resolve_diff_backgrounds_for(
    theme: DiffTheme,
    color_level: DiffColorLevel,
    scope_backgrounds: DiffScopeBackgrounds,
) -> ResolvedDiffBackgrounds {
    let mut resolved = fallback_diff_backgrounds(theme, color_level);
    let Some(level) = RichDiffColorLevel::from_diff_color_level(color_level) else {
        return resolved;
    };
    for (target, background) in [
        (&mut resolved.add, scope_backgrounds.inserted),
        (&mut resolved.del, scope_backgrounds.deleted),
    ] {
        match background {
            Some(DiffScopeBackground::Rgb(rgb)) => {
                *target = Some(color_from_rgb_for_level(rgb, level))
            }
            Some(DiffScopeBackground::TerminalDefault) => *target = None,
            None => {}
        }
    }
    resolved
}

fn fallback_diff_backgrounds(
    theme: DiffTheme,
    color_level: DiffColorLevel,
) -> ResolvedDiffBackgrounds {
    match RichDiffColorLevel::from_diff_color_level(color_level) {
        Some(level) => ResolvedDiffBackgrounds {
            add: Some(add_line_bg(theme, level)),
            del: Some(del_line_bg(theme, level)),
        },
        None => ResolvedDiffBackgrounds::default(),
    }
}

fn color_from_rgb_for_level(rgb: (u8, u8, u8), color_level: RichDiffColorLevel) -> Color {
    match color_level {
        RichDiffColorLevel::TrueColor => rgb_color(rgb),
        RichDiffColorLevel::Ansi256 => nearest_xterm_color(rgb),
    }
}

/// The theme a terminal background `bg` asks for; dark when it is not
/// known.
pub(crate) fn diff_theme_for_bg(bg: Option<(u8, u8, u8)>) -> DiffTheme {
    match bg {
        Some(rgb) if is_light(rgb) => DiffTheme::Light,
        _ => DiffTheme::Dark,
    }
}

/// The terminal's background is not queried (`style.rs`).
fn diff_theme() -> DiffTheme {
    diff_theme_for_bg(None)
}

/// The terminal's depth; 16 colours in tests, whatever the environment
/// they run in (a test sets its own, [`with_diff_style_context`]).
fn diff_color_level() -> DiffColorLevel {
    if cfg!(test) {
        return DiffColorLevel::Ansi16;
    }
    diff_color_level_for_terminal(
        stdout_color_level(),
        terminal_info().name,
        std::env::var_os("WT_SESSION").is_some(),
        std::env::var_os("FORCE_COLOR").is_some(),
    )
}

/// Codex's depth policy: `WT_SESSION` is 24-bit unless `FORCE_COLOR` is
/// set; outside it an identified Windows Terminal's 16 colours are
/// promoted; an unknown depth is 16 colours.
pub(crate) fn diff_color_level_for_terminal(
    stdout_level: StdoutColorLevel,
    terminal_name: TerminalName,
    has_wt_session: bool,
    has_force_color_override: bool,
) -> DiffColorLevel {
    if has_wt_session && !has_force_color_override {
        return DiffColorLevel::TrueColor;
    }
    let base = match stdout_level {
        StdoutColorLevel::TrueColor => DiffColorLevel::TrueColor,
        StdoutColorLevel::Ansi256 => DiffColorLevel::Ansi256,
        StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown => DiffColorLevel::Ansi16,
    };
    if stdout_level == StdoutColorLevel::Ansi16
        && terminal_name == TerminalName::WindowsTerminal
        && !has_force_color_override
    {
        DiffColorLevel::TrueColor
    } else {
        base
    }
}

/// The background of the whole row (the line's style, so it reaches the
/// right edge); a kept line has none.
pub(crate) fn style_line_bg_for(
    kind: DiffLineType,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Style {
    let bg = match kind {
        DiffLineType::Insert => diff_backgrounds.add,
        DiffLineType::Delete => diff_backgrounds.del,
        DiffLineType::Context => None,
    };
    bg.map_or_else(Style::default, |bg| Style::default().bg(bg))
}

pub(crate) fn style_context() -> Style {
    Style::default()
}

fn add_line_bg(theme: DiffTheme, color_level: RichDiffColorLevel) -> Color {
    match (theme, color_level) {
        (DiffTheme::Dark, RichDiffColorLevel::TrueColor) => rgb_color(DARK_TC_ADD_LINE_BG_RGB),
        (DiffTheme::Dark, RichDiffColorLevel::Ansi256) => indexed_color(DARK_256_ADD_LINE_BG_IDX),
        (DiffTheme::Light, RichDiffColorLevel::TrueColor) => rgb_color(LIGHT_TC_ADD_LINE_BG_RGB),
        (DiffTheme::Light, RichDiffColorLevel::Ansi256) => indexed_color(LIGHT_256_ADD_LINE_BG_IDX),
    }
}

fn del_line_bg(theme: DiffTheme, color_level: RichDiffColorLevel) -> Color {
    match (theme, color_level) {
        (DiffTheme::Dark, RichDiffColorLevel::TrueColor) => rgb_color(DARK_TC_DEL_LINE_BG_RGB),
        (DiffTheme::Dark, RichDiffColorLevel::Ansi256) => indexed_color(DARK_256_DEL_LINE_BG_IDX),
        (DiffTheme::Light, RichDiffColorLevel::TrueColor) => rgb_color(LIGHT_TC_DEL_LINE_BG_RGB),
        (DiffTheme::Light, RichDiffColorLevel::Ansi256) => indexed_color(LIGHT_256_DEL_LINE_BG_IDX),
    }
}

fn light_gutter_fg(color_level: DiffColorLevel) -> Color {
    match color_level {
        DiffColorLevel::TrueColor => rgb_color(LIGHT_TC_GUTTER_FG_RGB),
        DiffColorLevel::Ansi256 => indexed_color(LIGHT_256_GUTTER_FG_IDX),
        DiffColorLevel::Ansi16 => Color::Black,
    }
}

fn light_add_num_bg(color_level: RichDiffColorLevel) -> Color {
    match color_level {
        RichDiffColorLevel::TrueColor => rgb_color(LIGHT_TC_ADD_NUM_BG_RGB),
        RichDiffColorLevel::Ansi256 => indexed_color(LIGHT_256_ADD_NUM_BG_IDX),
    }
}

fn light_del_num_bg(color_level: RichDiffColorLevel) -> Color {
    match color_level {
        RichDiffColorLevel::TrueColor => rgb_color(LIGHT_TC_DEL_NUM_BG_RGB),
        RichDiffColorLevel::Ansi256 => indexed_color(LIGHT_256_DEL_NUM_BG_IDX),
    }
}

/// The line number's style: dim on a dark terminal; on a light one an
/// opaque, stronger tint so the number reads against the pastel row.
pub(crate) fn style_gutter_for(
    kind: DiffLineType,
    theme: DiffTheme,
    color_level: DiffColorLevel,
) -> Style {
    match (
        theme,
        kind,
        RichDiffColorLevel::from_diff_color_level(color_level),
    ) {
        (DiffTheme::Light, DiffLineType::Insert | DiffLineType::Delete, None) => {
            Style::default().fg(light_gutter_fg(color_level))
        }
        (DiffTheme::Light, DiffLineType::Insert, Some(level)) => Style::default()
            .fg(light_gutter_fg(color_level))
            .bg(light_add_num_bg(level)),
        (DiffTheme::Light, DiffLineType::Delete, Some(level)) => Style::default()
            .fg(light_gutter_fg(color_level))
            .bg(light_del_num_bg(level)),
        _ => style_gutter_dim(),
    }
}

/// The `+`: the content's style on a dark terminal, green alone over the
/// row's tint on a light one.
pub(crate) fn style_sign_add(context: DiffRenderStyleContext) -> Style {
    match context.theme {
        DiffTheme::Light => Style::default().fg(Color::Green),
        DiffTheme::Dark => style_add(context),
    }
}

/// The `-`, as [`style_sign_add`].
pub(crate) fn style_sign_del(context: DiffRenderStyleContext) -> Style {
    match context.theme {
        DiffTheme::Light => Style::default().fg(Color::Red),
        DiffTheme::Dark => style_del(context),
    }
}

/// An added row's plain content: green on 16 colours; over a tint, green
/// on a dark terminal and the default foreground on a light one.
pub(crate) fn style_add(context: DiffRenderStyleContext) -> Style {
    content_style(context, context.diff_backgrounds.add, Color::Green)
}

/// A removed row's plain content, as [`style_add`] in red.
pub(crate) fn style_del(context: DiffRenderStyleContext) -> Style {
    content_style(context, context.diff_backgrounds.del, Color::Red)
}

fn content_style(context: DiffRenderStyleContext, bg: Option<Color>, fg: Color) -> Style {
    match (context.theme, context.color_level, bg) {
        (_, DiffColorLevel::Ansi16, _) => Style::default().fg(fg),
        (DiffTheme::Light, _, Some(bg)) => Style::default().bg(bg),
        (DiffTheme::Dark, _, Some(bg)) => Style::default().fg(fg).bg(bg),
        (DiffTheme::Light, _, None) => Style::default(),
        (DiffTheme::Dark, _, None) => Style::default().fg(fg),
    }
}

pub(crate) fn style_gutter_dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
