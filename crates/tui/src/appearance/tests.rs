// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::sync::mpsc::{channel, Receiver};

use hanafubuki_agent::{AgentGoal, AgentSessionId};
use insta::assert_snapshot;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::BottomPaneView;
use crate::render::highlight::{resolve_theme_by_name, set_syntax_theme};

fn preferences(status_line: &[&str], terminal_title: &[&str]) -> AgentTuiPreferences {
    AgentTuiPreferences {
        status_line: status_line.iter().map(|id| id.to_string()).collect(),
        terminal_title: terminal_title.iter().map(|id| id.to_string()).collect(),
        ..AgentTuiPreferences::default()
    }
}

fn goal(objective: &str) -> AgentGoal {
    serde_json::from_value(serde_json::json!({
        "objective": objective,
        "status": "active",
    }))
    .unwrap()
}

/// A session with every value known.
fn status() -> SessionStatus {
    SessionStatus {
        session_id: AgentSessionId::new(7),
        title: Some("Fix the build".to_string()),
        model: "gpt-5".to_string(),
        reasoning_effort: Some("high".to_string()),
        usage: Some(UsageSummary {
            input_tokens: 12_000,
            output_tokens: 3_400,
            context: Some((25_000, 100_000)),
        }),
        task_progress: Some((2, 5)),
        permission_mode: Some(AgentPermissionMode::AutoReview),
        goal: Some(goal(
            "Ship the release with every check passing on all platforms",
        )),
        git_branch: Some("main".to_string()),
    }
}

fn appearance(status_line: &[&str], terminal_title: &[&str]) -> Appearance {
    let (mut appearance, warnings) = Appearance::new(
        &preferences(status_line, terminal_title),
        "/work/project".to_string(),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    appearance.values_mut().model = "gpt-5".to_string();
    appearance
}

fn plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Each span as `text` with its foreground and dimming.
fn styled(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| {
            let dim = if span
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::DIM)
            {
                "~"
            } else {
                ""
            };
            match span.style.fg {
                Some(color) => format!("[{}]{{{color}{dim}}}", span.content),
                None => format!("[{}]{dim}", span.content),
            }
        })
        .collect()
}

fn events() -> (AppEventSender, Receiver<AppEvent>) {
    let (sender, receiver) = channel();
    (AppEventSender::new(sender), receiver)
}

fn appearance_events(receiver: &Receiver<AppEvent>) -> Vec<AppearanceEvent> {
    receiver
        .try_iter()
        .filter_map(|event| match event {
            AppEvent::Appearance(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn press(view: &mut impl BottomPaneView, code: KeyCode) {
    view.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn every_status_line_item_shows_the_sessions_value_snapshot() {
    let all: Vec<&str> = StatusLineItem::ALL.iter().map(|item| item.id()).collect();
    let mut appearance = appearance(&all, &[]);
    appearance.values_mut().status = Some(status());
    let line = appearance.status_line().unwrap();
    assert_snapshot!(plain(&line).replace(" · ", "\n"));
}

#[test]
fn the_status_line_takes_the_themes_colours_or_is_dimmed() {
    set_syntax_theme(resolve_theme_by_name("base16-ocean-dark", None).unwrap());
    let mut appearance = appearance(&["model-with-reasoning", "git-branch", "context-used"], &[]);
    appearance.values_mut().status = Some(status());
    assert_snapshot!(styled(&appearance.status_line().unwrap()));
    appearance.apply(AppearanceEvent::StatusLine {
        items: None,
        colors: Some(false),
        save: false,
    });
    assert_eq!(
        styled(&appearance.status_line().unwrap()),
        "[gpt-5 high]~[ · ]~[main]~[ · ]~[Context 25% used]~"
    );
}

#[test]
fn items_with_no_value_yet_are_left_out() {
    let appearance = appearance(
        &[
            "session-title",
            "model",
            "git-branch",
            "current-dir",
            "goal",
        ],
        &[],
    );
    assert_eq!(
        plain(&appearance.status_line().unwrap()),
        "gpt-5 · /work/project"
    );
    let empty = Appearance::new(&preferences(&[], &[]), String::new()).0;
    assert_eq!(empty.status_line(), None);
}

#[test]
fn unknown_and_repeated_ids_are_left_out_with_a_warning() {
    let (appearance, warnings) = Appearance::new(
        &preferences(&["model", "no-such", "model"], &["activity", "rate-limit"]),
        "/work".to_string(),
    );
    assert_eq!(appearance.status_line_items(), [StatusLineItem::Model]);
    assert_eq!(
        appearance.terminal_title_items(),
        [TerminalTitleItem::Activity]
    );
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("tui.statusLine") && warnings[0].contains("no-such"));
    assert!(warnings[1].contains("tui.terminalTitle") && warnings[1].contains("rate-limit"));
}

#[test]
fn the_title_joins_its_items_with_the_spinner_spaced() {
    let mut appearance = appearance(
        &[],
        &[
            "activity",
            "session-title",
            "project-name",
            "run-state",
            "model",
        ],
    );
    appearance.values_mut().status = Some(status());
    let now = Instant::now();
    assert_eq!(
        appearance.terminal_title(now),
        "Fix the build | project | Ready | gpt-5"
    );
    assert!(!appearance.title_is_animated());
    appearance.values_mut().task_running = true;
    let title = appearance.terminal_title(now);
    let (spinner, rest) = title.split_once(' ').unwrap();
    assert!(SPINNER_FRAMES.contains(&spinner), "{title}");
    assert_eq!(rest, "Fix the build | project | Working | gpt-5");
    assert!(appearance.title_is_animated());
}

#[test]
fn a_previewed_title_shows_until_it_is_put_back_or_saved() {
    let mut appearance = appearance(&[], &["app-name"]);
    let now = Instant::now();
    appearance.apply(AppearanceEvent::PreviewTerminalTitle(Some(vec![
        TerminalTitleItem::Model,
    ])));
    assert_eq!(appearance.terminal_title(now), "gpt-5");
    appearance.apply(AppearanceEvent::PreviewTerminalTitle(None));
    assert_eq!(appearance.terminal_title(now), "hanafubuki-agent");
    appearance.apply(AppearanceEvent::TerminalTitle {
        items: Vec::new(),
        save: false,
    });
    assert_eq!(appearance.terminal_title(now), "");
}

#[test]
fn the_status_line_setup_saves_the_items_in_their_order_and_the_colours() {
    let (tx, receiver) = events();
    let appearance = appearance(&["model", "current-dir"], &[]);
    let mut view = setup::status_line_setup(&appearance, tx);
    // The items shown come first: `model` is turned off; `git-branch`, the
    // fifth, is turned on and moved up above `current-dir`.
    press(&mut view, KeyCode::Char(' '));
    for _ in 0..4 {
        press(&mut view, KeyCode::Down);
    }
    press(&mut view, KeyCode::Char(' '));
    for _ in 0..3 {
        press(&mut view, KeyCode::Left);
    }
    press(&mut view, KeyCode::Enter);
    assert_eq!(
        appearance_events(&receiver),
        [AppearanceEvent::StatusLine {
            items: Some(vec![StatusLineItem::GitBranch, StatusLineItem::CurrentDir]),
            colors: Some(true),
            save: true,
        }]
    );
}

#[test]
fn the_title_setup_previews_in_the_terminal_and_esc_puts_it_back() {
    let (tx, receiver) = events();
    let appearance = appearance(&[], &["app-name"]);
    let mut view = setup::terminal_title_setup(&appearance, tx);
    press(&mut view, KeyCode::Down);
    press(&mut view, KeyCode::Char(' '));
    press(&mut view, KeyCode::Esc);
    assert_eq!(
        appearance_events(&receiver),
        [
            AppearanceEvent::PreviewTerminalTitle(Some(vec![
                TerminalTitleItem::AppName,
                TerminalTitleItem::ProjectName,
            ])),
            AppearanceEvent::PreviewTerminalTitle(None),
        ]
    );
}

#[test]
fn the_title_setup_saves_what_is_confirmed() {
    let (tx, receiver) = events();
    let appearance = appearance(&[], &["app-name"]);
    let mut view = setup::terminal_title_setup(&appearance, tx);
    press(&mut view, KeyCode::Char(' '));
    press(&mut view, KeyCode::Enter);
    assert_eq!(
        appearance_events(&receiver).last(),
        Some(&AppearanceEvent::TerminalTitle {
            items: Vec::new(),
            save: true,
        })
    );
}
