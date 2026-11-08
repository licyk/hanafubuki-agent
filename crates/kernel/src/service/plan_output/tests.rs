use super::*;

fn collect(chunks: &[&str]) -> Vec<PlanOutputSegment> {
    let mut parser = PlanOutputParser::new();
    let mut segments = Vec::new();
    for chunk in chunks {
        for segment in parser.push_str(chunk) {
            push_segment(&mut segments, segment);
        }
    }
    for segment in parser.finish() {
        push_segment(&mut segments, segment);
    }
    segments
}

#[test]
fn separates_normal_text_and_a_closed_plan_across_split_tags() {
    assert_eq!(
        collect(&[
            "Intro\n<prop",
            "osed_plan>\n- inspect\n",
            "</proposed_",
            "plan>\nOutro",
        ]),
        vec![
            PlanOutputSegment::Normal("Intro\n".to_string()),
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("- inspect\n".to_string()),
            PlanOutputSegment::PlanEnd,
            PlanOutputSegment::Normal("Outro".to_string()),
        ]
    );
}

#[test]
fn accepts_arbitrary_character_boundaries_and_utf8_content() {
    let text = "开头\n<proposed_plan>\n- 检查路径\n</proposed_plan>\n结尾";
    let owned = text.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();
    let chunks = owned.iter().map(String::as_str).collect::<Vec<_>>();

    assert_eq!(
        collect(&chunks),
        vec![
            PlanOutputSegment::Normal("开头\n".to_string()),
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("- 检查路径\n".to_string()),
            PlanOutputSegment::PlanEnd,
            PlanOutputSegment::Normal("结尾".to_string()),
        ]
    );
}

#[test]
fn supports_a_plan_only_response_and_tags_at_end_of_stream() {
    assert_eq!(
        collect(&["<proposed_plan>\nplan\n</proposed_plan>"]),
        vec![
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("plan\n".to_string()),
            PlanOutputSegment::PlanEnd,
        ]
    );
}

#[test]
fn does_not_close_an_incomplete_plan_at_end_of_stream() {
    assert_eq!(
        collect(&["before\n<proposed_plan>\nplan"]),
        vec![
            PlanOutputSegment::Normal("before\n".to_string()),
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("plan".to_string()),
        ]
    );
}

#[test]
fn preserves_malformed_and_unmatched_tag_lines_as_text() {
    assert_eq!(
        collect(&["<proposed_plan> extra\n", "</proposed_plan>\n", "normal",]),
        vec![PlanOutputSegment::Normal(
            "<proposed_plan> extra\n</proposed_plan>\nnormal".to_string()
        )]
    );
}

#[test]
fn reports_repeated_closed_blocks_in_source_order() {
    assert_eq!(
        collect(&[
            "<proposed_plan>\none\n</proposed_plan>\n",
            "<proposed_plan>\ntwo\n</proposed_plan>\n",
        ]),
        vec![
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("one\n".to_string()),
            PlanOutputSegment::PlanEnd,
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("two\n".to_string()),
            PlanOutputSegment::PlanEnd,
        ]
    );
}

#[test]
fn retains_blank_normal_lines_around_a_plan_without_leaking_tags() {
    assert_eq!(
        collect(&["\n  <proposed_plan>  \nbody\n  </proposed_plan>\n\n"]),
        vec![
            PlanOutputSegment::Normal("\n".to_string()),
            PlanOutputSegment::PlanStart,
            PlanOutputSegment::PlanDelta("body\n".to_string()),
            PlanOutputSegment::PlanEnd,
            PlanOutputSegment::Normal("\n".to_string()),
        ]
    );
}

#[test]
fn projection_accepts_exactly_one_closed_non_empty_plan() {
    let mut projection = PlanOutputProjection::default();
    projection.push_str("before\n<proposed_plan>\n- change\n</proposed_plan>\nafter");
    projection.finish();

    assert_eq!(projection.visible_text(), "before\nafter");
    assert_eq!(projection.proposed_plan(), Some("- change\n"));
}

#[test]
fn projection_rejects_incomplete_empty_and_repeated_blocks() {
    for text in [
        "<proposed_plan>\nincomplete",
        "<proposed_plan>\n\n</proposed_plan>",
        "<proposed_plan>\none\n</proposed_plan>\n<proposed_plan>\ntwo\n</proposed_plan>",
    ] {
        let mut projection = PlanOutputProjection::default();
        projection.push_str(text);
        projection.finish();
        assert_eq!(
            projection.proposed_plan(),
            None,
            "unexpected plan for {text}"
        );
    }
}
