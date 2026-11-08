use super::*;

fn stream(fragments: &[(u32, &str)]) -> (ReasoningHeadlineProjection, Vec<String>) {
    let mut projection = ReasoningHeadlineProjection::default();
    let mut published = Vec::new();
    for (summary_index, delta) in fragments {
        if let Some(headline) = projection.push(*summary_index, delta) {
            published.push(headline);
        }
    }
    (projection, published)
}

#[test]
fn the_streaming_line_becomes_the_headline_as_it_grows() {
    let (projection, published) = stream(&[(0, "Mapp"), (0, "ing the"), (0, " app")]);

    assert_eq!(published, ["Mapp", "Mapping the", "Mapping the app"]);
    assert_eq!(projection.headline.as_deref(), Some("Mapping the app"));
}

#[test]
fn a_bold_heading_is_withheld_until_it_closes_and_then_loses_its_markers() {
    let (_, published) = stream(&[
        (0, "**Mapping"),
        (0, " the app"),
        (0, "**\n"),
        (0, "\nBody text"),
    ]);

    assert_eq!(published, ["Mapping the app", "Body text"]);
}

#[test]
fn heading_markers_comments_and_blank_lines_are_not_headlines() {
    let (_, published) = stream(&[(0, "## Plan\n"), (0, "<!-- hidden -->\n"), (0, "   \n")]);

    assert_eq!(published, ["Plan"]);
}

#[test]
fn a_new_summary_part_ends_the_previous_line_without_a_newline() {
    let (_, published) = stream(&[(0, "Body text."), (1, "**Next"), (1, " step**")]);

    assert_eq!(published, ["Body text.", "Next step"]);
}

#[test]
fn a_repeated_fragment_with_the_same_headline_publishes_nothing() {
    let mut projection = ReasoningHeadlineProjection::default();
    assert_eq!(projection.push(0, "Same\n").as_deref(), Some("Same"));
    assert_eq!(projection.push(0, "\n"), None);
    assert_eq!(projection.push(0, "<!-- note -->"), None);
    assert_eq!(projection.headline.as_deref(), Some("Same"));
}

#[test]
fn an_overlong_line_keeps_a_bounded_prefix_on_a_character_boundary() {
    let prefix = "界".repeat(MAX_REASONING_HEADLINE_BYTES / 3 - 1);
    let (projection, published) = stream(&[(0, &prefix), (0, "界界 and much more")]);

    let headline = published.last().unwrap();
    assert!(headline.len() <= MAX_REASONING_HEADLINE_BYTES);
    assert!(headline.starts_with(&prefix));
    assert_eq!(headline.chars().count(), MAX_REASONING_HEADLINE_BYTES / 3);
    assert_eq!(projection.line.len(), headline.len());

    let full = "a".repeat(MAX_REASONING_HEADLINE_BYTES - 1);
    let mut projection = ReasoningHeadlineProjection::default();
    assert_eq!(projection.push(0, &full).as_deref(), Some(full.as_str()));
    assert_eq!(projection.push(0, "界 does not fit"), None);
    assert_eq!(
        projection.push(0, "\nNext line").as_deref(),
        Some("Next line")
    );
}
