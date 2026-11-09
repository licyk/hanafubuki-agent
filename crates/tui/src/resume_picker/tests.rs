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
        })
        .collect()
}

fn picker_with(rows: Vec<Row>, view_rows: usize) -> ResumePicker {
    let mut picker = ResumePicker::new(true);
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
        },
        Row {
            session_id: AgentSessionId::new(2),
            preview: String::from("Investigate lazy pagination cap"),
            updated_at: Some(now - Duration::minutes(35)),
        },
        Row {
            session_id: AgentSessionId::new(3),
            preview: String::from("Explain the codebase"),
            updated_at: Some(now - Duration::hours(2)),
        },
    ];
    let mut picker = picker_with(rows, 3);
    picker.selected = 1;
    picker.scroll_top = 0;

    let metrics = calculate_column_metrics(&picker.rows);

    let width: u16 = 80;
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
    let mut picker = ResumePicker::new(true);
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
