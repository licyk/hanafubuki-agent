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
pub(crate) mod setup;
pub(crate) mod terminal_title;
pub(crate) mod theme_picker;

use std::time::Instant;

use hanafubuki_agent::{AgentPermissionMode, AgentTuiPreferences};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use crate::agent_bridge::{SessionStatus, UsageSummary};
use crate::history_cell::{format_tokens_compact, permission_mode_name};
use crate::render::highlight::foreground_for_scopes;
use items::{parse_items, StatusLineItem, SurfaceItem, TerminalTitleItem};

const STATUS_LINE_SEPARATOR: &str = " · ";
const APP_NAME: &str = "hanafubuki-agent";
/// Braille dots, one frame per tenth of a second.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub(crate) const SPINNER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// The longest goal objective the status line shows.
const GOAL_CHARS: usize = 40;

/// What a command or a setup view changed, for the app to apply and, when
/// `save` says so, to write into `[tui]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppearanceEvent {
    /// `/theme` chose a theme, which is already the one in use.
    ThemeSaved(String),
    StatusLine {
        items: Option<Vec<StatusLineItem>>,
        colors: Option<bool>,
        save: bool,
    },
    TerminalTitle {
        items: Vec<TerminalTitleItem>,
        save: bool,
    },
    /// The title setup shows `Some` items in the title while it is open;
    /// `None` puts the title's own items back.
    PreviewTerminalTitle(Option<Vec<TerminalTitleItem>>),
}

/// The settings and what they show.
#[derive(Clone)]
pub(crate) struct Appearance {
    /// `tui.theme` as saved, which `/theme` marks.
    theme: Option<String>,
    status_line: Vec<StatusLineItem>,
    status_line_colors: bool,
    terminal_title: Vec<TerminalTitleItem>,
    title_preview: Option<Vec<TerminalTitleItem>>,
    values: SurfaceValues,
}

/// What the items show: the session's status, and what the chat knows.
#[derive(Clone, Debug, Default)]
pub(crate) struct SurfaceValues {
    pub(crate) directory: String,
    /// The model of the session, or of the next one.
    pub(crate) model: String,
    pub(crate) status: Option<SessionStatus>,
    pub(crate) task_running: bool,
}

impl Appearance {
    /// The settings `preferences` hold; with the warnings for the items
    /// they name that this interface does not know.
    pub(crate) fn new(preferences: &AgentTuiPreferences, directory: String) -> (Self, Vec<String>) {
        let (status_line, unknown_status) = parse_items(&preferences.status_line);
        let (terminal_title, unknown_title) = parse_items(&preferences.terminal_title);
        let mut warnings = Vec::new();
        if !unknown_status.is_empty() {
            warnings.push(unknown_items_warning("tui.statusLine", &unknown_status));
        }
        if !unknown_title.is_empty() {
            warnings.push(unknown_items_warning("tui.terminalTitle", &unknown_title));
        }
        let appearance = Self {
            theme: preferences.theme.clone(),
            status_line,
            status_line_colors: preferences.status_line_colors,
            terminal_title,
            title_preview: None,
            values: SurfaceValues {
                directory,
                ..SurfaceValues::default()
            },
        };
        (appearance, warnings)
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

    pub(crate) fn values(&self) -> &SurfaceValues {
        &self.values
    }

    pub(crate) fn values_mut(&mut self) -> &mut SurfaceValues {
        &mut self.values
    }

    pub(crate) fn apply(&mut self, event: AppearanceEvent) {
        match event {
            AppearanceEvent::ThemeSaved(name) => self.theme = Some(name),
            AppearanceEvent::StatusLine { items, colors, .. } => {
                if let Some(items) = items {
                    self.status_line = items;
                }
                if let Some(colors) = colors {
                    self.status_line_colors = colors;
                }
            }
            AppearanceEvent::TerminalTitle { items, .. } => {
                self.terminal_title = items;
                self.title_preview = None;
            }
            AppearanceEvent::PreviewTerminalTitle(items) => self.title_preview = items,
        }
    }

    /// The status line under the composer; `None` with no item to show.
    pub(crate) fn status_line(&self) -> Option<Line<'static>> {
        status_line(&self.status_line, self.status_line_colors, |item| {
            self.values.status_value(item)
        })
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
            StatusLineItem::CurrentDir => Some(display_directory(&self.directory)),
            StatusLineItem::GitBranch => status.and_then(|status| status.git_branch.clone()),
            StatusLineItem::ContextRemaining => {
                context_percent_used(usage).map(|used| format!("Context {}% left", 100 - used))
            }
            StatusLineItem::ContextUsed => {
                context_percent_used(usage).map(|used| format!("Context {used}% used"))
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

/// The items with a value, joined; in the theme's colours or dimmed.
pub(crate) fn status_line(
    items: &[StatusLineItem],
    colors: bool,
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
            Style::default().fg(foreground_for_scopes(scopes).unwrap_or(fallback))
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
fn display_directory(directory: &str) -> String {
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
