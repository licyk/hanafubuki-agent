//! `/subagents`' status cell and picker, `/side`'s and `/agents`' dispatch,
//! and the commands a second conversation runs.

use hanafubuki_agent::{AgentSessionId, AgentSubagentState, AgentSubagentStatus};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::agent_bridge::TurnUpdate;
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::commands::tests::session::{chat, key, run, screen, sent, Sent};
use crate::history_cell::HistoryCell;

fn status(
    id: u64,
    title: &str,
    state: AgentSubagentState,
    answer: Option<&str>,
) -> AgentSubagentStatus {
    AgentSubagentStatus {
        session_id: AgentSessionId::new(id),
        title: title.to_string(),
        status: state,
        last_answer: answer.map(str::to_string),
    }
}

fn statuses() -> Vec<AgentSubagentStatus> {
    vec![
        status(
            2,
            "counter",
            AgentSubagentState::Idle,
            Some("Twelve files\nin src."),
        ),
        status(3, "writer", AgentSubagentState::Running, None),
    ]
}

fn text(cell: &dyn HistoryCell) -> String {
    cell.display_lines(80)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_status_cell_lists_each_subagent_with_its_latest_answer() {
    insta::assert_snapshot!(text(&status_cell(&statuses())), @r"
    /subagents
    Subagents

      • counter  idle
        Twelve files in src.
      • writer  running
        No answer yet.
    ");
    assert!(text(&status_cell(&[])).contains("No subagents in this conversation."));
}

#[test]
fn subagents_lists_them_and_enter_shows_the_one_selected() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "subagents"),
        [Sent::Op(Op::Command(AgentCommand::Subagents(
            SubagentsCall::List
        )))]
    );
    chat.handle_turn_update(TurnUpdate::Subagents(SubagentsReply::Listed(statuses())));
    let shown = screen(&chat, 80);
    assert!(shown.contains("Subagents"), "{shown}");
    assert!(shown.contains("counter"), "{shown}");
    assert!(shown.contains("running"), "{shown}");

    // The watch's statuses replace the list's.
    let mut changed = statuses();
    changed[1].status = AgentSubagentState::Failed;
    chat.handle_turn_update(TurnUpdate::Subagents(SubagentsReply::Changed(changed)));
    let shown = screen(&chat, 80);
    assert!(shown.contains("failed"), "{shown}");

    sent(&events);
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    let opened = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
        matches!(
            event,
            AppEvent::Workspace(WorkspaceEvent::OpenChild(id)) if id == AgentSessionId::new(3)
        )
    });
    assert!(opened);
}

#[test]
fn side_and_agents_ask_the_app() {
    let (mut chat, events) = chat(false);
    let workspace = |events: &std::sync::mpsc::Receiver<AppEvent>| {
        std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                AppEvent::Workspace(event) => Some(event),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    sent(&events);
    let side = crate::commands::find("btw").unwrap();
    chat.dispatch_command(side, " What is this? ".to_string());
    assert!(matches!(
        workspace(&events).as_slice(),
        [WorkspaceEvent::OpenSide(Some(message))] if message == "What is this?"
    ));
    chat.dispatch_command(side, "close".to_string());
    assert!(matches!(
        workspace(&events).as_slice(),
        [WorkspaceEvent::CloseSide]
    ));
    chat.dispatch_command(crate::commands::find("agents").unwrap(), String::new());
    assert!(matches!(
        workspace(&events).as_slice(),
        [WorkspaceEvent::OpenAgents]
    ));
}

#[test]
fn a_second_conversation_runs_only_its_commands() {
    let (chat, _events) = chat(true);
    let (sender, events) = std::sync::mpsc::channel();
    let mut second = chat.second(AppEventSender::new(sender));
    second.set_composer_text("/model".to_string());
    key(&mut second, KeyCode::Enter);
    let said: Vec<Sent> = sent(&events);
    assert!(
        said.contains(&Sent::Cell(
            "■ '/model' is unavailable here; Esc returns to the main conversation.".to_string()
        )),
        "{said:?}"
    );
    // The popup offers only what runs there.
    second.set_composer_text("/".to_string());
    let popup = screen(&second, 80);
    assert!(popup.contains("/copy"), "{popup}");
    assert!(!popup.contains("/model"), "{popup}");
}
