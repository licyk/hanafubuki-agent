// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use chrono::Duration;
use insta::assert_snapshot;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::custom_terminal::Terminal;
use crate::test_support::VT100Backend;

fn rows(count: usize) -> Vec<Row> {
    let now = Utc::now();
    (0..count)
        .map(|idx| Row {
            session_id: AgentSessionId::new(idx as u64 + 1),
            preview: format!("item-{idx}"),
            updated_at: Some(now - Duration::days(idx as i64)),
            cwd: None,
            branch: None,
        })
        .collect()
}

fn picker_with(rows: Vec<Row>, view_rows: usize) -> ResumePicker {
    let mut picker = ResumePicker::new(true, None);
    picker.loaded = true;
    picker.rows = rows;
    picker.update_view_rows(view_rows);
    picker
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn resume_table_snapshot() {
    let now = Utc::now();
    let rows = vec![
        Row {
            session_id: AgentSessionId::new(1),
            preview: String::from("Fix resume picker timestamps"),
            updated_at: Some(now - Duration::seconds(42)),
            cwd: Some(String::from("/home/user/projects/hanafubuki-agent")),
            branch: Some(String::from("main")),
        },
        Row {
            session_id: AgentSessionId::new(2),
            preview: String::from("Investigate lazy pagination cap"),
            updated_at: Some(now - Duration::minutes(35)),
            cwd: Some(String::from("/srv/app")),
            branch: Some(String::from("fix/pagination-cap-for-long-lists")),
        },
        Row {
            session_id: AgentSessionId::new(3),
            preview: String::from("Explain the codebase"),
            updated_at: Some(now - Duration::hours(2)),
            cwd: None,
            branch: None,
        },
    ];
    let mut picker = picker_with(rows, 3);
    picker.selected = 1;
    picker.scroll_top = 0;

    let metrics = calculate_column_metrics(&picker.rows);

    let width: u16 = 110;
    let height: u16 = 6;
    let backend = VT100Backend::new(width, height);
    let mut terminal = Terminal::with_options(backend).expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, width, height));

    {
        let mut frame = terminal.get_frame();
        let area = frame.area();
        let segments = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
        render_column_headers(frame.buffer_mut(), segments[0], &metrics);
        picker.render_list(frame.buffer_mut(), segments[1], &metrics);
    }
    terminal.flush().expect("flush");

    let snapshot = terminal.backend().to_string();
    assert_snapshot!("resume_picker_table", snapshot);
}

#[test]
fn resume_picker_screen_snapshot() {
    let mut picker = ResumePicker::new(true, None);
    picker.loaded = true;

    let width: u16 = 80;
    let height: u16 = 9;
    let backend = VT100Backend::new(width, height);
    let mut terminal = Terminal::with_options(backend).expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, width, height));

    {
        let mut frame = terminal.get_frame();
        let area = frame.area();
        picker.render(area, frame.buffer_mut());
    }
    terminal.flush().expect("flush");

    let snapshot = terminal.backend().to_string();
    assert_snapshot!("resume_picker_screen", snapshot);
}

#[test]
fn page_navigation_uses_view_rows() {
    let mut picker = picker_with(rows(20), 5);

    assert_eq!(picker.selected, 0);
    picker.handle_key(key(KeyCode::PageDown));
    assert_eq!(picker.selected, 5);

    picker.handle_key(key(KeyCode::PageDown));
    assert_eq!(picker.selected, 10);

    picker.handle_key(key(KeyCode::PageUp));
    assert_eq!(picker.selected, 5);
}

#[test]
fn up_at_bottom_does_not_scroll_when_visible() {
    let mut picker = picker_with(rows(10), 5);

    picker.selected = picker.rows.len().saturating_sub(1);
    picker.ensure_selected_visible();

    let initial_top = picker.scroll_top;
    assert_eq!(initial_top, picker.rows.len().saturating_sub(5));

    picker.handle_key(key(KeyCode::Up));

    assert_eq!(picker.scroll_top, initial_top);
    assert_eq!(picker.selected, picker.rows.len().saturating_sub(2));
}

#[test]
fn enter_resumes_the_selected_session_and_esc_starts_fresh() {
    let mut picker = picker_with(rows(3), 5);
    picker.handle_key(key(KeyCode::Down));
    assert_eq!(
        picker.handle_key(key(KeyCode::Enter)),
        Some(SessionSelection::Resume(AgentSessionId::new(2)))
    );
    assert_eq!(
        picker.handle_key(key(KeyCode::Esc)),
        Some(SessionSelection::StartFresh)
    );
    assert_eq!(
        picker.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(SessionSelection::Exit)
    );
}

#[test]
fn more_is_asked_for_near_the_end_and_a_query_starts_over() {
    let mut picker = picker_with(rows(10), 5);
    picker.next_cursor = Some("next".to_string());
    assert_eq!(picker.page_request(), None);

    picker.selected = 5;
    let request = picker.page_request().expect("a page near the end");
    assert_eq!(request.cursor.as_deref(), Some("next"));
    assert_eq!(request.search, None);

    picker.handle_key(key(KeyCode::Char('f')));
    picker.handle_key(key(KeyCode::Char('x')));
    let request = picker.page_request().expect("the query's first page");
    assert_eq!(request.cursor, None);
    assert_eq!(request.search.as_deref(), Some("fx"));
    assert!(picker.rows.is_empty());
}

#[test]
fn tab_lists_the_archived_sessions_and_ctrl_a_archives_or_restores() {
    let mut picker = picker_with(rows(3), 5);
    let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let request = picker.page_request().map(|request| request.lifecycle);
    assert_eq!(request, None);

    picker.handle_key(key(KeyCode::Down));
    assert_eq!(picker.handle_key(ctrl_a), None);
    let archived = AgentSessionId::new(2);
    assert_eq!(
        picker.take_archive_request(),
        Some(ArchiveRequest {
            session_id: archived,
            archive: true
        })
    );
    assert_eq!(picker.take_archive_request(), None);
    picker.archive_done(archived, Ok(()));
    let left: Vec<u64> = picker
        .rows
        .iter()
        .map(|row| row.session_id.value())
        .collect();
    assert_eq!(left, [1, 3]);
    assert_eq!(picker.notice.as_deref(), Some("Session archived"));

    picker.handle_key(key(KeyCode::Tab));
    assert!(picker.rows.is_empty());
    let request = picker
        .page_request()
        .expect("the archived sessions' first page");
    assert_eq!(request.lifecycle, AgentSessionLifecycle::Archived);
    picker.rows = rows(1);
    picker.loaded = true;
    // Enter on an archived session restores it.
    assert_eq!(picker.handle_key(key(KeyCode::Enter)), None);
    assert_eq!(
        picker.take_archive_request(),
        Some(ArchiveRequest {
            session_id: AgentSessionId::new(1),
            archive: false
        })
    );
    picker.archive_done(AgentSessionId::new(1), Err("stale".to_string()));
    assert_eq!(
        picker.notice.as_deref(),
        Some("The session was not restored: stale")
    );
}

#[test]
fn the_shown_session_is_not_archived_from_the_picker() {
    let mut picker = ResumePicker::new(false, Some(AgentSessionId::new(1)));
    picker.loaded = true;
    picker.rows = rows(2);
    picker.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(picker.take_archive_request(), None);
    assert_eq!(
        picker.notice.as_deref(),
        Some("Use /archive to archive the current session")
    );
}

#[test]
fn a_query_given_at_opening_is_searched_from_the_start() {
    let picker = ResumePicker::new(false, None).with_query("login fix".to_string());
    let request = picker.page_request().expect("the query's first page");
    assert_eq!(request.search.as_deref(), Some("login fix"));
    assert_eq!(request.lifecycle, AgentSessionLifecycle::Available);
}

#[test]
fn a_long_directory_keeps_its_end() {
    let shortened = shorten_directory("/home/someone/projects/a-rather-long-repository-name");
    assert_eq!(UnicodeWidthStr::width(shortened.as_str()), DIRECTORY_WIDTH);
    assert!(shortened.starts_with('…'), "{shortened}");
    assert!(shortened.ends_with("long-repository-name"), "{shortened}");
    assert_eq!(shorten_directory("/srv/app"), "/srv/app");
}
