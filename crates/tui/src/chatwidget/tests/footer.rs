//! What a click on a footer count opens, and what it leaves alone.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::chat;
use crate::app::WorkspaceEvent;
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::appearance::SideItem;
use crate::tasks::{Section, TasksEvent};

/// A click with the left button at `column`, `row`.
fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    }
}

/// Gives `chat`'s footer one count at `rect`.
fn place(chat: &mut crate::chatwidget::ChatWidget, rect: Rect, item: SideItem) {
    chat.set_footer_hits_for_test(vec![(rect, item)]);
}

#[test]
fn a_click_on_the_terminals_opens_the_task_manager_at_its_terminals_section() {
    let (mut chat, events) = chat();
    place(&mut chat, Rect::new(10, 4, 11, 1), SideItem::Terminals);
    assert!(chat.handle_picker_mouse(click(12, 4)));
    let opened = events
        .try_iter()
        .find_map(|event| match event {
            AppEvent::Workspace(WorkspaceEvent::Tasks(event)) => Some(*event),
            _ => None,
        })
        .expect("the click opened the task manager");
    assert!(
        matches!(opened, TasksEvent::Open(Section::Terminals)),
        "{opened:?}"
    );
}

#[test]
fn a_click_on_the_agents_opens_the_task_manager_at_its_agents_section() {
    let (mut chat, events) = chat();
    place(&mut chat, Rect::new(10, 4, 9, 1), SideItem::Agents);
    assert!(chat.handle_picker_mouse(click(11, 4)));
    let opened = events
        .try_iter()
        .find_map(|event| match event {
            AppEvent::Workspace(WorkspaceEvent::Tasks(event)) => Some(*event),
            _ => None,
        })
        .expect("the click opened the task manager");
    assert!(
        matches!(opened, TasksEvent::Open(Section::Agents)),
        "{opened:?}"
    );
}

#[test]
fn a_click_on_the_warnings_asks_for_them_as_slash_warnings_does() {
    let (mut chat, events) = chat();
    place(&mut chat, Rect::new(10, 4, 9, 1), SideItem::Warnings);
    assert!(chat.handle_picker_mouse(click(11, 4)));
    let asked = events.try_iter().any(|event| {
        matches!(
            event,
            AppEvent::Op(Op::Command(AgentCommand::Settings(
                crate::agent_bridge::SettingsCall::Warnings
            )))
        )
    });
    assert!(asked, "the click asked for the warnings");
}

#[test]
fn a_click_beside_the_counts_opens_nothing() {
    let (mut chat, events) = chat();
    place(&mut chat, Rect::new(10, 4, 9, 1), SideItem::Warnings);
    assert!(!chat.handle_picker_mouse(click(1, 4)), "not on a count");
    assert!(
        !events
            .try_iter()
            .any(|event| matches!(event, AppEvent::Op(Op::Command(_)))),
        "nothing was asked"
    );
}
