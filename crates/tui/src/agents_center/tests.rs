// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::fs;

use hanafubuki_agent::{
    AgentAgentsGrouping, AgentRevision, AgentSessionId, AgentSessionLifecycle, AgentSessionOrigin,
    AgentSessionSummary, PathKey,
};
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use serde_json::json;

use super::grouping::Place;
use super::status::Group;
use super::{AgentsCenter, CenterAction};
use crate::agent_bridge::{CenterJob, CenterRead, Exchange, LiveSession, RootsRead, Waiting};
use crate::tui_state::AgentsMarks;

/// Minutes, as the rows' times are.
const MINUTE: u128 = 60_000;
const NOW: u128 = 1_000 * MINUTE;

#[test]
fn queued_updates_are_visible_without_a_running_turn() {
    let mut center = center(
        vec![summary(1, "Waiting for updates", "completed", 0)],
        vec![],
    );
    let mut session = live(1, None, 1);
    session.pending_wake_events = 3;
    center.ingest(CenterRead::Live(vec![session]));
    assert!(screen(&mut center, 120, 24).contains("3 updates"));
}

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
    let mut child = summary(id, &format!("child {id}"), phase, 0);
    child.parent_session_id = Some(AgentSessionId::new(parent));
    child.origin = Some(AgentSessionOrigin::Subagent);
    child
}

fn live(id: u64, parent: Option<u64>, revision: u64) -> LiveSession {
    LiveSession {
        pending_wake_events: 0,
        session_id: AgentSessionId::new(id),
        parent: parent.map(AgentSessionId::new),
        revision: AgentRevision::new(revision),
        waiting: None,
        activity: None,
    }
}

/// Answers every read the center asks for: the list with `roots`, each
/// one's children from `children`, an empty exchange.
fn answer(
    center: &mut AgentsCenter,
    roots: &[AgentSessionSummary],
    children: &[AgentSessionSummary],
) {
    loop {
        let jobs = center.jobs();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            center.ingest(read_for(job, roots, children));
        }
    }
}

fn read_for(
    job: CenterJob,
    roots: &[AgentSessionSummary],
    children: &[AgentSessionSummary],
) -> CenterRead {
    match job {
        CenterJob::Roots { epoch, .. } => CenterRead::Roots {
            epoch,
            read: Ok(RootsRead {
                sessions: roots.to_vec(),
                more: false,
            }),
        },
        CenterJob::Children { epoch, parent, .. } => CenterRead::Children {
            epoch,
            parent,
            read: Ok(children
                .iter()
                .filter(|child| child.parent_session_id == Some(parent))
                .cloned()
                .collect()),
        },
        CenterJob::Exchange(session_id) => CenterRead::Exchange {
            session_id,
            exchange: Exchange::default(),
        },
    }
}

/// A center over `roots` and their `children`, every read answered.
fn center(roots: Vec<AgentSessionSummary>, children: Vec<AgentSessionSummary>) -> AgentsCenter {
    marked(roots, children, AgentsMarks::default())
}

fn marked(
    roots: Vec<AgentSessionSummary>,
    children: Vec<AgentSessionSummary>,
    marks: AgentsMarks,
) -> AgentsCenter {
    let mut center = AgentsCenter::new(None, AgentAgentsGrouping::Status, marks);
    answer(&mut center, &roots, &children);
    center
}

/// The list request of the reads the center asks for next.
fn roots_request(center: &mut AgentsCenter) -> Option<hanafubuki_agent::AgentSessionListRequest> {
    center.jobs().into_iter().find_map(|job| match job {
        CenterJob::Roots { request, .. } => Some(request),
        _ => None,
    })
}

fn key(center: &mut AgentsCenter, code: KeyCode) -> Option<CenterAction> {
    center.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn click(center: &mut AgentsCenter, column: u16, row: u16) -> Option<CenterAction> {
    center.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
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

/// Where `text` is on the screen, as (column, row).
fn find(screen: &str, text: &str) -> (u16, u16) {
    screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let column = line.find(text)?;
            let column = line[..column].chars().count();
            Some((u16::try_from(column).unwrap(), u16::try_from(row).unwrap()))
        })
        .unwrap_or_else(|| panic!("{text:?} is not on the screen:\n{screen}"))
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
    // The watch says which waits on an approval.
    let mut asking = live(11, Some(1), 1);
    asking.waiting = Some(Waiting::Approval("rm -rf target".to_string()));
    center.ingest(CenterRead::Live(vec![asking, live(12, Some(2), 1)]));

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
    let request = roots_request(&mut center).unwrap();
    assert_eq!(request.lifecycle, AgentSessionLifecycle::Archived);
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
    let mut center = AgentsCenter::new(None, AgentAgentsGrouping::Project, AgentsMarks::default());
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
    let roots = vec![summary(1, "fix login", "completed", 1)];
    let mut center = center(roots.clone(), Vec::new());
    assert!(center.jobs().is_empty(), "the list is read");
    key(&mut center, KeyCode::Char('/'));
    assert!(center.takes_text());
    typed(&mut center, "log");
    let jobs = center.jobs();
    let CenterJob::Roots { request, .. } = &jobs[0] else {
        panic!("{jobs:?}");
    };
    assert_eq!(request.search.as_deref(), Some("log"));
    for job in jobs {
        center.ingest(read_for(job, &roots, &[]));
    }
    for job in center.jobs() {
        if let CenterJob::Children { request, .. } = job {
            assert_eq!(request.search, None);
        }
    }
    // Typed text is text, the action keys included; Backspace edits it.
    typed(&mut center, "gn");
    key(&mut center, KeyCode::Backspace);
    assert_eq!(
        roots_request(&mut center).unwrap().search.as_deref(),
        Some("logg")
    );
    // Esc ends it and reads the whole list again.
    key(&mut center, KeyCode::Esc);
    assert!(!center.takes_text());
    assert_eq!(roots_request(&mut center).unwrap().search, None);
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
    let mut center = AgentsCenter::new(
        Some(AgentSessionId::new(1)),
        AgentAgentsGrouping::Status,
        AgentsMarks::default(),
    );
    answer(&mut center, &[summary(1, "shown", "completed", 1)], &[]);
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
    assert!(matches!(
        key(&mut center, KeyCode::Char('x')),
        Some(CenterAction::Stop(session)) if session.session_id.value() == 1
    ));
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

#[test]
fn pinned_sessions_come_first_and_hidden_ones_only_in_a_search() {
    let roots = vec![
        summary(1, "latest", "completed", 1),
        summary(2, "pinned", "completed", 2),
        summary(3, "hidden", "completed", 3),
    ];
    let mut center = center(roots.clone(), Vec::new());
    // `p` on the second pins it; the app keeps the marks and hands them back.
    key(&mut center, KeyCode::Down);
    let Some(CenterAction::Marks(marks)) = key(&mut center, KeyCode::Char('p')) else {
        panic!("p pins");
    };
    assert!(marks.pinned.contains(&AgentSessionId::new(2)));
    center.set_marks(marks);
    assert_eq!(titles(&center), ["pinned", "latest", "hidden"]);
    assert_eq!(center.rows()[0].heading.label, "Pinned");
    let screen_text = screen(&mut center, 60, 12);
    assert!(screen_text.contains("Pinned  1"), "{screen_text}");

    key(&mut center, KeyCode::End);
    let Some(CenterAction::Marks(marks)) = key(&mut center, KeyCode::Char('h')) else {
        panic!("h hides");
    };
    center.set_marks(marks.clone());
    assert_eq!(titles(&center), ["pinned", "latest"]);
    let notice = screen(&mut center, 100, 12);
    assert!(notice.contains("Hidden (1 in all). A search"), "{notice}");
    // A search lists a hidden session, marked so.
    key(&mut center, KeyCode::Char('/'));
    typed(&mut center, "hid");
    answer(&mut center, &roots[2..], &[]);
    assert_eq!(titles(&center), ["hidden"]);
    assert!(screen(&mut center, 80, 12).contains("hidden (hidden)"));
}

#[test]
fn a_session_changed_since_it_was_last_shown_is_unread() {
    let mut marks = AgentsMarks::default();
    // Shown five minutes ago: the one updated a minute ago is unread, the
    // one updated ten minutes ago is not, and the one never shown is not.
    for id in [1, 2, 4] {
        marks.seen.insert(AgentSessionId::new(id), NOW - 5 * MINUTE);
    }
    let center = marked(
        vec![
            summary(1, "changed", "completed", 1),
            summary(2, "unchanged", "completed", 10),
            summary(3, "never shown", "completed", 1),
            summary(4, "its child changed", "completed", 10),
        ],
        vec![child(41, 4, "completed")],
        marks,
    );
    let unread: Vec<(String, bool)> = center
        .rows()
        .into_iter()
        .map(|row| (row.summary.title.unwrap().value, row.unread))
        .collect();
    assert_eq!(
        unread,
        [
            ("changed".to_string(), true),
            ("never shown".to_string(), false),
            ("unchanged".to_string(), false),
            ("its child changed".to_string(), true),
        ]
    );
}

#[test]
fn the_watch_reads_again_only_what_changed() {
    let roots = vec![
        summary(1, "first", "completed", 1),
        summary(2, "second", "completed", 2),
    ];
    let children = vec![child(21, 2, "sampling")];
    let mut center = center(roots.clone(), children.clone());
    // The first view is where the watch starts: nothing is read again.
    center.ingest(CenterRead::Live(vec![
        live(1, None, 3),
        live(21, Some(2), 5),
    ]));
    assert!(center.jobs().is_empty());
    // The same view again reads nothing.
    center.ingest(CenterRead::Live(vec![
        live(1, None, 3),
        live(21, Some(2), 5),
    ]));
    assert!(center.jobs().is_empty());

    // A child moved on: its parent's children are read again.
    center.ingest(CenterRead::Live(vec![
        live(1, None, 3),
        live(21, Some(2), 6),
    ]));
    let jobs = center.jobs();
    assert!(
        matches!(jobs.as_slice(), [CenterJob::Children { parent, .. }] if parent.value() == 2),
        "{jobs:?}"
    );
    for job in jobs {
        center.ingest(read_for(job, &roots, &children));
    }

    // A session without a parent moved on: the list, and the selected
    // one's exchange, are read again; the list's answer replaces the rows
    // in place, with no reload.
    let watched = vec![live(1, None, 4), live(2, None, 1), live(21, Some(2), 6)];
    center.ingest(CenterRead::Live(watched));
    let jobs = center.jobs();
    assert!(
        jobs.iter()
            .any(|job| matches!(job, CenterJob::Roots { .. })),
        "{jobs:?}"
    );
    assert!(
        jobs.iter()
            .any(|job| matches!(job, CenterJob::Exchange(id) if id.value() == 2)),
        "{jobs:?}"
    );
    assert_eq!(
        titles(&center),
        ["second", "first"],
        "the rows stay meanwhile"
    );

    // An answer from before a new search is left unused.
    let stale = jobs
        .into_iter()
        .find(|job| matches!(job, CenterJob::Roots { .. }))
        .unwrap();
    key(&mut center, KeyCode::Char('/'));
    typed(&mut center, "x");
    center.ingest(read_for(stale, &roots, &children));
    assert!(titles(&center).is_empty(), "{:?}", titles(&center));
}

#[test]
fn c_selects_the_children_in_the_details_to_open_or_stop_them() {
    let mut center = center(
        vec![summary(1, "parent", "completed", 1)],
        vec![child(11, 1, "completed"), child(12, 1, "sampling")],
    );
    assert_eq!(key(&mut center, KeyCode::Char('c')), None);
    // The first child has the keys; a stop of one that runs nothing says so.
    assert_eq!(key(&mut center, KeyCode::Char('x')), None);
    assert!(screen(&mut center, 120, 30).contains("Nothing runs there."));
    key(&mut center, KeyCode::Down);
    assert!(matches!(
        key(&mut center, KeyCode::Char('x')),
        Some(CenterAction::Stop(child)) if child.session_id.value() == 12
    ));
    assert!(matches!(
        key(&mut center, KeyCode::Enter),
        Some(CenterAction::Open(child)) if child.session_id.value() == 12
    ));
    // Narrow, the details take the list's place while a child is selected.
    let narrow = screen(&mut center, 60, 30);
    assert!(narrow.contains("› ● child 12"), "{narrow}");
    // Esc returns to the sessions, where Enter opens the parent.
    key(&mut center, KeyCode::Esc);
    assert!(matches!(
        key(&mut center, KeyCode::Enter),
        Some(CenterAction::Open(root)) if root.session_id.value() == 1
    ));

    let mut alone = self::center(vec![summary(2, "alone", "completed", 1)], Vec::new());
    key(&mut alone, KeyCode::Char('c'));
    assert!(screen(&mut alone, 100, 12).contains("It has no subagents or reviews."));
}

#[test]
fn the_mouse_selects_opens_and_filters() {
    let mut center = center(
        vec![
            summary(1, "first", "completed", 1),
            summary(2, "second", "sampling", 2),
        ],
        vec![child(21, 2, "sampling")],
    );
    let shown = screen(&mut center, 120, 24);
    // A click on a row selects it, a second one opens it.
    let (column, row) = find(&shown, "second");
    assert_eq!(click(&mut center, column, row), None);
    assert_eq!(center.selected_row().unwrap().summary.session_id.value(), 2);
    assert!(matches!(
        click(&mut center, column, row),
        Some(CenterAction::Open(session)) if session.session_id.value() == 2
    ));

    // The details list its child: a click selects it, another opens it.
    let shown = screen(&mut center, 120, 24);
    let (column, row) = find(&shown, "child 21");
    assert_eq!(click(&mut center, column, row), None);
    assert!(matches!(
        click(&mut center, column, row),
        Some(CenterAction::Open(child)) if child.session_id.value() == 21
    ));

    // The wheel moves the selection and stops at the ends.
    key(&mut center, KeyCode::Esc);
    let wheel = |center: &mut AgentsCenter, kind| {
        center.handle_mouse(MouseEvent {
            kind,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        })
    };
    wheel(&mut center, MouseEventKind::ScrollDown);
    assert_eq!(center.selected_row().unwrap().summary.session_id.value(), 1);
    wheel(&mut center, MouseEventKind::ScrollUp);
    assert_eq!(center.selected_row().unwrap().summary.session_id.value(), 2);

    // A click on a tab shows that filter.
    let shown = screen(&mut center, 120, 24);
    let (column, row) = find(&shown, "Working 1");
    click(&mut center, column, row);
    assert_eq!(titles(&center), ["second"]);
}

#[test]
fn the_details_of_the_selected_session() {
    let mut working = summary(1, "Port the command center", "running_tools", 3);
    working.branch = Some("agents-center".to_string());
    working.usage = serde_json::from_value(json!({
        "lastResponseInputTokens": 1200,
        "lastResponseOutputTokens": 300,
        "cumulativeInputTokens": 45_000,
        "cumulativeOutputTokens": 3_200,
        "contextWindowTokens": null,
        "contextConsumedTokens": null,
        "source": "provider_reported",
    }))
    .unwrap();
    let mut marks = AgentsMarks::default();
    marks.pinned.insert(AgentSessionId::new(2));
    marks.seen.insert(AgentSessionId::new(3), NOW - 50 * MINUTE);
    let mut center = marked(
        vec![
            working,
            summary(2, "Fix the login form", "completed", 42),
            summary(3, "Explain the codebase", "completed", 3),
        ],
        vec![child(11, 1, "running_tools"), child(12, 1, "completed")],
        marks,
    );
    let mut asking = live(11, Some(1), 4);
    asking.waiting = Some(Waiting::Approval("cargo test".to_string()));
    asking.activity = Some("Run command: cargo test".to_string());
    center.ingest(CenterRead::Live(vec![live(1, None, 9), asking]));
    center.ingest(CenterRead::Exchange {
        session_id: AgentSessionId::new(1),
        exchange: Exchange {
            prompt: Some("Port Codex's Agent Center, opened by the Left arrow.".to_string()),
            answer: Some(
                "The details pane lists the children; each opens beside its parent.".to_string(),
            ),
        },
    });
    key(&mut center, KeyCode::Down);
    assert_snapshot!("center_details", screen(&mut center, 130, 40));
    key(&mut center, KeyCode::Char('c'));
    assert_snapshot!("center_children_narrow", screen(&mut center, 60, 40));
}
