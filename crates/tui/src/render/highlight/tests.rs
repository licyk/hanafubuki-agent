// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::fs;
use std::path::Path;

use ratatui::style::{Color, Modifier};

use super::*;

/// A theme that paints keywords red and everything else white.
const CUSTOM_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>name</key><string>Mine</string>
<key>settings</key><array>
<dict><key>settings</key><dict><key>foreground</key><string>#FFFFFF</string></dict></dict>
<dict><key>scope</key><string>keyword, storage</string>
<key>settings</key><dict><key>foreground</key><string>#FF0000</string></dict></dict>
</array></dict></plist>
"#;

fn colour_of(lines: &[Line<'static>], text: &str) -> Option<Color> {
    lines
        .iter()
        .flat_map(|line| &line.spans)
        .find(|span| span.content == text)
        .and_then(|span| span.style.fg)
}

#[test]
fn known_languages_are_highlighted_by_name_alias_and_extension() {
    set_syntax_theme(resolve_theme_by_name("dracula", None).unwrap());
    for lang in ["rust", "Rust", "rs"] {
        let lines = highlight_code_to_lines("fn main() {}\n", lang);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.len() > 1, "{lang}: {lines:?}");
        assert!(colour_of(&lines, "fn").is_some(), "{lang}: {lines:?}");
    }
    for alias in ["golang", "python3", "shell", "csharp"] {
        assert!(find_syntax(alias).is_some(), "{alias}");
    }
}

#[test]
fn an_unknown_language_or_a_huge_input_is_plain_text() {
    let lines = highlight_code_to_lines("one\ntwo\n", "no-such-language");
    assert_eq!(lines, vec![Line::from("one"), Line::from("two")]);
    let long_line = "x".repeat(MAX_HIGHLIGHT_LINE_BYTES + 1);
    assert!(highlight_code_to_styled_spans(&long_line, "rust").is_none());
    assert!(exceeds_highlight_limits(MAX_HIGHLIGHT_BYTES + 1, 1));
    assert!(!exceeds_highlight_limits(10, 10));
    assert_eq!(highlight_code_to_lines("", "rust"), vec![Line::default()]);
}

#[test]
fn the_ansi_themes_use_the_terminals_palette() {
    set_syntax_theme(resolve_theme_by_name("ansi", None).unwrap());
    let lines = highlight_code_to_lines("fn main() { let x = 1; }\n", "rust");
    for span in lines.iter().flat_map(|line| &line.spans) {
        assert!(
            !matches!(span.style.fg, Some(Color::Rgb(..))),
            "{span:?} is not a palette colour"
        );
        assert!(!span.style.add_modifier.contains(Modifier::UNDERLINED));
    }
}

#[test]
fn a_custom_theme_is_read_from_the_themes_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("mine.tmTheme"), CUSTOM_THEME).unwrap();
    fs::write(dir.path().join("broken.tmTheme"), "not a plist").unwrap();
    fs::write(dir.path().join("notes.txt"), "not a theme").unwrap();

    assert_eq!(
        themes::apply_theme_setting(Some("mine"), Some(dir.path())),
        None
    );
    let lines = highlight_code_to_lines("fn main() {}\n", "rust");
    assert_eq!(colour_of(&lines, "fn"), Some(Color::Rgb(0xFF, 0, 0)));

    let names = list_available_themes(Some(dir.path()));
    assert!(names.contains(&entry("mine", true)));
    assert!(names.contains(&entry("dracula", false)));
    assert!(names
        .iter()
        .all(|entry| entry.name != "broken" && entry.name != "notes"));
    assert!(names.windows(2).all(|pair| pair[0].name <= pair[1].name));
}

#[test]
fn a_missing_or_unreadable_theme_falls_back_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("broken.tmTheme"), "not a plist").unwrap();
    set_syntax_theme(resolve_theme_by_name("dracula", None).unwrap());

    let warning = themes::apply_theme_setting(Some("nowhere"), Some(dir.path())).unwrap();
    assert!(warning.contains("\"nowhere\" was not found"), "{warning}");
    assert!(warning.contains("nowhere.tmTheme"), "{warning}");
    let warning = themes::apply_theme_setting(Some("broken"), Some(dir.path())).unwrap();
    assert!(warning.contains("could not be read"), "{warning}");
    // The theme in use stays; at launch, that is the default.
    let theme = current_syntax_theme();
    assert_eq!(
        theme.name,
        resolve_theme_by_name("dracula", None).unwrap().name
    );
    assert_eq!(themes::apply_theme_setting(None, None), None);
}

#[test]
fn the_default_theme_is_the_terminals_palette() {
    assert_eq!(
        themes::default_theme().name,
        resolve_theme_by_name("ansi", None).unwrap().name
    );
}

fn entry(name: &str, is_custom: bool) -> themes::ThemeEntry {
    themes::ThemeEntry {
        name: name.to_string(),
        is_custom,
    }
}

/// What the release appends to `THIRD-PARTY-LICENSES.txt`: the licences
/// of the grammars and themes two-face bundles into the binary. After a
/// two-face upgrade, `UPDATE_SYNTAX_LICENSES=1` writes them anew.
#[test]
fn the_bundled_grammars_and_themes_licences_are_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("syntax-licenses.txt");
    let expected = format!(
        "The syntax definitions and themes bundled by two-face \
         (https://github.com/CosmicHorrorDev/two-face), curated from bat \
         (https://github.com/sharkdp/bat), and their licences.\n\n{}",
        two_face::acknowledgement::listing().to_md()
    );
    // Some licences have Windows line endings, and so may a Windows
    // checkout.
    let expected = expected.replace("\r\n", "\n");
    if std::env::var_os("UPDATE_SYNTAX_LICENSES").is_some() {
        fs::write(&path, &expected).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    assert!(
        committed == expected,
        "{} is not two-face's; run the test with UPDATE_SYNTAX_LICENSES=1",
        path.display()
    );
}
