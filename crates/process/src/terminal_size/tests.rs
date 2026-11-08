use super::*;

const NARROW: TerminalSize = TerminalSize::new(80, 24);
const WIDE: TerminalSize = TerminalSize::new(132, 40);
const TALL: TerminalSize = TerminalSize::new(132, 60);

fn resize(offset: u64, size: TerminalSize) -> TerminalResize {
    TerminalResize { offset, size }
}

#[test]
fn records_changes_at_the_offsets_they_took_effect() {
    let mut timeline = TerminalSizeTimeline::new(NARROW);
    assert!(timeline.record(10, WIDE));
    assert!(timeline.record(25, TALL));

    assert_eq!(timeline.current(), TALL);
    assert_eq!(timeline.size_at(0), NARROW);
    assert_eq!(timeline.size_at(9), NARROW);
    assert_eq!(timeline.size_at(10), WIDE);
    assert_eq!(timeline.size_at(30), TALL);
    assert_eq!(
        timeline.changes_between(0, 40),
        vec![resize(10, WIDE), resize(25, TALL)]
    );
}

#[test]
fn an_unchanged_size_is_not_a_change() {
    let mut timeline = TerminalSizeTimeline::new(NARROW);
    assert!(!timeline.record(10, NARROW));
    assert!(timeline.changes_between(0, 20).is_empty());
}

#[test]
fn changes_without_output_between_them_collapse() {
    let mut timeline = TerminalSizeTimeline::new(NARROW);
    assert!(timeline.record(10, WIDE));
    assert!(timeline.record(10, TALL));
    assert_eq!(timeline.changes_between(0, 20), vec![resize(10, TALL)]);

    // Returning to the size before them leaves no change at all.
    assert!(timeline.record(10, NARROW));
    assert!(timeline.changes_between(0, 20).is_empty());
    assert_eq!(timeline.current(), NARROW);
}

#[test]
fn a_change_before_any_output_replaces_the_starting_size() {
    let mut timeline = TerminalSizeTimeline::new(NARROW);
    assert!(timeline.record(0, WIDE));
    assert_eq!(timeline.size_at(0), WIDE);
    assert!(timeline.changes_between(0, 10).is_empty());
}

#[test]
fn range_bounds_exclude_a_change_at_either_end() {
    let mut timeline = TerminalSizeTimeline::new(NARROW);
    timeline.record(10, WIDE);
    timeline.record(20, TALL);

    // A change at the start is the read's starting size; one at the end
    // belongs to the next read.
    assert_eq!(timeline.changes_between(10, 20), Vec::new());
    assert_eq!(timeline.size_at(10), WIDE);
    assert_eq!(
        timeline.changes_between(9, 21),
        vec![resize(10, WIDE), resize(20, TALL)]
    );
}

#[test]
fn trimming_keeps_the_size_in_effect_at_the_oldest_offset() {
    let mut timeline = TerminalSizeTimeline::new(NARROW);
    timeline.record(10, WIDE);
    timeline.record(20, TALL);

    timeline.trim_before(15);
    assert_eq!(timeline.size_at(15), WIDE);
    assert_eq!(timeline.changes_between(15, 30), vec![resize(20, TALL)]);

    timeline.trim_before(20);
    assert_eq!(timeline.size_at(20), TALL);
    assert_eq!(timeline.current(), TALL);
}

#[test]
fn serializes_a_resize_as_offset_and_size() {
    let value = serde_json::to_value(resize(7, WIDE)).unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "offset": 7, "cols": 132, "rows": 40 })
    );
}
