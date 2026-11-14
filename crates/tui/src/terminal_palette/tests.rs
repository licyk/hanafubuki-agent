// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn best_color_uses_truecolor_without_quantization() {
    assert_eq!(
        best_color_for_level((12, 34, 56), StdoutColorLevel::TrueColor),
        rgb_color((12, 34, 56))
    );
}

#[test]
fn best_color_resets_for_ansi16() {
    assert_eq!(
        best_color_for_level((12, 34, 56), StdoutColorLevel::Ansi16),
        Color::Reset
    );
}

/// Codex keeps the xterm palette as a table; here it is computed. A few
/// entries of the table, its corners and ramps.
#[test]
fn the_computed_xterm_palette_is_codexs_table() {
    for (index, rgb) in [
        (16, (0, 0, 0)),
        (17, (0, 0, 95)),
        (22, (0, 95, 0)),
        (52, (95, 0, 0)),
        (157, (175, 255, 175)),
        (194, (215, 255, 215)),
        (217, (255, 175, 175)),
        (224, (255, 215, 215)),
        (231, (255, 255, 255)),
        (232, (8, 8, 8)),
        (236, (48, 48, 48)),
        (255, (238, 238, 238)),
    ] {
        assert_eq!(xterm_color(index), rgb, "{index}");
    }
    assert_eq!(nearest_xterm_color((33, 58, 43)), Color::Indexed(236));
    assert_eq!(nearest_xterm_color((255, 0, 0)), Color::Indexed(196));
}
