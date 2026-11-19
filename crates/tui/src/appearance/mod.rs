// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What `/theme`, `/statusline` and `/title` set: the syntax theme's name,
//! the status line under the composer and the terminal's title, each a
//! list of items ([`items`]) filled from what the chat knows of the
//! session ([`SurfaceValues`]).
//!
//! The lists start as `[tui]` has them; a command's arguments change them
//! for this run only, and a setup view saves them (Codex's
//! `chatwidget/status_surfaces.rs` and `status_surface_commands.rs`).

pub(crate) mod items;
pub(crate) mod palette;
pub(crate) mod setup;
pub(crate) mod terminal_title;
pub(crate) mod theme_picker;

use std::time::Instant;

use hanafubuki_agent::{
    AgentPermissionMode, AgentStatusLineOverflow, AgentStatusLinePlace, AgentTuiPreferences,
};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use crate::agent_bridge::{SessionStatus, UsageSummary};
use crate::history_cell::{format_tokens_compact, permission_mode_name};
use crate::render::highlight::foreground_for_scopes;
use crate::update_notice::TuiUpdateNotice;
use items::{parse_items, StatusLineItem, SurfaceItem, TerminalTitleItem};

const STATUS_LINE_SEPARATOR: &str = " · ";
const APP_NAME: &str = "hanafubuki-agent";
/// Braille dots, one frame per tenth of a second.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub(crate) const SPINNER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// The longest goal objective the status line shows.
const GOAL_CHARS: usize = 40;

/// The colour `value` names: one of the sixteen ANSI names, or `#rrggbb`;
/// `None` for anything else, which the caller warns of and leaves to the
/// theme.
fn parse_color(value: &str) -> Option<ratatui::style::Color> {
    use ratatui::style::Color;
    let named = match value.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "white" => Color::White,
        hex => {
            let digits = hex.strip_prefix('#')?;
            if digits.len() != 6 || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
                return None;
            }
            let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
            return Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?));
        }
    };
    Some(named)
}

/// What a command or a setup view changed, for the app to apply and, when
/// `save` says so, to write into `[tui]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppearanceEvent {
    /// `/theme` chose a theme, which is already the one in use.
    ThemeSaved(String),
    StatusLine {
        items: Option<Vec<StatusLineItem>>,
        colors: Option<bool>,
        /// `tui.statusLinePosition.<item>` as the setup view set them.
        positions: Option<std::collections::BTreeMap<String, AgentStatusLinePlace>>,
        /// `tui.statusLineOverflow`, when the view changed it.
        overflow: Option<AgentStatusLineOverflow>,
        /// `tui.statusLinePalette` as the palette view set it, each entry
        /// `id:color`; `None` leaves it as it was.
        palette: Option<Vec<String>>,
        save: bool,
    },
    TerminalTitle {
        items: Vec<TerminalTitleItem>,
        save: bool,
    },
    /// The title setup shows `Some` items in the title while it is open;
    /// `None` puts the title's own items back.
    PreviewTerminalTitle(Option<Vec<TerminalTitleItem>>),
    /// A newer release, which the session header shows for the rest of the run.
    UpdateNotice(TuiUpdateNotice),
}

/// One count the footer's right side shows, and what a click on it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideItem {
    Terminals,
    Agents,
    Warnings,
}

/// A count, and the word after it, `1 agent` or `3 agents`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SideCount {
    pub(crate) item: SideItem,
    pub(crate) count: usize,
    singular: &'static str,
    plural: &'static str,
}

impl SideCount {
    pub(crate) fn new(
        item: SideItem,
        count: usize,
        singular: &'static str,
        plural: &'static str,
    ) -> Self {
        Self {
            item,
            count,
            singular,
            plural,
        }
    }

    /// The count and its word, `3 agents`.
    pub(crate) fn text(&self) -> String {
        format!(
            "{} {}",
            self.count,
            if self.count == 1 {
                self.singular
            } else {
                self.plural
            }
        )
    }
}

/// The settings and what they show.
#[derive(Clone)]
pub(crate) struct Appearance {
    /// `tui.theme` as saved, which `/theme` marks.
    theme: Option<String>,
    status_line: Vec<StatusLineItem>,
    /// Where each item sits; an item without an entry is on the left of the
    /// first row.
    positions: std::collections::BTreeMap<String, AgentStatusLinePlace>,
    overflow: AgentStatusLineOverflow,
    status_line_colors: bool,
    /// The colour `tui.statusLinePalette` gives an item, by its id.
    palette: std::collections::BTreeMap<String, ratatui::style::Color>,
    terminal_title: Vec<TerminalTitleItem>,
    title_preview: Option<Vec<TerminalTitleItem>>,
    values: SurfaceValues,
    update_notice: Option<TuiUpdateNotice>,
}

/// What the items show: the session's status, and what the chat knows.
#[derive(Clone, Debug, Default)]
pub(crate) struct SurfaceValues {
    pub(crate) directory: String,
    /// The model of the session, or of the next one.
    pub(crate) model: String,
    pub(crate) status: Option<SessionStatus>,
    pub(crate) task_running: bool,
    /// The warnings `/warnings` would list, those dismissed left out. The
    /// Agent's own are known only when it is asked, so this is as fresh as
    /// the last listing.
    pub(crate) warnings: usize,
}

impl Appearance {
    /// The settings `preferences` hold; with the warnings for the items
    /// they name that this interface does not know.
    pub(crate) fn new(preferences: &AgentTuiPreferences, directory: String) -> (Self, Vec<String>) {
        // An item may carry its place after a colon (`model:right-2`), which
        // the one setting then holds beside the id.
        let mut ids = Vec::new();
        let mut places = Vec::new();
        let mut bad_places = Vec::new();
        for entry in &preferences.status_line {
            match entry.split_once(':') {
                Some((id, place)) => match AgentStatusLinePlace::parse(place) {
                    Some(place) => {
                        ids.push(id.to_string());
                        places.push((id.to_string(), place));
                    }
                    None => bad_places.push(entry.clone()),
                },
                None => ids.push(entry.clone()),
            }
        }
        let (status_line, unknown_status) = parse_items(&ids);
        let (terminal_title, unknown_title) = parse_items(&preferences.terminal_title);
        let mut warnings = Vec::new();
        if !bad_places.is_empty() {
            warnings.push(format!(
                "tui.statusLine: {} is not `id` or `id:left-1`, `id:left-2`, `id:right-1`, \
                 `id:right-2`; it is ignored.",
                bad_places.join(", ")
            ));
        }
        if !unknown_status.is_empty() {
            warnings.push(unknown_items_warning("tui.statusLine", &unknown_status));
        }
        if !unknown_title.is_empty() {
            warnings.push(unknown_items_warning("tui.terminalTitle", &unknown_title));
        }
        let mut positions = std::collections::BTreeMap::new();
        for (item, place) in places {
            positions.insert(item, place);
        }
        // `tui.statusLinePalette`: each entry `id:color`. An item the
        // interface does not know, or a colour it cannot read, is left to
        // the theme with a warning.
        let (palette, bad_palette) = palette::parse(&preferences.status_line_palette);
        if !bad_palette.is_empty() {
            warnings.push(format!(
                "tui.statusLinePalette: {} is not `id:color` (a name like cyan or #rrggbb); it is ignored.",
                bad_palette.join(", ")
            ));
        }
        let appearance = Self {
            theme: preferences.theme.clone(),
            status_line,
            positions,
            overflow: preferences.status_line_overflow,
            status_line_colors: preferences.status_line_colors,
            palette,
            terminal_title,
            title_preview: None,
            values: SurfaceValues {
                directory,
                ..SurfaceValues::default()
            },
            update_notice: None,
        };
        (appearance, warnings)
    }

    /// The settings `preferences` hold in place of these; what the items
    /// show and the update notice are kept, and the unknown items were
    /// warned of at start.
    pub(crate) fn adopt(&mut self, preferences: &AgentTuiPreferences) {
        let (adopted, _) = Self::new(preferences, String::new());
        *self = Self {
            values: std::mem::take(&mut self.values),
            update_notice: self.update_notice.take(),
            ..adopted
        };
    }

    pub(crate) fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    pub(crate) fn status_line_items(&self) -> &[StatusLineItem] {
        &self.status_line
    }

    pub(crate) fn status_line_colors(&self) -> bool {
        self.status_line_colors
    }

    pub(crate) fn terminal_title_items(&self) -> &[TerminalTitleItem] {
        &self.terminal_title
    }

    pub(crate) fn update_notice(&self) -> Option<&TuiUpdateNotice> {
        self.update_notice.as_ref()
    }

    pub(crate) fn values(&self) -> &SurfaceValues {
        &self.values
    }

    pub(crate) fn values_mut(&mut self) -> &mut SurfaceValues {
        &mut self.values
    }

    pub(crate) fn apply(&mut self, event: AppearanceEvent) {
        match event {
            AppearanceEvent::ThemeSaved(name) => self.theme = Some(name),
            AppearanceEvent::StatusLine {
                items,
                colors,
                positions,
                overflow,
                palette,
                ..
            } => {
                if let Some(items) = items {
                    self.status_line = items;
                }
                if let Some(colors) = colors {
                    self.status_line_colors = colors;
                }
                if let Some(positions) = positions {
                    self.positions = positions;
                }
                if let Some(overflow) = overflow {
                    self.overflow = overflow;
                }
                if let Some(palette) = palette {
                    self.palette = palette::parse(&palette).0;
                }
            }
            AppearanceEvent::TerminalTitle { items, .. } => {
                self.terminal_title = items;
                self.title_preview = None;
            }
            AppearanceEvent::PreviewTerminalTitle(items) => self.title_preview = items,
            AppearanceEvent::UpdateNotice(notice) => self.update_notice = Some(notice),
        }
    }

    /// The items placed at `place`, in the order `tui.statusLine` holds.
    fn placed(&self, place: AgentStatusLinePlace) -> Vec<StatusLineItem> {
        self.status_line
            .iter()
            .copied()
            .filter(|item| self.place_of(*item) == place)
            .collect()
    }

    /// Where `item` sits: the place its `tui.statusLine` entry names, or the
    /// left of the first row.
    fn place_of(&self, item: StatusLineItem) -> AgentStatusLinePlace {
        self.positions.get(item.id()).copied().unwrap_or_default()
    }

    /// How many rows the status line needs: the highest row an item sits on,
    /// never more than the rows a place may name.
    pub(crate) fn rows(&self) -> u8 {
        self.status_line
            .iter()
            .map(|item| self.place_of(*item).row)
            .max()
            .unwrap_or(1)
            .min(hanafubuki_agent::STATUS_LINE_ROWS)
    }

    /// What the footer does when a row does not fit beside the counts.
    pub(crate) fn overflow(&self) -> crate::bottom_pane::Overflow {
        match self.overflow {
            AgentStatusLineOverflow::WrapLeft => crate::bottom_pane::Overflow::WrapLeft,
            AgentStatusLineOverflow::WrapAll => crate::bottom_pane::Overflow::WrapAll,
            AgentStatusLineOverflow::Truncate => crate::bottom_pane::Overflow::Truncate,
        }
    }

    /// Where the item with `id` sits, `left-1` and the like; `None` for the
    /// items that are not the status line's.
    pub(crate) fn place_label(&self, id: &str) -> Option<String> {
        StatusLineItem::parse(id).map(|item| self.place_of(item).value())
    }

    /// The overflow rule as its setting writes it.
    pub(crate) fn overflow_setting(&self) -> AgentStatusLineOverflow {
        self.overflow
    }

    /// The rows after the first, on the left, in order; empty with no item
    /// on any of them.
    pub(crate) fn further_rows(&self) -> Vec<Line<'static>> {
        (2..=self.rows())
            .filter_map(|row| {
                let placed = self.placed(AgentStatusLinePlace { row, right: false });
                status_line(&placed, self.status_line_colors, &self.palette, |item| {
                    self.values.status_value(item)
                })
            })
            .collect()
    }

    /// The status line under the composer; `None` with no item to show.
    pub(crate) fn status_line(&self) -> Option<Line<'static>> {
        let placed = self.placed(AgentStatusLinePlace::default());
        let mut line = status_line(&placed, self.status_line_colors, &self.palette, |item| {
            self.values.status_value(item)
        })
        .unwrap_or_default();
        if let Some(status) = &self.values.status {
            let mut spans = vec![ratatui::text::Span::raw(
                crate::text_formatting::strip_terminal_controls(&status.agent_label),
            )];
            if !line.spans.is_empty() {
                spans.push(" · ".into());
            }
            if !self.status_line_colors {
                for span in &mut spans {
                    span.style = span.style.dim();
                }
            }
            spans.append(&mut line.spans);
            line.spans = spans;
        }
        (!line.spans.is_empty()).then_some(line)
    }

    /// The footer's counts, under the composer: each running terminal, then
    /// each agent, then the warnings `/warnings` would list; `None` while
    /// there is none of any.
    pub(crate) fn side_counts(&self) -> Vec<SideCount> {
        let background = self
            .values
            .status
            .as_ref()
            .and_then(|status| status.tree_background.as_ref());
        let mut counts = Vec::new();
        if let Some(background) = background {
            counts.push(SideCount::new(
                SideItem::Terminals,
                background.work.terminals,
                "terminal",
                "terminals",
            ));
            counts.push(SideCount::new(
                SideItem::Agents,
                background.work.agents,
                "agent",
                "agents",
            ));
        }
        counts.push(SideCount::new(
            SideItem::Warnings,
            self.values.warnings,
            "warning",
            "warnings",
        ));
        counts.retain(|count| count.count > 0);
        counts
    }

    /// The terminal's title at `now` (the spinner's frame); empty with no
    /// item to show.
    pub(crate) fn terminal_title(&self, now: Instant) -> String {
        let items = self.title_preview.as_ref().unwrap_or(&self.terminal_title);
        terminal_title_text(items, |item| self.values.title_value(item, now))
    }

    /// Whether the title changes with time now.
    pub(crate) fn title_is_animated(&self) -> bool {
        let items = self.title_preview.as_ref().unwrap_or(&self.terminal_title);
        self.values.task_running && items.contains(&TerminalTitleItem::Activity)
    }
}

impl SurfaceValues {
    pub(crate) fn status_value(&self, item: StatusLineItem) -> Option<String> {
        let status = self.status.as_ref();
        let usage = status.and_then(|status| status.usage.as_ref());
        let effort = status.and_then(|status| status.reasoning_effort.clone());
        let model = status.map_or(&self.model, |status| &status.model);
        match item {
            StatusLineItem::Model => Some(model.clone()).filter(|model| !model.is_empty()),
            StatusLineItem::ModelWithReasoning => {
                let model = Some(model.clone()).filter(|model| !model.is_empty())?;
                Some(match effort {
                    Some(effort) => format!("{model} {effort}"),
                    None => model,
                })
            }
            StatusLineItem::Reasoning => effort,
            StatusLineItem::BackgroundTasks => {
                // The counts are the footer's second row now
                // ([`Appearance::side_line`]); the item is kept so the
                // setting still names it, and shows nothing here.
                None
            }
            StatusLineItem::CurrentDir => Some(display_directory(&self.directory)),
            StatusLineItem::GitBranch => status.and_then(|status| status.git_branch.clone()),
            StatusLineItem::ContextRemaining => {
                context_percent_used(usage).map(|used| format!("Context {}% left", 100 - used))
            }
            StatusLineItem::ContextUsed => {
                context_percent_used(usage).map(|used| format!("Context {used}% used"))
            }
            StatusLineItem::ContextWindow => usage
                .and_then(|usage| usage.context)
                .map(|(_, window)| format!("Context {}", format_tokens_compact(window))),
            StatusLineItem::ContextUsedTokens => usage
                .and_then(|usage| usage.context)
                .map(|(used, _)| format!("{} used", format_tokens_compact(used))),
            StatusLineItem::ContextLeftTokens => {
                usage.and_then(|usage| usage.context).map(|(used, window)| {
                    format!(
                        "{} left",
                        format_tokens_compact(window.saturating_sub(used))
                    )
                })
            }
            StatusLineItem::CacheHitRate => {
                let usage = usage.filter(|usage| usage.cache_reported)?;
                let total = usage.last_input_tokens.max(1);
                let rate = usage.cache_read_tokens.saturating_mul(100) / total;
                Some(format!("Cache {rate}% hit"))
            }
            StatusLineItem::CacheHitTokens => {
                let usage = usage.filter(|usage| usage.cache_reported)?;
                Some(format!(
                    "Cache {} read",
                    format_tokens_compact(usage.cache_read_tokens)
                ))
            }
            StatusLineItem::CacheMissTokens => {
                let usage = usage.filter(|usage| usage.cache_reported)?;
                // The prompt total is uncached input plus what the cache
                // read and wrote, so the rest is what it did not answer.
                let miss = usage
                    .last_input_tokens
                    .saturating_sub(usage.cache_read_tokens)
                    .saturating_sub(usage.cache_write_tokens);
                Some(format!("Cache {} uncached", format_tokens_compact(miss)))
            }
            StatusLineItem::UsedTokens => usage
                .map(|usage| usage.input_tokens + usage.output_tokens)
                .filter(|total| *total > 0)
                .map(|total| format!("{} used", format_tokens_compact(total))),
            StatusLineItem::TotalInputTokens => {
                usage.map(|usage| format!("{} in", format_tokens_compact(usage.input_tokens)))
            }
            StatusLineItem::TotalOutputTokens => {
                usage.map(|usage| format!("{} out", format_tokens_compact(usage.output_tokens)))
            }
            StatusLineItem::SessionId => status.map(|status| status.session_id.to_string()),
            StatusLineItem::SessionTitle => self.session_title(),
            StatusLineItem::TaskProgress => self.task_progress(),
            StatusLineItem::PermissionMode => status
                .and_then(|status| status.permission_mode)
                .map(|mode: AgentPermissionMode| permission_mode_name(mode).to_string()),
            StatusLineItem::Goal => status.and_then(|status| status.goal.as_ref()).map(|goal| {
                let objective: String = goal.objective.chars().take(GOAL_CHARS).collect();
                let ellipsis = if goal.objective.chars().count() > GOAL_CHARS {
                    "…"
                } else {
                    ""
                };
                let state = format!("{:?}", goal.status).to_lowercase();
                format!("Goal: {objective}{ellipsis} ({state})")
            }),
            StatusLineItem::Advisor => status
                .and_then(|s| s.advisor.as_ref())
                .filter(|a| a.active || a.consultations > 0)
                .map(|a| {
                    let budget = a.limit.map(|n| format!("/{n}")).unwrap_or_default();
                    format!(
                        "Advisor {}{budget}{}",
                        a.consultations,
                        if a.active { "" } else { " (off)" }
                    )
                }),
            StatusLineItem::Role => status
                .and_then(|status| status.role.clone())
                .map(|role| format!("Role: {role}")),
            StatusLineItem::Compression => {
                status
                    .and_then(|status| status.compression)
                    .map(|compression| {
                        let blocks = match compression.active_blocks {
                            1 => "1 block".to_string(),
                            count => format!("{count} blocks"),
                        };
                        format!(
                            "{blocks}, {} saved",
                            format_tokens_compact(compression.tokens_compressed)
                        )
                    })
            }
        }
    }

    pub(crate) fn title_value(&self, item: TerminalTitleItem, now: Instant) -> Option<String> {
        match item {
            TerminalTitleItem::AppName => Some(APP_NAME.to_string()),
            TerminalTitleItem::ProjectName => std::path::Path::new(&self.directory)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            TerminalTitleItem::CurrentDir => Some(display_directory(&self.directory)),
            TerminalTitleItem::Activity => {
                self.task_running.then(|| spinner_frame(now).to_string())
            }
            TerminalTitleItem::RunState => Some(
                if self.task_running {
                    "Working"
                } else {
                    "Ready"
                }
                .to_string(),
            ),
            TerminalTitleItem::SessionTitle => self.session_title(),
            TerminalTitleItem::Model => self.status_value(StatusLineItem::Model),
            TerminalTitleItem::TaskProgress => self.task_progress(),
        }
    }

    fn session_title(&self) -> Option<String> {
        self.status.as_ref()?.title.clone()
    }

    fn task_progress(&self) -> Option<String> {
        let (done, total) = self.status.as_ref()?.task_progress?;
        (total > 0).then(|| format!("Tasks {done}/{total}"))
    }
}

/// The items with a value, joined; in the colours `palette` gives them, else
/// the theme's, or dimmed without `colors`.
pub(crate) fn status_line(
    items: &[StatusLineItem],
    colors: bool,
    palette: &std::collections::BTreeMap<String, ratatui::style::Color>,
    value: impl Fn(StatusLineItem) -> Option<String>,
) -> Option<Line<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for item in items {
        let Some(text) = value(*item) else {
            continue;
        };
        if !spans.is_empty() {
            spans.push(STATUS_LINE_SEPARATOR.dim());
        }
        let style = if colors {
            let (scopes, fallback) = item.accent();
            let color = palette
                .get(item.id())
                .copied()
                .or_else(|| foreground_for_scopes(scopes))
                .unwrap_or(fallback);
            Style::default().fg(color)
        } else {
            Style::default().dim()
        };
        spans.push(Span::styled(text, style));
    }
    (!spans.is_empty()).then(|| Line::from(spans))
}

/// The items with a value, joined as a title.
pub(crate) fn terminal_title_text(
    items: &[TerminalTitleItem],
    value: impl Fn(TerminalTitleItem) -> Option<String>,
) -> String {
    let mut title = String::new();
    let mut previous = None;
    for item in items {
        let Some(text) = value(*item) else {
            continue;
        };
        title.push_str(item.separator_after(previous));
        title.push_str(&text);
        previous = Some(*item);
    }
    title
}

fn spinner_frame(now: Instant) -> &'static str {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(|| now);
    let tick = now.saturating_duration_since(start).as_millis() / SPINNER_INTERVAL.as_millis();
    SPINNER_FRAMES[(tick % SPINNER_FRAMES.len() as u128) as usize]
}

/// The share of the context window in use, when the window is known.
fn context_percent_used(usage: Option<&UsageSummary>) -> Option<u64> {
    let (consumed, window) = usage?.context.filter(|(_, window)| *window > 0)?;
    Some((consumed.saturating_mul(100) / window).min(100))
}

/// The directory with the home directory written `~`.
pub(crate) fn display_directory(directory: &str) -> String {
    let home = std::env::home_dir().map(|home| home.display().to_string());
    match home
        .as_deref()
        .and_then(|home| directory.strip_prefix(home))
    {
        Some(rest) if rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR) => {
            format!("~{rest}")
        }
        _ => directory.to_string(),
    }
}

fn unknown_items_warning(key: &str, ids: &[String]) -> String {
    format!(
        "{key} names items this interface does not know, which are left out: {}",
        ids.join(", ")
    )
}

/// The ids of `T` for a usage message.
pub(crate) fn known_ids<T: SurfaceItem>() -> String {
    T::ALL
        .iter()
        .map(|item| item.id())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
