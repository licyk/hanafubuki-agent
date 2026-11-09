use super::*;

fn text(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn the_recap_wraps_under_its_label_and_ends_with_the_next_step() {
    let cell = RecapCell(AgentRecap {
        summary: "Fixed the login timeout; the fix is not deployed yet.".to_string(),
        next_action: Some("Deploy the fix?".to_string()),
    });

    assert_eq!(
        text(&cell.display_lines(40)),
        [
            "  ↳ Recap: Fixed the login timeout; the",
            "          fix is not deployed yet.",
            "          Next: Deploy the fix?",
        ]
    );
    assert_eq!(
        text(&cell.raw_lines()),
        [
            "Recap:",
            "Fixed the login timeout; the fix is not deployed yet.",
            "Next: Deploy the fix?",
        ]
    );
}

#[test]
fn a_recap_without_a_next_step_has_no_next_line() {
    let cell = RecapCell(AgentRecap {
        summary: "Nothing is pending.".to_string(),
        next_action: None,
    });

    assert_eq!(
        text(&cell.display_lines(80)),
        ["  ↳ Recap: Nothing is pending."]
    );
}
