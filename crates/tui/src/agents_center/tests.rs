// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::fs;

use hanafubuki_agent::{
    AgentAgentsGrouping, AgentSessionId, AgentSessionPage, AgentSessionSummary, PathKey,
};
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use serde_json::json;

use super::grouping::Place;
use super::status::Group;
use super::{AgentsCenter, CenterAction};

/// Minutes, as the rows' times are.
const MINUTE: u128 = 60_000;
const NOW: u128 = 1_000 * MINUTE;

fn summary(id: u64, title: &str, phase: &str, minutes_ago: u128) -> AgentSessionSummary {
    serde_json::from_value(json!({
        "sessionId": id,
        "forkedFromSessionId": null,
        "generation": 1,
        "revision": 1,
        "providerId": "test",
        "providerProtocol": "responses",
        "model": "gpt-test",
        "mode": "default",
        "reasoningEffort": null,
        "title": { "value": title, "source": "user" },
        "lifecycle": "available",
        "usage": null,
        "compaction": null,
        "latestTurnPhase": phase,
        "createdAt": 0,
        "updatedAt": NOW - minutes_ago * MINUTE,
        "recoveryMarker": null,
        "cwd": "/work/app",
    }))
    .unwrap()
}

fn child(id: u64, parent: u64, phase: &str) -> AgentSessionSummary {
    let mut child = summary(id, "child", phase, 0);
    child.parent_session_id = Some(AgentSessionId::new(parent));
    child
}

fn page(sessions: Vec<AgentSessionSummary>) -> Result<AgentSessionPage, String> {
    Ok(AgentSessionPage {
        sessions,
        next_cursor: None,
    })
}

/// A center over `roots`, with `children` under the first and none under
/// the others.
fn center(roots: Vec<AgentSessionSummary>, children: Vec<AgentSessionSummary>) -> AgentsCenter {
    let mut center = AgentsCenter::new(None, AgentAgentsGrouping::Status);
    center.ingest_page(page(roots));
    for (parent, _) in center.children_wanted() {
        let mine = children
            .iter()
            .filter(|child| child.parent_session_id == Some(parent))
            .cloned()
            .collect();
        center.ingest_children(parent, page(mine));
    }
    center
}

fn key(center: &mut AgentsCenter, code: KeyCode) -> Option<CenterAction> {
    center.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn typed(center: &mut AgentsCenter, text: &str) {
    for character in text.chars() {
        key(center, KeyCode::Char(character));
    }
}

fn titles(center: &AgentsCenter) -> Vec<String> {
    center
        .rows()
        .into_iter()
        .map(|row| row.summary.title.unwrap().value)
        .collect()
}

fn screen(center: &mut AgentsCenter, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    center.render(area, &mut buf, NOW);
    (0..height)
        .map(|y| {
            let row: String = (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_root_takes_the_most_urgent_status_of_itself_and_its_children() {
    let mut center = center(
        vec![
            summary(1, "asks", "completed", 1),
            summary(2, "works", "completed", 2),
            summary(3, "child failed", "completed", 3),
            summary(4, "failed", "failed", 4),
            summary(5, "interrupted", "interrupted", 5),
        ],
        vec![
            child(11, 1, "running_tools"),
            child(12, 2, "sampling"),
            child(13, 3, "failed"),
        ],
    );
    // Only the running sessions are asked about an approval.
    let wanted = center.approvals_wanted();
    assert_eq!(wanted.len(), 2, "{wanted:?}");
    center.ingest_approval(AgentSessionId::new(11), true);
    center.ingest_approval(AgentSessionId::new(12), false);

    let statuses: Vec<(String, &str)> = center
        .rows()
        .into_iter()
        .map(|row| (row.summary.title.unwrap().value, row.status.label()))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("asks".to_string(), "Needs input"),
            ("failed".to_string(), "Error"),
            ("works".to_string(), "Working"),
            // A child's failure asks nothing of you.
            ("child failed".to_string(), "Ready"),
            ("interrupted".to_string(), "Inactive"),
        ]
    );
}

#[test]
fn tab_and_the_digits_filter_by_status_and_archived_reads_the_archive() {
    let mut center = center(
        vec![
            summary(1, "ready", "completed", 1),
            summary(2, "working", "sampling", 2),
        ],
        Vec::new(),
    );
    key(&mut center, KeyCode::Tab);
    assert!(titles(&center).is_empty(), "Needs you");
    key(&mut center, KeyCode::Tab);
    assert_eq!(titles(&center), ["working"]);
    key(&mut center, KeyCode::BackTab);
    key(&mut center, KeyCode::BackTab);
    assert_eq!(titles(&center).len(), 2, "All");
    key(&mut center, KeyCode::Char('4'));
    assert_eq!(titles(&center), ["ready"]);
    // Left and Right no longer change it: Right opens.
    let opened = key(&mut center, KeyCode::Right);
    assert!(matches!(opened, Some(CenterAction::Open(session)) if session.session_id.value() == 1));
    assert!(key(&mut center, KeyCode::Left).is_none());
    assert_eq!(titles(&center), ["ready"]);

    key(&mut center, KeyCode::Char('6'));
    let request = center.page_request().unwrap();
    assert_eq!(
        request.lifecycle,
        hanafubuki_agent::AgentSessionLifecycle::Archived
    );
}

#[test]
fn the_selection_wraps_around_and_follows_its_session() {
    let mut center = center(
        vec![
            summary(1, "first", "completed", 1),
            summary(2, "second", "completed", 2),
            summary(3, "third", "completed", 3),
        ],
        Vec::new(),
    );
    let selected = |center: &AgentsCenter| center.selected_row().unwrap().summary.session_id;
    assert_eq!(selected(&center).value(), 1);
    key(&mut center, KeyCode::Up);
    assert_eq!(selected(&center).value(), 3);
    key(&mut center, KeyCode::Down);
    assert_eq!(selected(&center).value(), 1);
    key(&mut center, KeyCode::Down);
    // Another grouping reorders the rows; the selection stays on its session.
    key(&mut center, KeyCode::Char('g'));
    assert_eq!(selected(&center).value(), 2);
}

#[test]
fn g_cycles_the_groupings_for_the_app_to_keep() {
    let mut center = AgentsCenter::new(None, AgentAgentsGrouping::Project);
    let mut seen = Vec::new();
    for _ in 0..4 {
        match key(&mut center, KeyCode::Char('g')) {
            Some(CenterAction::Grouping(grouping)) => seen.push(grouping),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        seen,
        [
            AgentAgentsGrouping::Directory,
            AgentAgentsGrouping::Status,
            AgentAgentsGrouping::Model,
            AgentAgentsGrouping::Project,
        ]
    );
}

#[test]
fn headings_group_by_status_model_and_directory() {
    let mut other = summary(3, "elsewhere", "completed", 3);
    other.cwd = Some("/work/lib".to_string());
    other.model = "gpt-other".to_string();
    let mut nowhere = summary(4, "nowhere", "completed", 4);
    nowhere.cwd = None;
    let mut center = center(
        vec![
            summary(1, "done", "completed", 1),
            summary(2, "busy", "sampling", 2),
            other,
            nowhere,
        ],
        Vec::new(),
    );
    let headings = |center: &AgentsCenter| {
        let mut labels: Vec<String> = center
            .rows()
            .into_iter()
            .map(|row| row.heading.label)
            .collect();
        labels.dedup();
        labels
    };
    assert_eq!(headings(&center), ["Working", "Ready"]);
    key(&mut center, KeyCode::Char('g'));
    assert_eq!(headings(&center), ["gpt-other", "gpt-test"]);
    key(&mut center, KeyCode::Char('g'));
    key(&mut center, KeyCode::Char('g'));
    assert_eq!(
        headings(&center),
        ["/work/app", "/work/lib", "No directory"]
    );
}

#[test]
fn a_worktree_falls_under_its_main_checkout_in_the_project_grouping() {
    let home = tempfile::tempdir().unwrap();
    let main = home.path().join("repo");
    let git_dir = main.join(".git").join("worktrees").join("wt");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("commondir"), "../..\n").unwrap();
    let worktree = home.path().join("worktrees").join("wt");
    fs::create_dir_all(worktree.join("src")).unwrap();
    fs::create_dir_all(main.join("src")).unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", git_dir.display()),
    )
    .unwrap();

    let in_worktree = Place::of(&worktree.join("src").display().to_string());
    let in_main = Place::of(&main.join("src").display().to_string());
    let elsewhere = Place::of(&home.path().join("other").display().to_string());
    assert_eq!(in_worktree.project_key(), in_main.project_key());
    assert_ne!(in_worktree.directory_key(), in_main.directory_key());
    assert_ne!(elsewhere.project_key(), in_main.project_key());
    assert_eq!(
        in_main.project_key(),
        &PathKey::of(&main.join("src")),
        "a main checkout is its own project"
    );
}

#[test]
fn a_search_reads_the_roots_by_title_and_never_the_children() {
    let mut center = center(vec![summary(1, "fix login", "completed", 1)], Vec::new());
    assert!(center.page_request().is_none(), "the list is read");
    key(&mut center, KeyCode::Char('/'));
    assert!(center.takes_text());
    typed(&mut center, "log");
    let request = center.page_request().unwrap();
    assert_eq!(request.search.as_deref(), Some("log"));
    center.ingest_page(page(vec![summary(1, "fix login", "completed", 1)]));
    for (_, request) in center.children_wanted() {
        assert_eq!(request.search, None);
    }
    // Typed text is text, the action keys included; Backspace edits it.
    typed(&mut center, "gn");
    key(&mut center, KeyCode::Backspace);
    assert_eq!(
        center.page_request().unwrap().search.as_deref(),
        Some("logg")
    );
    // Esc ends it and reads the whole list again.
    key(&mut center, KeyCode::Esc);
    assert!(!center.takes_text());
    assert_eq!(center.page_request().unwrap().search, None);
}

#[test]
fn rename_edits_the_title_in_place_and_enter_submits_it() {
    let mut center = center(vec![summary(1, "old", "completed", 1)], Vec::new());
    key(&mut center, KeyCode::Char('r'));
    assert!(screen(&mut center, 80, 12).contains("Rename › old"));
    for _ in 0..3 {
        key(&mut center, KeyCode::Backspace);
    }
    typed(&mut center, "new name");
    assert_eq!(
        key(&mut center, KeyCode::Enter),
        Some(CenterAction::Rename {
            session_id: AgentSessionId::new(1),
            title: "new name".to_string(),
        })
    );
    assert!(!center.takes_text());

    key(&mut center, KeyCode::Char('r'));
    key(&mut center, KeyCode::Esc);
    assert!(!center.takes_text(), "Esc cancels the rename");
}

#[test]
fn archive_and_delete_ask_first_and_default_to_cancel() {
    let mut center = center(vec![summary(1, "task", "completed", 1)], Vec::new());
    key(&mut center, KeyCode::Backspace);
    assert!(screen(&mut center, 80, 20).contains("Permanently delete “task”?"));
    assert_eq!(key(&mut center, KeyCode::Enter), None, "Cancel is chosen");
    key(&mut center, KeyCode::Backspace);
    key(&mut center, KeyCode::Down);
    assert_eq!(
        key(&mut center, KeyCode::Enter),
        Some(CenterAction::Delete(AgentSessionId::new(1)))
    );

    key(&mut center, KeyCode::Char('a'));
    assert!(screen(&mut center, 80, 20).contains("Archive “task”?"));
    assert_eq!(key(&mut center, KeyCode::Esc), None);
    key(&mut center, KeyCode::Char('a'));
    key(&mut center, KeyCode::Tab);
    assert_eq!(
        key(&mut center, KeyCode::Enter),
        Some(CenterAction::SetArchived {
            session_id: AgentSessionId::new(1),
            archive: true,
        })
    );
}

#[test]
fn the_session_shown_is_not_archived_or_deleted_from_here() {
    let mut center = AgentsCenter::new(Some(AgentSessionId::new(1)), AgentAgentsGrouping::Status);
    center.ingest_page(page(vec![summary(1, "shown", "completed", 1)]));
    key(&mut center, KeyCode::Backspace);
    assert!(screen(&mut center, 80, 12).contains("Use /delete for the session shown."));
    key(&mut center, KeyCode::Char('a'));
    assert!(screen(&mut center, 80, 12).contains("Use /archive for the session shown."));
}

#[test]
fn x_stops_only_a_session_whose_turn_runs() {
    let mut center = center(
        vec![
            summary(1, "running", "sampling", 1),
            summary(2, "idle", "completed", 2),
        ],
        Vec::new(),
    );
    assert_eq!(
        key(&mut center, KeyCode::Char('x')),
        Some(CenterAction::Stop(AgentSessionId::new(1)))
    );
    key(&mut center, KeyCode::Down);
    assert_eq!(key(&mut center, KeyCode::Char('x')), None);
    assert!(screen(&mut center, 80, 12).contains("Nothing runs there."));
}

#[test]
fn the_new_keys_are_the_keymaps() {
    let mut center = center(vec![summary(1, "task", "completed", 1)], Vec::new());
    assert_eq!(
        key(&mut center, KeyCode::Char('n')),
        Some(CenterAction::New)
    );
    assert_eq!(
        key(&mut center, KeyCode::Char('w')),
        Some(CenterAction::NewWorktree)
    );
    assert_eq!(
        key(&mut center, KeyCode::Char('f')),
        Some(CenterAction::Fork(AgentSessionId::new(1)))
    );
    assert_eq!(key(&mut center, KeyCode::Esc), Some(CenterAction::Close));
}

#[test]
fn the_center_with_its_headings_and_columns() {
    let mut working = summary(2, "Port the command center", "running_tools", 3);
    working.model = "gpt-other".to_string();
    let mut center = center(
        vec![
            summary(1, "Fix the login form", "completed", 42),
            working,
            summary(3, "Explain the codebase", "interrupted", 3 * 60),
        ],
        vec![child(11, 1, "waiting_for_user_input")],
    );
    assert_eq!(center.rows()[0].status.group, Group::NeedsYou);
    assert_snapshot!("center_wide", screen(&mut center, 100, 16));
    assert_snapshot!("center_narrow", screen(&mut center, 50, 16));
    key(&mut center, KeyCode::Char('?'));
    assert_snapshot!("center_help", screen(&mut center, 100, 24));
    key(&mut center, KeyCode::Esc);
    key(&mut center, KeyCode::Backspace);
    assert_snapshot!("center_confirm", screen(&mut center, 100, 16));
}
