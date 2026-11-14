// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's `diff_render` tests, ported where the Agent's patch reading has
//! what Codex's unified diff has: a file Codex builds with `diffy` from an
//! original and a modified text is written here as the patch the applier
//! reads, with the chunk starts it reports. The palette's and the
//! wrapper's tests are Codex's own. `tool/tests/diffs.rs` renders the same
//! patches in their cells, against Codex's snapshots.

use hanafubuki_agent::AgentPatchPreview;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Line;

use super::style::*;
use super::wrap::wrap_styled_spans;
use super::*;
use crate::render::highlight::{
    resolve_theme_by_name, set_syntax_theme, DiffScopeBackground, DiffScopeBackgrounds,
};
use crate::terminal_detection::TerminalName;
use crate::terminal_palette::{indexed_color, rgb_color, StdoutColorLevel};
use crate::wrapping::line_width;

use crate::test_support::{
    diff_gallery as gallery, patch_file as file, patch_preview as patch, update_block,
};

fn text(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn context(theme: DiffTheme, color_level: DiffColorLevel) -> DiffRenderStyleContext {
    DiffRenderStyleContext::new(theme, color_level, DiffScopeBackgrounds::default())
}

fn dark_truecolor() -> DiffRenderStyleContext {
    context(DiffTheme::Dark, DiffColorLevel::TrueColor)
}

fn fallback(theme: DiffTheme, color_level: DiffColorLevel) -> ResolvedDiffBackgrounds {
    context(theme, color_level).diff_backgrounds
}

/// Each span as `text` with its foreground and its dimming.
fn styled(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .filter(|span| !span.content.is_empty())
                .map(|span| {
                    let dim = if span.style.add_modifier.contains(Modifier::DIM) {
                        "~"
                    } else {
                        ""
                    };
                    match span.style.fg {
                        Some(color) => format!("[{}]{{{color}{dim}}}", span.content),
                        None => format!("[{}]{dim}", span.content),
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// The palette (Codex's tests).

#[test]
fn ansi16_add_style_uses_foreground_only() {
    let style = style_add(context(DiffTheme::Dark, DiffColorLevel::Ansi16));
    assert_eq!(style.fg, Some(Color::Green));
    assert_eq!(style.bg, None);
}

#[test]
fn ansi16_del_style_uses_foreground_only() {
    let style = style_del(context(DiffTheme::Dark, DiffColorLevel::Ansi16));
    assert_eq!(style.fg, Some(Color::Red));
    assert_eq!(style.bg, None);
}

#[test]
fn ansi16_sign_styles_use_foreground_only() {
    let ansi16 = context(DiffTheme::Dark, DiffColorLevel::Ansi16);
    let add_sign = style_sign_add(ansi16);
    assert_eq!(add_sign.fg, Some(Color::Green));
    assert_eq!(add_sign.bg, None);
    let del_sign = style_sign_del(ansi16);
    assert_eq!(del_sign.fg, Some(Color::Red));
    assert_eq!(del_sign.bg, None);
}

#[test]
fn truecolor_dark_theme_uses_configured_backgrounds() {
    let backgrounds = fallback(DiffTheme::Dark, DiffColorLevel::TrueColor);
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(rgb_color((33, 58, 43)))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(rgb_color((74, 34, 29)))
    );
    for kind in [DiffLineType::Insert, DiffLineType::Delete] {
        assert_eq!(
            style_gutter_for(kind, DiffTheme::Dark, DiffColorLevel::TrueColor),
            style_gutter_dim()
        );
    }
}

#[test]
fn ansi256_dark_theme_uses_distinct_add_and_delete_backgrounds() {
    let backgrounds = fallback(DiffTheme::Dark, DiffColorLevel::Ansi256);
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(indexed_color(22))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(indexed_color(52))
    );
}

#[test]
fn theme_scope_backgrounds_override_truecolor_fallback_when_available() {
    let backgrounds = DiffRenderStyleContext::new(
        DiffTheme::Dark,
        DiffColorLevel::TrueColor,
        DiffScopeBackgrounds {
            inserted: Some(DiffScopeBackground::Rgb((1, 2, 3))),
            deleted: Some(DiffScopeBackground::Rgb((4, 5, 6))),
        },
    )
    .diff_backgrounds;
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(rgb_color((1, 2, 3)))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(rgb_color((4, 5, 6)))
    );
}

#[test]
fn explicit_default_scope_disables_only_its_diff_fill() {
    for theme in [DiffTheme::Light, DiffTheme::Dark] {
        for color_level in [
            DiffColorLevel::TrueColor,
            DiffColorLevel::Ansi256,
            DiffColorLevel::Ansi16,
        ] {
            let backgrounds = DiffRenderStyleContext::new(
                theme,
                color_level,
                DiffScopeBackgrounds {
                    inserted: Some(DiffScopeBackground::TerminalDefault),
                    deleted: None,
                },
            )
            .diff_backgrounds;
            assert_eq!(
                style_line_bg_for(DiffLineType::Insert, backgrounds),
                Style::default()
            );
            assert_eq!(
                style_line_bg_for(DiffLineType::Delete, backgrounds),
                style_line_bg_for(DiffLineType::Delete, fallback(theme, color_level))
            );
        }
    }
}

#[test]
fn theme_scope_backgrounds_quantize_to_ansi256() {
    let backgrounds = DiffRenderStyleContext::new(
        DiffTheme::Dark,
        DiffColorLevel::Ansi256,
        DiffScopeBackgrounds {
            inserted: Some(DiffScopeBackground::Rgb((0, 95, 0))),
            deleted: None,
        },
    )
    .diff_backgrounds;
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(indexed_color(22))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(indexed_color(52))
    );
}

/// Codex's `theme_scope_background_resolution` snapshot, inline.
#[test]
fn theme_scope_background_resolution() {
    let backgrounds = DiffRenderStyleContext::new(
        DiffTheme::Dark,
        DiffColorLevel::TrueColor,
        DiffScopeBackgrounds {
            inserted: Some(DiffScopeBackground::Rgb((12, 34, 56))),
            deleted: None,
        },
    )
    .diff_backgrounds;
    assert_eq!(
        format!(
            "insert={:?}\ndelete={:?}",
            style_line_bg_for(DiffLineType::Insert, backgrounds).bg,
            style_line_bg_for(DiffLineType::Delete, backgrounds).bg,
        ),
        "insert=Some(Rgb(12, 34, 56))\ndelete=Some(Rgb(74, 34, 29))"
    );
}

#[test]
fn ansi16_disables_line_and_gutter_backgrounds() {
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Insert,
            fallback(DiffTheme::Dark, DiffColorLevel::Ansi16)
        ),
        Style::default()
    );
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Delete,
            fallback(DiffTheme::Light, DiffColorLevel::Ansi16)
        ),
        Style::default()
    );
    for kind in [DiffLineType::Insert, DiffLineType::Delete] {
        assert_eq!(
            style_gutter_for(kind, DiffTheme::Light, DiffColorLevel::Ansi16),
            Style::default().fg(Color::Black)
        );
    }
    let themed = DiffRenderStyleContext::new(
        DiffTheme::Light,
        DiffColorLevel::Ansi16,
        DiffScopeBackgrounds {
            inserted: Some(DiffScopeBackground::Rgb((8, 9, 10))),
            deleted: Some(DiffScopeBackground::Rgb((11, 12, 13))),
        },
    )
    .diff_backgrounds;
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, themed),
        Style::default()
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, themed),
        Style::default()
    );
}

#[test]
fn light_truecolor_theme_uses_readable_gutter_and_line_backgrounds() {
    let backgrounds = fallback(DiffTheme::Light, DiffColorLevel::TrueColor);
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(rgb_color((218, 251, 225)))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(rgb_color((255, 235, 233)))
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Insert,
            DiffTheme::Light,
            DiffColorLevel::TrueColor
        ),
        Style::default()
            .fg(rgb_color((31, 35, 40)))
            .bg(rgb_color((172, 238, 187)))
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Delete,
            DiffTheme::Light,
            DiffColorLevel::TrueColor
        ),
        Style::default()
            .fg(rgb_color((31, 35, 40)))
            .bg(rgb_color((255, 206, 203)))
    );
}

#[test]
fn light_theme_wrapped_lines_keep_number_gutter_contrast() {
    let lines = push_wrapped_diff_line(
        DiffLineType::Insert,
        "abcdefghij",
        None,
        Some((12, line_number_width(12))),
        8,
        context(DiffTheme::Light, DiffColorLevel::TrueColor),
    );
    assert!(lines.len() > 1);
    let gutter = Style::default()
        .fg(rgb_color((31, 35, 40)))
        .bg(rgb_color((172, 238, 187)));
    assert_eq!(lines[0].spans[0].style, gutter);
    assert_eq!(lines[1].spans[0].style, gutter);
    assert_eq!(lines[0].style.bg, Some(rgb_color((218, 251, 225))));
    assert_eq!(lines[1].style.bg, Some(rgb_color((218, 251, 225))));
}

#[test]
fn the_terminals_colour_depth_picks_the_diff_palette() {
    use DiffColorLevel::{Ansi16, Ansi256, TrueColor};
    use StdoutColorLevel as Level;
    // (stdout, terminal, WT_SESSION, FORCE_COLOR) → depth, Codex's seven
    // Windows Terminal cases.
    for (stdout, terminal, wt_session, force, expected) in [
        (
            Level::Ansi16,
            TerminalName::WindowsTerminal,
            false,
            false,
            TrueColor,
        ),
        (Level::Ansi16, TerminalName::Unknown, true, false, TrueColor),
        (Level::Ansi16, TerminalName::WezTerm, false, false, Ansi16),
        (
            Level::Unknown,
            TerminalName::WindowsTerminal,
            true,
            false,
            TrueColor,
        ),
        (
            Level::Unknown,
            TerminalName::WindowsTerminal,
            false,
            false,
            Ansi16,
        ),
        (
            Level::Ansi16,
            TerminalName::WindowsTerminal,
            false,
            true,
            Ansi16,
        ),
        (
            Level::Ansi256,
            TerminalName::WindowsTerminal,
            true,
            true,
            Ansi256,
        ),
    ] {
        assert_eq!(
            diff_color_level_for_terminal(stdout, terminal, wt_session, force),
            expected,
            "{stdout:?} {terminal:?} {wt_session} {force}"
        );
    }
    // The terminal's background is not queried: dark, unless one is known.
    assert_eq!(diff_theme_for_bg(None), DiffTheme::Dark);
    assert_eq!(diff_theme_for_bg(Some((255, 255, 255))), DiffTheme::Light);
    assert_eq!(diff_theme_for_bg(Some((20, 20, 20))), DiffTheme::Dark);
}

/// Codex's test with its terminal's default colours: here the depth is the
/// context's, and the background the row's own tint.
#[test]
fn syntax_colors_follow_the_actual_diff_background() {
    let spans = [
        "let value = 1;".fg(rgb_color((235, 210, 170))).dim(),
        " plain".into(),
        " reset".fg(Color::Reset),
    ];
    let style = DiffRenderStyleContext {
        theme: DiffTheme::Light,
        color_level: DiffColorLevel::TrueColor,
        diff_backgrounds: ResolvedDiffBackgrounds {
            add: Some(rgb_color((220, 250, 230))),
            del: Some(rgb_color((55, 25, 35))),
        },
    };
    let lines: Vec<Line<'static>> = [DiffLineType::Insert, DiffLineType::Delete]
        .into_iter()
        .flat_map(|kind| {
            push_wrapped_diff_line(
                kind,
                "let value = 1; plain reset",
                Some(&spans),
                Some((1, 1)),
                40,
                style,
            )
        })
        .collect();
    for line in &lines {
        // Nothing dimmed; the theme colour made readable on each tint.
        assert!(line
            .spans
            .iter()
            .all(|span| !span.style.add_modifier.contains(Modifier::DIM)));
    }
    let fg = |row: usize| lines[row].spans[2].style.fg;
    assert_ne!(fg(0), fg(1));
    assert_eq!(lines[0].spans[3].style.fg, Some(Color::Reset));
    assert_eq!(lines[0].spans[4].style.fg, Some(Color::Reset));
}

// Wrapping (Codex's tests).

#[test]
fn wrap_styled_spans_single_line() {
    let result = wrap_styled_spans(&["short".into()], 80);
    assert_eq!(result.len(), 1);
}

#[test]
fn wrap_styled_spans_splits_long_content() {
    let result = wrap_styled_spans(&["a".repeat(100).into()], 40);
    assert!(result.len() >= 3, "{}", result.len());
}

#[test]
fn wrap_styled_spans_flushes_at_span_boundary() {
    let style_a = Style::default().fg(Color::Red);
    let style_b = Style::default().fg(Color::Blue);
    let spans = vec![
        ratatui::text::Span::styled("aaaa", style_a),
        ratatui::text::Span::styled("bb", style_b),
    ];
    let result = wrap_styled_spans(&spans, 4);
    assert_eq!(result.len(), 2, "{result:?}");
    let first_width: usize = result[0].iter().map(|s| s.content.chars().count()).sum();
    assert!(first_width <= 4);
}

#[test]
fn wrap_styled_spans_preserves_styles() {
    let style = Style::default().fg(Color::Green);
    let spans = vec![ratatui::text::Span::styled("x".repeat(50), style)];
    for chunk in wrap_styled_spans(&spans, 20) {
        for span in chunk {
            assert_eq!(span.style, style);
        }
    }
}

#[test]
fn wrap_styled_spans_tabs_have_visible_width() {
    let style = Style::default().fg(Color::Green);
    let spans = vec![ratatui::text::Span::styled("\tabcde", style)];
    assert_eq!(
        wrap_styled_spans(&spans, 8),
        vec![
            vec![ratatui::text::Span::styled("    abcd", style)],
            vec![ratatui::text::Span::styled("e", style)],
        ]
    );
}

#[test]
fn wrap_styled_spans_wraps_before_first_overflowing_char() {
    let result = wrap_styled_spans(&["abcd\t界".into()], 5);
    let rows: Vec<String> = result
        .iter()
        .map(|line| line.iter().map(|span| span.content.as_ref()).collect())
        .collect();
    assert_eq!(rows, vec!["abcd", "    ", "界"]);
    for line in &result {
        assert!(line_width(&Line::from(line.clone())) <= 5, "{line:?}");
    }
}

#[test]
fn fallback_wrapping_uses_display_width_for_tabs_and_wide_chars() {
    let width = 8;
    let lines = push_wrapped_diff_line(
        DiffLineType::Insert,
        "abcd\t界🙂",
        None,
        Some((1, line_number_width(1))),
        width,
        current_diff_render_style_context(),
    );
    assert!(lines.len() >= 2, "{lines:?}");
    for line in &lines {
        assert!(line_width(line) <= width, "{line:?}");
    }
}

/// Codex's `wrap_behavior_insert` snapshot (its rows, trimmed).
#[test]
fn wrap_behavior_insert() {
    let long_line =
        "this is a very long line that should wrap across multiple terminal columns and continue";
    let lines = push_wrapped_diff_line(
        DiffLineType::Insert,
        long_line,
        None,
        Some((1, line_number_width(1))),
        80,
        current_diff_render_style_context(),
    );
    assert_eq!(
        text(&lines),
        "1 +this is a very long line that should wrap across multiple terminal columns an\n   d continue"
    );
}

/// Codex's `ansi16_insert_delete_no_background` snapshot: rows without a
/// background, the sign and plain text in the row's colour.
#[test]
fn ansi16_insert_delete_no_background() {
    let ansi16 = context(DiffTheme::Dark, DiffColorLevel::Ansi16);
    let mut lines = push_wrapped_diff_line(
        DiffLineType::Insert,
        "added in ansi16 mode",
        None,
        Some((1, line_number_width(2))),
        80,
        ansi16,
    );
    lines.extend(push_wrapped_diff_line(
        DiffLineType::Delete,
        "deleted in ansi16 mode",
        None,
        Some((2, line_number_width(2))),
        80,
        ansi16,
    ));
    assert_eq!(
        text(&lines),
        "1 +added in ansi16 mode\n2 -deleted in ansi16 mode"
    );
    assert!(lines.iter().all(|line| line.style.bg.is_none()));
    assert_eq!(
        styled(&lines),
        "[1 ]~[+]{Green}[added in ansi16 mode]{Green}\n[2 ]~[-]{Red}[deleted in ansi16 mode]{Red}"
    );
}

/// The tint is the line's style, so it fills every row the line takes, the
/// continuation's indent included.
#[test]
fn a_tinted_row_keeps_its_background_on_every_wrapped_row() {
    let lines = push_wrapped_diff_line(
        DiffLineType::Delete,
        &"x".repeat(30),
        None,
        Some((7, 1)),
        12,
        dark_truecolor(),
    );
    assert_eq!(lines.len(), 4);
    for line in &lines {
        assert_eq!(line.style.bg, Some(rgb_color((74, 34, 29))));
        assert!(line_width(line) <= 12);
    }
    // The sign and the plain content: red on the tint.
    assert_eq!(
        lines[0].spans[1].style,
        Style::default().fg(Color::Red).bg(rgb_color((74, 34, 29)))
    );
}

// Numbering.

/// Codex's `apply_update_block`: its rows, without the four columns its
/// summary indents them by (here the cell's frame does).
#[test]
fn apply_update_block() {
    let preview = update_block();
    assert_eq!(
        text(&patch_diff_lines(&preview, 76)),
        "1  line one\n2 -line two\n2 +line two changed\n3  line three"
    );
    assert_eq!(
        text(&[Line::from(patch_title(&preview, false))]),
        "Edited example.txt (+1 -1)"
    );
}

#[test]
fn an_updated_files_rows_are_numbered_from_where_its_chunks_landed() {
    let preview = patch(vec![file(
        "update",
        "notes.txt",
        None,
        "@@ 1 1\n line 1\n-line 2\n+line two changed\n line 3\n@@ 8 8\n line 8\n-line 9\n+line nine changed\n line 10",
    )]);
    assert_eq!(
        text(&patch_diff_lines(&preview, 76)),
        [
            " 1  line 1",
            " 2 -line 2",
            " 2 +line two changed",
            " 3  line 3",
            "   ⋮",
            " 8  line 8",
            " 9 -line 9",
            " 9 +line nine changed",
            "10  line 10",
        ]
        .join("\n")
    );
}

#[test]
fn removed_rows_count_in_the_old_file_and_the_rest_in_the_new() {
    // Two lines become three, so the second chunk's old and new numbers
    // part.
    let preview = patch(vec![file(
        "update",
        "a.txt",
        None,
        "@@ 2 2\n b\n-c\n+c1\n+c2\n+c3\n@@ 6 8\n f\n-g\n+G",
    )]);
    assert_eq!(
        text(&patch_diff_lines(&preview, 76)),
        ["2  b", "3 -c", "3 +c1", "4 +c2", "5 +c3", "  ⋮", "8  f", "7 -g", "9 +G"].join("\n")
    );
}

#[test]
fn an_update_without_its_starts_is_unnumbered() {
    // A patch that did not apply, or one recorded before the starts were.
    let preview = patch(vec![file(
        "update",
        "notes.txt",
        None,
        "@@\n line 1\n-line 2\n+line two\n@@\n line 9\n*** End of File",
    )]);
    assert_eq!(
        text(&patch_diff_lines(&preview, 76)),
        " line 1\n-line 2\n+line two\n⋮\n line 9\n*** End of File"
    );
}

/// Codex's `apply_update_block_line_numbers_three_digits_text`.
#[test]
fn line_numbers_three_digits() {
    let body = format!(
        "@@ 97 97\n{}-line 100\n+line 100 changed\n{}",
        (97..100)
            .map(|n| format!(" line {n}\n"))
            .collect::<String>(),
        (101..104)
            .map(|n| format!(" line {n}\n"))
            .collect::<String>(),
    );
    let preview = patch(vec![file("update", "hundreds.txt", None, &body)]);
    assert_eq!(
        text(&patch_diff_lines(&preview, 76)),
        [
            " 97  line 97",
            " 98  line 98",
            " 99  line 99",
            "100 -line 100",
            "100 +line 100 changed",
            "101  line 101",
            "102  line 102",
            "103  line 103",
        ]
        .join("\n")
    );
}

/// Codex's `apply_add_block` at its narrowest (3 columns).
#[test]
fn an_added_file_at_three_columns() {
    let preview = patch(vec![file("add", "new_file.txt", None, "+alpha\n+beta")]);
    let lines = patch_diff_lines(&preview, 3);
    assert_eq!(text(&lines[..2]), "1 +a\n   l");
}

#[test]
fn a_marker_and_omitted_rows_stand_under_the_gutter() {
    let mut cut = file("update", "a.txt", None, "@@ 4 4\n x\n+y\n*** End of File");
    cut.omitted_lines = 12;
    assert_eq!(
        text(&patch_diff_lines(&patch(vec![cut]), 76)),
        "4  x\n5 +y\n  *** End of File\n  … +12 lines"
    );
}

#[test]
fn several_files_each_get_a_header_snapshot() {
    let mut renamed = file("update", "old.txt", Some("renamed.txt"), "@@\n-a\n+b");
    renamed.omitted_lines = 3;
    let preview = patch(vec![file("add", "new.txt", None, "+hello"), renamed]);
    let mut lines = vec![Line::from(patch_title(&preview, false))];
    lines.extend(patch_diff_lines(&preview, 60));
    insta::assert_snapshot!(text(&lines));
}

#[test]
fn an_unreadable_patch_says_why() {
    let preview = AgentPatchPreview {
        files: Vec::new(),
        error: Some("missing *** Begin Patch".to_string()),
    };
    let mut lines = vec![Line::from(patch_title(&preview, false))];
    lines.extend(patch_diff_lines(&preview, 60));
    assert_eq!(
        text(&lines),
        "Could not read the patch\nmissing *** Begin Patch"
    );
    assert_eq!(patch_summary_rows(&preview).len(), 1);
    assert!(!patch_has_lines(&preview));
}

// Highlighting.

fn rgb_spans(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| &line.spans)
        .filter(|span| matches!(span.style.fg, Some(Color::Rgb(..))))
        .map(|span| span.content.to_string())
        .collect()
}

fn rgb_theme() {
    set_syntax_theme(resolve_theme_by_name("base16-ocean-dark", None).unwrap());
}

/// The rows in 24-bit colour: with 16, a theme's 24-bit colours are not
/// shown (Codex's `readable_color_on`).
fn truecolor_lines(preview: &AgentPatchPreview, width: usize) -> Vec<Line<'static>> {
    patch_diff_lines_with_style(preview, width, dark_truecolor())
}

#[test]
fn a_patch_is_highlighted_by_its_files_language_snapshot() {
    rgb_theme();
    let preview = patch(vec![file(
        "update",
        "src/lib.rs",
        None,
        "@@ 1 1\n fn answer() -> u32 {\n-    41\n+    42 // the answer\n }",
    )]);
    insta::assert_snapshot!(styled(&truecolor_lines(&preview, 80)));
}

#[test]
fn add_and_rename_diffs_use_the_destinations_extension() {
    rgb_theme();
    let added = patch(vec![file(
        "add",
        "highlight_add.rs",
        None,
        "+pub fn sum(a: i32, b: i32) -> i32 { a + b }",
    )]);
    assert!(!rgb_spans(&truecolor_lines(&added, 80)).is_empty());
    let renamed = patch(vec![file(
        "update",
        "foo.xyzzy",
        Some("foo.rs"),
        "@@ 1 1\n-fn main() {}\n+fn main() { println!(\"hi\"); }",
    )]);
    assert!(!rgb_spans(&truecolor_lines(&renamed, 80)).is_empty());
}

#[test]
fn cpp_module_extensions_use_cpp_highlighting() {
    rgb_theme();
    for extension in [
        "cpp", "cu", "cuh", "cppm", "CPPM", "cxxm", "CxXm", "ixx", "IXX",
    ] {
        let preview = patch(vec![file(
            "add",
            &format!("math.{extension}"),
            None,
            "+export module math;\n+export int sum(int a, int b) { return a + b; }",
        )]);
        assert!(
            !rgb_spans(&truecolor_lines(&preview, 80)).is_empty(),
            "{extension}"
        );
    }
}

#[test]
fn unknown_extension_falls_back_without_syntax_highlighting() {
    rgb_theme();
    let preview = patch(vec![file(
        "add",
        "math.unknown-extension",
        None,
        "+export module math;\n+export int value = 42;",
    )]);
    assert!(rgb_spans(&truecolor_lines(&preview, 80)).is_empty());
}

#[test]
fn large_update_diff_skips_highlighting() {
    rgb_theme();
    let body = format!("@@ 1 1\n{}", "+let value = 1;\n".repeat(10_001));
    let preview = patch(vec![file("update", "huge.rs", None, &body)]);
    let lines = truecolor_lines(&preview, 80);
    assert!(lines.len() > 100);
    assert!(rgb_spans(&lines).is_empty());
}

#[test]
fn update_diff_preserves_multiline_highlight_state_within_hunk() {
    rgb_theme();
    let preview = patch(vec![file(
        "update",
        "demo.rs",
        None,
        "@@ 1 1\n fn demo() {\n-    let s = \"hello\";\n+    let s = \"hello\n+world\";\n }",
    )]);
    let expected =
        highlight_code_to_styled_spans("    let s = \"hello\nworld\";\n", "rust").unwrap();
    let expected = expected[1]
        .iter()
        .find(|span| span.content.contains("world"))
        .unwrap()
        .style;
    let lines = patch_diff_lines_with_style(
        &preview,
        120,
        context(DiffTheme::Dark, DiffColorLevel::TrueColor),
    );
    let actual = lines
        .iter()
        .flat_map(|line| &line.spans)
        .find(|span| span.content.contains("world"))
        .unwrap()
        .style;
    assert!(matches!(expected.fg, Some(Color::Rgb(..))));
    assert!(!actual.add_modifier.contains(Modifier::DIM));
    // The removed `"hello";` closes its string; the added one goes on into
    // `world`, still a string.
    assert_eq!(
        actual.fg,
        Some(crate::color::readable_color_on(
            expected.fg.unwrap(),
            Some(rgb_color((33, 58, 43))),
            StdoutColorLevel::TrueColor
        ))
    );
}

#[test]
fn each_chunk_is_highlighted_on_its_own() {
    rgb_theme();
    // A string left open in the first chunk does not run into the second.
    let preview = patch(vec![file(
        "update",
        "demo.rs",
        None,
        "@@ 1 1\n+let s = \"open\n@@ 40 41\n+fn closed() {}",
    )]);
    let lines = truecolor_lines(&preview, 80);
    let separate = highlight_code_to_styled_spans("fn closed() {}\n", "rust").unwrap();
    let keyword = separate[0][0].style.fg;
    let row = lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("closed"))
        })
        .unwrap();
    let fn_span = row
        .spans
        .iter()
        .find(|span| span.content.as_ref() == "fn")
        .expect("`fn` on its own span, as a keyword");
    assert_eq!(
        fn_span.style.fg,
        Some(crate::color::readable_color_on(
            keyword.unwrap(),
            Some(rgb_color((33, 58, 43))),
            StdoutColorLevel::TrueColor
        ))
    );
    // Highlighted as one chunk, the second line would still be a string.
    let joined =
        highlight_code_to_styled_spans("let s = \"open\nfn closed() {}\n", "rust").unwrap();
    assert_ne!(joined[1][0].style.fg, keyword);
}

// What the rows hold: never wider than their width, never a control.

#[test]
fn rows_never_exceed_their_width_at_any_width() {
    rgb_theme();
    let preview = gallery();
    for width in [120, 80, 40, 20, 12, 8] {
        for line in patch_diff_lines(&preview, width) {
            assert!(line_width(&line) <= width, "{width}: {line:?}");
        }
    }
}

#[test]
fn hostile_rows_are_shown_without_their_controls() {
    let preview = patch(vec![file(
        "update",
        "a.txt",
        None,
        "@@ 1 1\n-\u{1b}[31mred\u{1b}[0m\rx\n+bell\u{7}\u{1b}]8;;https://x\u{7}link\n+\u{0}nul",
    )]);
    let rendered = text(&patch_diff_lines(&preview, 40));
    assert_eq!(rendered, "1 -redx\n1 +belllink\n2 +nul");
    // A row of one long unbroken word, a wide character at the edge.
    let wide = patch(vec![file(
        "add",
        "wide.txt",
        None,
        &format!("+{}\n+中{}", "w".repeat(5_000), "文".repeat(30)),
    )]);
    for line in patch_diff_lines(&wide, 7) {
        assert!(line_width(&line) <= 7, "{line:?}");
    }
}

#[test]
fn a_git_diff_is_highlighted_per_file_snapshot() {
    rgb_theme();
    let diff = [
        "diff --git a/src/main.py b/src/main.py",
        "index 1111111..2222222 100644",
        "--- a/src/main.py",
        "+++ b/src/main.py",
        "@@ -1,2 +1,2 @@",
        " def main():",
        "-    return 1",
        "+    return 2",
        "diff --git a/notes.txt b/notes.txt",
        "--- a/notes.txt",
        "+++ b/notes.txt",
        "@@ -1 +1 @@",
        "-old words",
        "+new words",
        "\\ No newline at end of file",
    ]
    .join("\n");
    insta::assert_snapshot!(styled(&git_diff_lines(&diff)));
    assert_eq!(
        git_diff_lines("  \n"),
        vec![Line::from("No changes detected.".italic())]
    );
}
