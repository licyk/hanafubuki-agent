// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use hanafubuki_agent::AgentTaskPlanSnapshot;

use super::*;

fn render_lines(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

fn plan(explanation: Option<&str>, steps: &[(&str, &str)]) -> AgentTaskPlanSnapshot {
    serde_json::from_value(serde_json::json!({
        "turnId": 1,
        "generation": 1,
        "revision": 1,
        "explanation": explanation,
        "plan": steps
            .iter()
            .map(|(step, status)| serde_json::json!({"step": step, "status": status}))
            .collect::<Vec<_>>(),
        "updatedAt": 0,
    }))
    .unwrap()
}

#[test]
fn user_history_cell_wraps_and_prefixes_each_line_snapshot() {
    let cell = new_user_prompt("one two three four five six seven".to_string());
    // Small width to force wrapping more clearly.
    let lines = cell.display_lines(12);
    insta::assert_snapshot!(render_lines(&lines).join("\n"));
}

#[test]
fn plan_update_with_note_and_wrapping_snapshot() {
    // Long explanation forces wrapping; include long step text to verify step wrapping and alignment.
    let update = plan(
        Some("I’ll update Grafana call error handling by adding retries and clearer messages when the backend is unreachable."),
        &[
            ("Investigate existing error paths and logging around HTTP timeouts", "completed"),
            ("Harden Grafana client error handling with retry/backoff and user‑friendly messages", "in_progress"),
            ("Add tests for transient failure scenarios and surfacing to the UI", "pending"),
        ],
    );
    let lines = new_plan_update(&update).display_lines(32);
    insta::assert_snapshot!(render_lines(&lines).join("\n"));
}

#[test]
fn plan_update_without_note_snapshot() {
    let update = plan(
        None,
        &[
            ("Define error taxonomy", "in_progress"),
            ("Implement mapping to user messages", "pending"),
        ],
    );
    let lines = new_plan_update(&update).display_lines(40);
    insta::assert_snapshot!(render_lines(&lines).join("\n"));
}

#[test]
fn session_header_names_the_model_and_the_directory() {
    let cell = SessionHeaderHistoryCell::new(
        "fixture-model".to_string(),
        "/home/someone/project".to_string(),
        "0.1.0",
    );
    let lines = render_lines(&cell.display_lines(48));
    insta::assert_snapshot!(lines.join("\n"));
}

#[test]
fn the_separator_says_how_long_the_turn_worked() {
    let lines = render_lines(&FinalMessageSeparator::new(Some(75), None).display_lines(24));
    assert_eq!(lines, vec!["─ Worked for 1m 15s ────"]);
}

#[test]
fn the_separator_says_how_long_the_turn_thought_when_it_did() {
    let thought = Some(std::time::Duration::from_millis(20_790));
    let separator = FinalMessageSeparator::new(Some(25), thought);
    let lines = render_lines(&separator.display_lines(44));
    assert_eq!(lines, vec!["─ Worked for 25s · thought for 20.7s ───────"]);
    assert_eq!(
        render_lines(&separator.raw_lines()),
        vec!["Worked for 25s · thought for 20.7s"]
    );
}
