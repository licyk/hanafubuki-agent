// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/keymap [debug]` (Codex's `keymap_setup.rs` and `keymap_setup/`): pick
//! an action, then replace its keys, add one, remove one, unbind it or put
//! its defaults back; a new key or chord is captured as it is pressed. A
//! change that would conflict with another action's keys, or with a key
//! the interface keeps, is refused with the reason; the others are saved
//! into `tui.keymap.<context>.<action>` and apply at once. `/keymap debug`
//! lists every action's keys as they resolved.

mod capture;

use std::collections::BTreeMap;

use hanafubuki_agent::AgentTuiPreferencesPatch;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{send_step, CommandContext, CommandSpec};
use crate::app_event::{AppCommand, AppEvent, AppEventSender};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell::{self, PlainHistoryCell};
use crate::keymap::{self, conflicts, Action, Binding, ACTIONS};
use capture::{Capture, KeymapCapture};

pub(super) static KEYMAP: CommandSpec = CommandSpec {
    name: "keymap",
    aliases: &[],
    description: "change the keys of the interface's actions",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    match args.trim() {
        "" => open_picker(context),
        "debug" => context.add_to_history(PlainHistoryCell::new(debug_lines())),
        _ => context.add_to_history(history_cell::new_error_event(
            "Usage: /keymap [debug]".to_string(),
        )),
    }
}

/// Every action, to pick one.
fn open_picker(context: &mut CommandContext<'_>) {
    let keymap = keymap::current();
    let items = ACTIONS
        .iter()
        .map(|spec| {
            let action = spec.action;
            let path = action.path();
            SelectionItem {
                name: path.clone(),
                description: Some(format!("{} — {}", keymap.label(action), spec.description)),
                search_value: Some(format!("{path} {}", spec.description)),
                actions: vec![Box::new(move |tx: &AppEventSender| {
                    send_step(tx, &KEYMAP, move |context| open_menu(context, action));
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Keymap".to_string()),
        subtitle: Some("Pick an action to change its keys.".to_string()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search actions".to_string()),
        ..Default::default()
    });
}

/// What can be done to `action`'s keys.
fn open_menu(context: &mut CommandContext<'_>, action: Action) {
    let keymap = keymap::current();
    let bindings = keymap.bindings(action).to_vec();
    let mut items = Vec::new();
    let captures: &[(Capture, &str)] = if action == Action::Leader {
        &[(Capture::Key, "a key")]
    } else {
        &[
            (Capture::Key, "a key"),
            (Capture::Chord, "two keys (a chord)"),
            (Capture::Leader, "the leader and a key"),
        ]
    };
    for (add, verb) in [(false, "Replace with"), (true, "Add")] {
        for (capture, what) in captures {
            let capture = *capture;
            items.push(SelectionItem {
                name: format!("{verb} {what}"),
                actions: vec![Box::new(move |tx: &AppEventSender| {
                    send_step(tx, &KEYMAP, move |context| {
                        let view = KeymapCapture::new(action, capture, add, context.app_event_tx());
                        context.show_view(Box::new(view));
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
    }
    for binding in &bindings {
        let kept: Vec<Binding> = bindings.iter().copied().filter(|b| b != binding).collect();
        items.push(change_item(
            format!("Remove `{}`", binding.spec()),
            action,
            Some(kept),
        ));
    }
    if !bindings.is_empty() {
        items.push(change_item(
            "Unbind it".to_string(),
            action,
            Some(Vec::new()),
        ));
    }
    if keymap.is_configured(action) {
        items.push(change_item(
            "Put its default keys back".to_string(),
            action,
            None,
        ));
    }
    let spec = action.spec();
    context.show_selection_view(SelectionViewParams {
        title: Some(action.path()),
        subtitle: Some(format!(
            "{} — now: {}",
            spec.description,
            keymap.label(action)
        )),
        items,
        ..Default::default()
    });
}

/// A menu item that saves `bindings` (`None`: the defaults) for `action`.
fn change_item(name: String, action: Action, bindings: Option<Vec<Binding>>) -> SelectionItem {
    SelectionItem {
        name,
        actions: vec![Box::new(move |tx: &AppEventSender| {
            if let Err(error) = save(tx, action, bindings.clone()) {
                let cell = history_cell::new_error_event(format!("/keymap: {error}"));
                tx.send(AppEvent::InsertHistoryCell(Box::new(cell)));
            }
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}

/// Saves `bindings` (`None`: the defaults) as `action`'s keys, unless they
/// conflict with another action's or a key the interface keeps.
pub(super) fn save(
    tx: &AppEventSender,
    action: Action,
    bindings: Option<Vec<Binding>>,
) -> Result<(), String> {
    let keymap = keymap::current();
    let resulting = bindings
        .clone()
        .unwrap_or_else(|| keymap::default_bindings(action));
    let candidate = keymap.with(action, resulting.clone());
    if let Some(conflict) = conflicts(&candidate)
        .into_iter()
        .find(|conflict| conflict.first == Some(action) || conflict.second == Some(action))
    {
        return Err(format!("{}; nothing was changed", conflict.message));
    }
    let spec = action.spec();
    let keys = bindings.map(|bindings| bindings.iter().map(Binding::spec).collect());
    let patch = AgentTuiPreferencesPatch {
        keymap: BTreeMap::from([(
            spec.context.name().to_string(),
            BTreeMap::from([(spec.name.to_string(), keys)]),
        )]),
        ..AgentTuiPreferencesPatch::default()
    };
    let label = if resulting.is_empty() {
        "unbound".to_string()
    } else {
        resulting
            .iter()
            .map(Binding::spec)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let message = format!("{}: {label}.", action.path());
    tx.send(AppEvent::Command(AppCommand::SavePreferences(
        patch, message,
    )));
    Ok(())
}

/// `/keymap debug`: every action's keys as they resolved, the configured
/// ones marked, and the keys the interface keeps.
fn debug_lines() -> Vec<Line<'static>> {
    let keymap = keymap::current();
    let mut lines = vec![Line::from("/keymap debug".magenta())];
    for spec in &ACTIONS {
        let origin = if keymap.is_configured(spec.action) {
            "configured"
        } else {
            "default"
        };
        lines.push(Line::from(vec![
            format!("  {:<34}", spec.action.path()).into(),
            keymap.label(spec.action).cyan(),
            format!("  ({origin})").dim(),
        ]));
    }
    lines.push(Line::from(
        "  Kept by the interface: ctrl-c (interrupt, clear, quit; close a pager), ctrl-d (quit \
         from an empty composer), esc in views and popups."
            .dim(),
    ));
    lines
}
