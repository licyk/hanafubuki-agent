use hanafubuki_agent::{
    AgentBackgroundWork, AgentSubagentState, AgentSubagentStatus, AgentWakePause, AgentWakeState,
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
};

use super::*;

fn fixture() -> Tasks {
    let root = AgentSessionId::new(1);
    let mut tasks = Tasks::new(root);
    let child = |id, parent, depth, title: &str, status| AgentSubagentStatus {
        session_id: AgentSessionId::new(id),
        parent_session_id: AgentSessionId::new(parent),
        depth,
        title: title.into(),
        status,
        provider_id: "fixture".into(),
        model: "test-model".into(),
        reasoning_effort: None,
        last_answer: Some("The first pass is complete.".into()),
        state_since: Some(10_000),
    };
    tasks.ingest(Ok(TaskView {
        tasks: AgentBackgroundTasks {
            agents: vec![child(2,1,1,"explore_api",AgentSubagentState::Waiting),child(3,2,2,"check_tests",AgentSubagentState::Running)],
            terminals: vec![AgentBackgroundTerminal {
                owner: AgentSessionId::new(3), owner_title: Some("check_tests".into()),
                terminal: serde_json::from_value(serde_json::json!({
                    "executionId":12,"generation":1,"command":"cargo test --workspace","cwd":"/work/repo",
                    "status":"running","exitCode":null,"createdAtMs":20_000,"updatedAtMs":21_000,
                    "cleanupRequired":false,"recentOutput":["Compiling example","Running tests","test first ... ok","test second ... ok"]
                })).unwrap(),
            }], pending_wake_events: 2, wake: AgentWakeState::On,
            work: AgentBackgroundWork { agents: 2, terminals: 1 },
        },
        details: [(AgentSessionId::new(2),AgentDetails{task:"Inspect the API and delegate its tests.".into(),role:"explorer".into(),work:Some(AgentBackgroundWork{agents:1,terminals:1})})].into(),
    }));
    tasks
}

fn press(tasks: &mut Tasks, key: KeyCode) -> Option<TaskAction> {
    tasks.key(KeyEvent::new(key, KeyModifiers::NONE))
}

fn screen(tasks: &Tasks) -> String {
    let area = Rect::new(0, 0, 120, 24);
    let mut buffer = Buffer::empty(area);
    tasks.render(area, &mut buffer, 202_000);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn tree_details_and_paused_footer() {
    let mut tasks = fixture();
    insta::assert_snapshot!("tasks_tree", screen(&tasks));
    tasks.view.as_mut().unwrap().tasks.wake = AgentWakeState::Paused {
        reason: AgentWakePause::PersonInterrupted,
    };
    insta::assert_snapshot!("tasks_paused", screen(&tasks));
    assert!(matches!(
        press(&mut tasks, KeyCode::Char('r')),
        Some(TaskAction::Resume)
    ));
    assert!(
        matches!(press(&mut tasks,KeyCode::Enter),Some(TaskAction::Open(id)) if id==AgentSessionId::new(2))
    );
}

#[test]
fn agent_close_requires_confirmation_and_shows_descendants() {
    let mut tasks = fixture();
    assert!(press(&mut tasks, KeyCode::Char('x')).is_none());
    insta::assert_snapshot!("tasks_close_agent", screen(&tasks));
    assert!(press(&mut tasks, KeyCode::Enter).is_none());
    assert!(tasks.confirmation.is_none());
    press(&mut tasks, KeyCode::Char('x'));
    press(&mut tasks, KeyCode::Tab);
    assert!(
        matches!(press(&mut tasks,KeyCode::Enter),Some(TaskAction::CloseAgent(id)) if id==AgentSessionId::new(2))
    );
}

#[test]
fn terminals_expand_close_and_stop_all() {
    let mut tasks = fixture();
    press(&mut tasks, KeyCode::Tab);
    press(&mut tasks, KeyCode::Enter);
    insta::assert_snapshot!("tasks_terminal", screen(&tasks));
    press(&mut tasks, KeyCode::Char('x'));
    insta::assert_snapshot!("tasks_close_terminal", screen(&tasks));
    assert!(
        matches!(press(&mut tasks,KeyCode::Char('y')),Some(TaskAction::CloseTerminal(t)) if t.owner==AgentSessionId::new(3))
    );
    press(&mut tasks, KeyCode::Char('X'));
    assert!(matches!(
        press(&mut tasks, KeyCode::Char('y')),
        Some(TaskAction::CloseTerminals)
    ));
    assert!(matches!(
        press(&mut tasks, KeyCode::Esc),
        Some(TaskAction::Leave)
    ));
}

#[test]
fn terminal_selection_stays_visible_with_silent_and_mixed_output_rows() {
    for mixed in [false, true] {
        let mut tasks = fixture();
        let terminal = tasks.view.as_ref().unwrap().tasks.terminals[0].clone();
        tasks.view.as_mut().unwrap().tasks.terminals = (0..12)
            .map(|index| {
                let mut terminal = terminal.clone();
                terminal.terminal.command = format!("task-{index}");
                terminal.terminal.recent_output = if mixed {
                    vec!["output".into(); index % 4]
                } else {
                    vec![]
                };
                terminal
            })
            .collect();
        press(&mut tasks, KeyCode::Tab);
        for selected in 0..12 {
            let screen = screen(&tasks);
            assert!(
                screen.contains(&format!("› #12 task-{selected}")),
                "{screen}"
            );
            press(&mut tasks, KeyCode::Down);
        }
    }
}
