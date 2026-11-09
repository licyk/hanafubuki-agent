// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use crate::app_event::AppEvent;
use crate::key_hint;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use std::sync::mpsc::channel as unbounded_channel;

fn make_selection_view(subtitle: Option<&str>) -> ListSelectionView {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let items = vec![
        SelectionItem {
            name: "Read Only".to_string(),
            description: Some("Codex can read files".to_string()),
            is_current: true,
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "Full Access".to_string(),
            description: Some("Codex can edit files".to_string()),
            is_current: false,
            dismiss_on_select: true,
            ..Default::default()
        },
    ];
    ListSelectionView::new(
        SelectionViewParams {
            title: Some("Select Approval Mode".to_string()),
            subtitle: subtitle.map(str::to_string),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        },
        tx,
    )
}

fn render_lines(view: &ListSelectionView) -> String {
    render_lines_with_width(view, 48)
}

fn render_lines_with_width(view: &ListSelectionView, width: u16) -> String {
    let height = view.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);

    let lines: Vec<String> = (0..area.height)
        .map(|row| {
            let mut line = String::new();
            for col in 0..area.width {
                let symbol = buf[(area.x + col, area.y + row)].symbol();
                if symbol.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(symbol);
                }
            }
            line
        })
        .collect();
    lines.join("\n")
}

#[test]
fn renders_blank_line_between_title_and_items_without_subtitle() {
    let view = make_selection_view(None);
    assert_snapshot!(
        "list_selection_spacing_without_subtitle",
        render_lines(&view)
    );
}

#[test]
fn renders_blank_line_between_subtitle_and_items() {
    let view = make_selection_view(Some("Switch between Codex approval presets"));
    assert_snapshot!("list_selection_spacing_with_subtitle", render_lines(&view));
}

#[test]
fn snapshot_footer_note_wraps() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let items = vec![SelectionItem {
        name: "Read Only".to_string(),
        description: Some("Codex can read files".to_string()),
        is_current: true,
        dismiss_on_select: true,
        ..Default::default()
    }];
    let footer_note = Line::from(vec![
        "Note: ".dim(),
        "Use /setup-elevated-sandbox".cyan(),
        " to allow network access.".dim(),
    ]);
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Select Approval Mode".to_string()),
            footer_note: Some(footer_note),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        },
        tx,
    );
    assert_snapshot!(
        "list_selection_footer_note_wraps",
        render_lines_with_width(&view, 40)
    );
}

#[test]
fn wraps_long_option_without_overflowing_columns() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let items = vec![
        SelectionItem {
            name: "Yes, proceed".to_string(),
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "Yes, and don't ask again for commands that start with `python -mpre_commit run --files eslint-plugin/no-mixed-const-enum-exports.js`".to_string(),
            dismiss_on_select: true,
            ..Default::default()
        },
    ];
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Approval".to_string()),
            items,
            ..Default::default()
        },
        tx,
    );

    let rendered = render_lines_with_width(&view, 60);
    let command_line = rendered
        .lines()
        .find(|line| line.contains("python -mpre_commit run"))
        .expect("rendered lines should include wrapped command");
    assert!(
        command_line.starts_with("     `python -mpre_commit run"),
        "wrapped command line should align under the numbered prefix:\n{rendered}"
    );
    assert!(
        rendered.contains("eslint-plugin/no-") && rendered.contains("mixed-const-enum-exports.js"),
        "long command should not be truncated even when wrapped:\n{rendered}"
    );
}

#[test]
fn a_name_too_wide_for_a_description_column_has_its_description_under_it() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let name =
        "Run PowerShell command in C:\\Users\\someone\\AppData\\Local\\Temp\\project: echo denied";
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Denials".to_string()),
            items: vec![SelectionItem {
                name: name.to_string(),
                description: Some("The user did not ask for this command.".to_string()),
                dismiss_on_select: true,
                ..Default::default()
            }],
            is_searchable: true,
            ..Default::default()
        },
        tx,
    );

    let rendered = render_lines_with_width(&view, 60);
    let unwrapped: String = rendered.split_whitespace().collect();
    let whole_name: String = name.split_whitespace().collect();
    assert!(unwrapped.contains(&whole_name), "{rendered}");
    assert!(
        rendered.contains("    The user did not ask for this command."),
        "{rendered}"
    );
}

#[test]
fn width_changes_do_not_hide_rows() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let items = vec![
        SelectionItem {
            name: "gpt-5.1-codex".to_string(),
            description: Some(
                "Optimized for Codex. Balance of reasoning quality and coding ability.".to_string(),
            ),
            is_current: true,
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "gpt-5.1-codex-mini".to_string(),
            description: Some(
                "Optimized for Codex. Cheaper, faster, but less capable.".to_string(),
            ),
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "gpt-4.1-codex".to_string(),
            description: Some(
                "Legacy model. Use when you need compatibility with older automations.".to_string(),
            ),
            dismiss_on_select: true,
            ..Default::default()
        },
    ];
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Select Model and Effort".to_string()),
            items,
            ..Default::default()
        },
        tx,
    );
    let mut missing: Vec<u16> = Vec::new();
    for width in 60..=90 {
        let rendered = render_lines_with_width(&view, width);
        if !rendered.contains("3.") {
            missing.push(width);
        }
    }
    assert!(
        missing.is_empty(),
        "third option missing at widths {missing:?}"
    );
}

#[test]
fn narrow_width_keeps_all_rows_visible() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let desc = "x".repeat(10);
    let items: Vec<SelectionItem> = (1..=3)
        .map(|idx| SelectionItem {
            name: format!("Item {idx}"),
            description: Some(desc.clone()),
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Debug".to_string()),
            items,
            ..Default::default()
        },
        tx,
    );
    let rendered = render_lines_with_width(&view, 24);
    assert!(
        rendered.contains("3."),
        "third option missing for width 24:\n{rendered}"
    );
}

#[test]
fn snapshot_model_picker_width_80() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let items = vec![
        SelectionItem {
            name: "gpt-5.1-codex".to_string(),
            description: Some(
                "Optimized for Codex. Balance of reasoning quality and coding ability.".to_string(),
            ),
            is_current: true,
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "gpt-5.1-codex-mini".to_string(),
            description: Some(
                "Optimized for Codex. Cheaper, faster, but less capable.".to_string(),
            ),
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "gpt-4.1-codex".to_string(),
            description: Some(
                "Legacy model. Use when you need compatibility with older automations.".to_string(),
            ),
            dismiss_on_select: true,
            ..Default::default()
        },
    ];
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Select Model and Effort".to_string()),
            items,
            ..Default::default()
        },
        tx,
    );
    assert_snapshot!(
        "list_selection_model_picker_width_80",
        render_lines_with_width(&view, 80)
    );
}

#[test]
fn snapshot_narrow_width_preserves_third_option() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let desc = "x".repeat(10);
    let items: Vec<SelectionItem> = (1..=3)
        .map(|idx| SelectionItem {
            name: format!("Item {idx}"),
            description: Some(desc.clone()),
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
    let view = ListSelectionView::new(
        SelectionViewParams {
            title: Some("Debug".to_string()),
            items,
            ..Default::default()
        },
        tx,
    );
    assert_snapshot!(
        "list_selection_narrow_width_preserves_rows",
        render_lines_with_width(&view, 24)
    );
}

/// Codex's standard popup hint, which the tests' views carry.
fn standard_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        "Press ".into(),
        key_hint::plain(KeyCode::Enter).into(),
        " to confirm or ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " to go back".into(),
    ])
}
