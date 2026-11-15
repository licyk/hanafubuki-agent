// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The Advanced maintenance page dispatches operations to the standalone CLI.
use super::CommandSpec;
use crate::app_event::AppEvent;
use crate::bottom_pane::{ListSelectionView, SelectionItem, SelectionViewParams};
use crate::maintenance::TuiMaintenanceAction;
use crate::settings_center::{Attach, PageOverride};
use hanafubuki_agent::SettingsGroup;

pub(super) static MAINTENANCE: CommandSpec = CommandSpec {
    name: "maintenance",
    aliases: &[],
    description: "version, updates and diagnostics",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _| super::open_settings(context, PAGE.id),
};

pub(super) static PAGE: PageOverride = PageOverride {
    id: "maintenance",
    attach: Attach::Row {
        group: SettingsGroup::Advanced,
        label: "Updates and diagnostics",
        summary: "Current version, check for updates, install update, doctor",
    },
    command: &MAINTENANCE,
    open: |context, _| {
        let items = [
            ("Check for updates", TuiMaintenanceAction::CheckUpdates),
            ("Install latest version", TuiMaintenanceAction::Update),
            ("Run diagnostics (doctor)", TuiMaintenanceAction::Doctor),
        ]
        .into_iter()
        .map(|(label, action)| SelectionItem {
            name: label.into(),
            dismiss_on_select: true,
            actions: vec![Box::new(move |tx| tx.send(AppEvent::Maintenance(action)))],
            ..Default::default()
        })
        .collect();
        context.show_view(Box::new(ListSelectionView::new(
            SelectionViewParams {
                title: Some(format!("Maintenance · v{}", env!("CARGO_PKG_VERSION"))),
                subtitle: Some("Runs in the terminal; press Enter afterwards to return".into()),
                items,
                ..Default::default()
            },
            context.app_event_tx(),
        )));
    },
};
