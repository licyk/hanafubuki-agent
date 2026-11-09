// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The themes: the ones two-face bundles, by their kebab-case names, and
//! custom `<name>.tmTheme` files in the themes directory (the data
//! directory's `themes/`), which the interface names at launch.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use syntect::highlighting::{Theme, ThemeSet};
use two_face::theme::EmbeddedThemeName;

/// With no `tui.theme`: bat's `ansi`, which colours with the terminal's own
/// palette and so reads on a light background and a dark one alike (Codex
/// picks between two themes by the background it queries; the terminal is
/// not queried here).
pub(crate) const DEFAULT_THEME: &str = "ansi";

static THEMES_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Sets the themes directory and makes `name` (the default for `None`)
/// the active theme. Returns a warning for the person when `name` names no
/// theme; the default is used then.
pub(crate) fn configure_theme(name: Option<&str>, themes_dir: Option<PathBuf>) -> Option<String> {
    let warning = apply_theme_setting(name, themes_dir.as_deref());
    if let Some(dir) = themes_dir {
        // Named once, at launch.
        let _ = THEMES_DIR.set(dir);
    }
    warning
}

/// Makes the theme `name` names in `dir` the active one; the warning when
/// it names none.
pub(super) fn apply_theme_setting(name: Option<&str>, dir: Option<&Path>) -> Option<String> {
    let name = name?;
    match resolve_theme_by_name(name, dir) {
        Some(theme) => {
            super::set_syntax_theme(theme);
            None
        }
        None => Some(missing_theme_warning(name, dir)),
    }
}

/// The directory custom themes are read from, once the interface named it.
pub(crate) fn themes_dir() -> Option<&'static Path> {
    THEMES_DIR.get().map(PathBuf::as_path)
}

/// The theme `name` names: a bundled one, else `<dir>/<name>.tmTheme`.
pub(crate) fn resolve_theme_by_name(name: &str, dir: Option<&Path>) -> Option<Theme> {
    if let Some(embedded) = embedded_theme(name) {
        return Some(two_face::theme::extra().get(embedded).clone());
    }
    ThemeSet::get_theme(custom_theme_path(name, dir?)).ok()
}

/// The theme the interface starts with when `tui.theme` names none.
pub(super) fn default_theme() -> Theme {
    resolve_theme_by_name(DEFAULT_THEME, None).unwrap_or_default()
}

/// A theme the picker lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThemeEntry {
    /// What `tui.theme` holds for it.
    pub(crate) name: String,
    /// Read from the themes directory rather than bundled.
    pub(crate) is_custom: bool,
}

/// The bundled themes and the custom ones that load, sorted by name; a
/// custom theme with a bundled theme's name is the bundled one.
pub(crate) fn list_available_themes(dir: Option<&Path>) -> Vec<ThemeEntry> {
    let mut entries: Vec<ThemeEntry> = BUNDLED_THEMES
        .iter()
        .map(|(name, _)| ThemeEntry {
            name: (*name).to_string(),
            is_custom: false,
        })
        .collect();
    let custom = dir
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("tmTheme"))
        .filter_map(|path| {
            let name = path.file_stem()?.to_str()?.to_string();
            ThemeSet::get_theme(&path).ok().map(|_| name)
        })
        .collect::<Vec<_>>();
    for name in custom {
        if !entries.iter().any(|entry| entry.name == name) {
            entries.push(ThemeEntry {
                name,
                is_custom: true,
            });
        }
    }
    entries.sort_by_cached_key(|entry| (entry.name.to_ascii_lowercase(), entry.name.clone()));
    entries
}

fn custom_theme_path(name: &str, dir: &Path) -> PathBuf {
    dir.join(format!("{name}.tmTheme"))
}

fn missing_theme_warning(name: &str, dir: Option<&Path>) -> String {
    let Some(dir) = dir else {
        return format!("The theme \"{name}\" was not found; the default theme is used.");
    };
    let path = custom_theme_path(name, dir);
    if path.exists() {
        format!(
            "The theme \"{name}\" at {} could not be read as a .tmTheme file; the default \
             theme is used.",
            path.display()
        )
    } else {
        format!(
            "The theme \"{name}\" was not found; the default theme is used. A custom theme \
             is a .tmTheme file at {}.",
            path.display()
        )
    }
}

fn embedded_theme(name: &str) -> Option<EmbeddedThemeName> {
    BUNDLED_THEMES
        .iter()
        .find(|(bundled, _)| *bundled == name)
        .map(|(_, theme)| *theme)
}

/// Every theme two-face bundles, by the name `tui.theme` gives it.
const BUNDLED_THEMES: [(&str, EmbeddedThemeName); 32] = [
    ("1337", EmbeddedThemeName::Leet),
    ("ansi", EmbeddedThemeName::Ansi),
    ("base16", EmbeddedThemeName::Base16),
    ("base16-256", EmbeddedThemeName::Base16_256),
    (
        "base16-eighties-dark",
        EmbeddedThemeName::Base16EightiesDark,
    ),
    ("base16-mocha-dark", EmbeddedThemeName::Base16MochaDark),
    ("base16-ocean-dark", EmbeddedThemeName::Base16OceanDark),
    ("base16-ocean-light", EmbeddedThemeName::Base16OceanLight),
    ("catppuccin-frappe", EmbeddedThemeName::CatppuccinFrappe),
    ("catppuccin-latte", EmbeddedThemeName::CatppuccinLatte),
    (
        "catppuccin-macchiato",
        EmbeddedThemeName::CatppuccinMacchiato,
    ),
    ("catppuccin-mocha", EmbeddedThemeName::CatppuccinMocha),
    ("coldark-cold", EmbeddedThemeName::ColdarkCold),
    ("coldark-dark", EmbeddedThemeName::ColdarkDark),
    ("dark-neon", EmbeddedThemeName::DarkNeon),
    ("dracula", EmbeddedThemeName::Dracula),
    ("github", EmbeddedThemeName::Github),
    ("gruvbox-dark", EmbeddedThemeName::GruvboxDark),
    ("gruvbox-light", EmbeddedThemeName::GruvboxLight),
    ("inspired-github", EmbeddedThemeName::InspiredGithub),
    ("monokai-extended", EmbeddedThemeName::MonokaiExtended),
    (
        "monokai-extended-bright",
        EmbeddedThemeName::MonokaiExtendedBright,
    ),
    (
        "monokai-extended-light",
        EmbeddedThemeName::MonokaiExtendedLight,
    ),
    (
        "monokai-extended-origin",
        EmbeddedThemeName::MonokaiExtendedOrigin,
    ),
    ("nord", EmbeddedThemeName::Nord),
    ("one-half-dark", EmbeddedThemeName::OneHalfDark),
    ("one-half-light", EmbeddedThemeName::OneHalfLight),
    ("solarized-dark", EmbeddedThemeName::SolarizedDark),
    ("solarized-light", EmbeddedThemeName::SolarizedLight),
    ("sublime-snazzy", EmbeddedThemeName::SublimeSnazzy),
    ("two-dark", EmbeddedThemeName::TwoDark),
    ("zenburn", EmbeddedThemeName::Zenburn),
];
