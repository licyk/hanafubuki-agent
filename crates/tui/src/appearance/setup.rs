// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The setup views of `/statusline` and `/title` (Codex's
//! `bottom_pane/status_line_setup.rs`, `title_setup.rs` and
//! `status_surface_preview.rs`): the items to turn on and order, with a
//! preview line that shows each item's value, or a sample of it until the
//! session has one. Enter saves; the title is previewed in the terminal
//! while the view is open, and Esc puts it back.

use std::time::Instant;

use ratatui::text::Line;

use super::items::{parse_items, StatusLineItem, SurfaceItem, TerminalTitleItem};
use super::{status_line, terminal_title_text, Appearance, AppearanceEvent, SurfaceValues};
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{MultiSelectItem, MultiSelectPicker};
use hanafubuki_agent::{AgentStatusLineOverflow, AgentStatusLinePlace};

/// The setup's own entry: whether the items take the theme's colours.
const COLORS_ID: &str = "use-theme-colors";

pub(crate) fn status_line_setup(appearance: &Appearance, tx: AppEventSender) -> MultiSelectPicker {
    let mut items = select_items(appearance.status_line_items());
    items.push(MultiSelectItem {
        id: COLORS_ID.to_string(),
        name: "Use theme colors".to_string(),
        description: Some("Color the items with the syntax theme".to_string()),
        enabled: appearance.status_line_colors(),
        note: None,
    });
    let values = appearance.values().clone();
    for item in &mut items {
        if let Some(place) = appearance.place_label(&item.id) {
            item.note = Some(place);
        }
    }
    let overflow = std::rc::Rc::new(std::cell::Cell::new(appearance.overflow_setting()));
    let confirmed = overflow.clone();
    let keys = overflow.clone();
    MultiSelectPicker::new(
        "Configure Status Line".to_string(),
        Some(
            "Select which items to display, where each sits, and what a row does that does not \
             fit."
                .to_string(),
        ),
        items,
    )
    .on_preview(Box::new(move |items| {
        let (selected, colors) = status_line_selection(&enabled_ids(items));
        status_line(&selected, colors, |item| status_preview(&values, item))
    }))
    .on_confirm(Box::new(move |chosen| {
        let ids = enabled_ids(chosen);
        let (items, colors) = status_line_selection(&ids);
        let positions = chosen
            .iter()
            .filter(|item| item.enabled)
            .filter_map(|item| {
                let place = AgentStatusLinePlace::parse(item.note.as_deref()?)?;
                Some((item.id.clone(), place))
            })
            .collect();
        tx.send(AppEvent::Appearance(AppearanceEvent::StatusLine {
            items: Some(items),
            colors: Some(colors),
            positions: Some(positions),
            overflow: Some(confirmed.get()),
            save: true,
        }));
    }))
    .on_extra_key(Box::new(move |items, selected, key| {
        use ratatui::crossterm::event::KeyCode;
        match key.code {
            // `p` steps the highlighted item's place: left-1, left-2,
            // right-1, right-2.
            KeyCode::Char('p') => {
                let Some(item) = selected.and_then(|index| items.get_mut(index)) else {
                    return false;
                };
                let place = item
                    .note
                    .as_deref()
                    .and_then(AgentStatusLinePlace::parse)
                    .unwrap_or_default();
                let next = match (place.right, place.second_row) {
                    (false, false) => AgentStatusLinePlace {
                        second_row: true,
                        right: false,
                    },
                    (false, true) => AgentStatusLinePlace {
                        second_row: false,
                        right: true,
                    },
                    (true, false) => AgentStatusLinePlace {
                        second_row: true,
                        right: true,
                    },
                    (true, true) => AgentStatusLinePlace::default(),
                };
                item.note = Some(next.value());
                true
            }
            // `o` cycles what a row does when it does not fit.
            KeyCode::Char('o') => {
                keys.set(match keys.get() {
                    AgentStatusLineOverflow::WrapAll => AgentStatusLineOverflow::WrapLeft,
                    AgentStatusLineOverflow::WrapLeft => AgentStatusLineOverflow::Truncate,
                    AgentStatusLineOverflow::Truncate => AgentStatusLineOverflow::WrapAll,
                });
                true
            }
            _ => false,
        }
    }))
}

pub(crate) fn terminal_title_setup(
    appearance: &Appearance,
    tx: AppEventSender,
) -> MultiSelectPicker {
    let values = appearance.values().clone();
    let change_tx = tx.clone();
    let cancel_tx = tx.clone();
    MultiSelectPicker::new(
        "Configure Terminal Title".to_string(),
        Some("Select which items to display in the terminal title.".to_string()),
        select_items(appearance.terminal_title_items()),
    )
    .on_preview(Box::new(move |items| {
        let (selected, _) = parse_items::<TerminalTitleItem>(&enabled_ids(items));
        let now = Instant::now();
        let title = terminal_title_text(&selected, |item| title_preview(&values, item, now));
        (!title.is_empty()).then(|| Line::from(title))
    }))
    .on_change(Box::new(move |items| {
        let (selected, _) = parse_items(&enabled_ids(items));
        change_tx.send(AppEvent::Appearance(AppearanceEvent::PreviewTerminalTitle(
            Some(selected),
        )));
    }))
    .on_confirm(Box::new(move |ids| {
        let ids = enabled_ids(ids);
        let (items, _) = parse_items(&ids);
        tx.send(AppEvent::Appearance(AppearanceEvent::TerminalTitle {
            items,
            save: true,
        }));
    }))
    .on_cancel(Box::new(move || {
        cancel_tx.send(AppEvent::Appearance(AppearanceEvent::PreviewTerminalTitle(
            None,
        )));
    }))
}

/// The items shown, in their order and turned on, then the others.
fn select_items<T: SurfaceItem>(selected: &[T]) -> Vec<MultiSelectItem> {
    let item = |item: T, enabled| MultiSelectItem {
        id: item.id().to_string(),
        name: item.id().to_string(),
        description: Some(item.description().to_string()),
        enabled,
        note: None,
    };
    selected
        .iter()
        .map(|selected| item(*selected, true))
        .chain(
            T::ALL
                .iter()
                .filter(|known| !selected.contains(known))
                .map(|other| item(*other, false)),
        )
        .collect()
}

fn enabled_ids(items: &[MultiSelectItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.enabled)
        .map(|item| item.id.clone())
        .collect()
}

fn status_line_selection(ids: &[String]) -> (Vec<StatusLineItem>, bool) {
    let (items, _) = parse_items(ids);
    (items, ids.iter().any(|id| id == COLORS_ID))
}

fn status_preview(values: &SurfaceValues, item: StatusLineItem) -> Option<String> {
    values.status_value(item).or_else(|| {
        Some(
            match item {
                StatusLineItem::Model => "gpt-5",
                StatusLineItem::ModelWithReasoning => "gpt-5 high",
                StatusLineItem::BackgroundTasks => "2 agents · 1 terminal",
                StatusLineItem::Reasoning => "high",
                StatusLineItem::CurrentDir => "~/project",
                StatusLineItem::GitBranch => "main",
                StatusLineItem::ContextRemaining => "Context 82% left",
                StatusLineItem::ContextUsed => "Context 18% used",
                StatusLineItem::UsedTokens => "12.3K used",
                StatusLineItem::TotalInputTokens => "10.1K in",
                StatusLineItem::TotalOutputTokens => "2.20K out",
                StatusLineItem::SessionId => "019a7c1e-0000-7000-8000-000000000000",
                StatusLineItem::SessionTitle => "Fix the build",
                StatusLineItem::TaskProgress => "Tasks 2/5",
                StatusLineItem::PermissionMode => "manual",
                StatusLineItem::Goal => "Goal: Ship the release (active)",
                StatusLineItem::Compression => "3 blocks, 42K saved",
                StatusLineItem::Advisor => "Advisor 2/5",
                StatusLineItem::Role => "Role: explorer",
            }
            .to_string(),
        )
    })
}

fn title_preview(values: &SurfaceValues, item: TerminalTitleItem, now: Instant) -> Option<String> {
    values.title_value(item, now).or_else(|| {
        let sample = match item {
            TerminalTitleItem::Activity => "⠋",
            TerminalTitleItem::SessionTitle => "Fix the build",
            TerminalTitleItem::Model => "gpt-5",
            TerminalTitleItem::TaskProgress => "Tasks 2/5",
            TerminalTitleItem::ProjectName => "project",
            TerminalTitleItem::AppName
            | TerminalTitleItem::CurrentDir
            | TerminalTitleItem::RunState => return None,
        };
        Some(sample.to_string())
    })
}
