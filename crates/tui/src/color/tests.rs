// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

fn resolve(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(index) => xterm_color(index),
        _ => panic!("expected an explicit color: {color:?}"),
    }
}

#[test]
fn foreground_meets_minimum_after_palette_conversion() {
    for background in [
        (255, 255, 255),
        (245, 240, 220),
        (130, 130, 130),
        (18, 20, 30),
        // The diff's dark tints.
        (33, 58, 43),
        (74, 34, 29),
    ] {
        for preferred in [(139, 98, 20), (196, 167, 103), (98, 114, 164), (40, 40, 40)] {
            for level in [StdoutColorLevel::TrueColor, StdoutColorLevel::Ansi256] {
                let rgb = resolve(foreground(preferred, Some(background), level));
                assert!(
                    ratio(rgb, background) >= MIN_TEXT_CONTRAST,
                    "{rgb:?} on {background:?}"
                );
            }
        }
    }
}

#[test]
fn repeated_colors_follow_palette_changes_and_remain_bounded() {
    let preferred = (95, 175, 255);
    let backgrounds = [None, Some((24, 24, 24)), Some((245, 245, 245))];
    let levels = [
        StdoutColorLevel::TrueColor,
        StdoutColorLevel::Ansi256,
        StdoutColorLevel::Ansi16,
        StdoutColorLevel::Unknown,
    ];
    let expected =
        backgrounds.map(|background| levels.map(|level| foreground(preferred, background, level)));
    for index in 0..MAX_CACHED_FOREGROUNDS * 2 {
        let varied = (index as u8, (index / 256) as u8, 42);
        foreground(varied, None, StdoutColorLevel::TrueColor);
        assert_eq!(
            backgrounds
                .map(|background| levels.map(|level| foreground(preferred, background, level))),
            expected,
        );
        FOREGROUNDS.with(|cache| assert!(cache.borrow().len() <= MAX_CACHED_FOREGROUNDS));
    }
    assert_ne!(expected[1][0], expected[2][0]);
    assert_eq!(expected[0][0], rgb_color(preferred));
    assert_eq!(expected[0][2..], [Color::Reset, Color::Reset]);
}

/// The terminal's own colours are its theme's, never adjusted.
#[test]
fn the_terminals_own_colours_stay_as_they_are() {
    let dark_green = Some(Color::Rgb(33, 58, 43));
    for color in [Color::Reset, Color::Green, Color::Indexed(3), Color::Gray] {
        for level in [StdoutColorLevel::TrueColor, StdoutColorLevel::Ansi16] {
            assert_eq!(readable_color_on(color, dark_green, level), color);
        }
    }
    // A theme colour too dark for the tint is lightened.
    let lightened = readable_color_on(
        Color::Rgb(40, 40, 40),
        dark_green,
        StdoutColorLevel::TrueColor,
    );
    assert!(ratio(resolve(lightened), (33, 58, 43)) >= MIN_TEXT_CONTRAST);
}
