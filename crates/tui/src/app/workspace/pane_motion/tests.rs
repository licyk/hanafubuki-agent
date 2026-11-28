// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The side pane's slide: where its share starts and ends, how it is eased,
//! and that it stops asking for frames once it arrives.

use super::*;

#[test]
fn an_unmoved_pane_stays_where_it_is_without_a_frame() {
    let now = Instant::now();
    let mut motion = PaneMotion::default();
    assert_eq!(motion.advance(false, now, true), (0.0, false));
    let mut open = PaneMotion {
        fraction: 1.0,
        target: 1.0,
        ..PaneMotion::default()
    };
    assert_eq!(open.advance(true, now, true), (1.0, false));
}

#[test]
fn opening_slides_to_the_full_width_and_stops_there() {
    let start = Instant::now();
    let mut motion = PaneMotion::default();
    let (share, sliding) = motion.advance(true, start, true);
    assert_eq!((share, sliding), (0.0, true));
    // Halfway, the share is between: a slide, not a jump.
    let (share, sliding) = motion.advance(true, start + SLIDE_DURATION / 2, true);
    assert!(share > 0.0 && share < 1.0, "{share}");
    assert!(sliding);
    assert_eq!(
        motion.advance(true, start + SLIDE_DURATION, true),
        (1.0, false)
    );
}

/// A closed pane is gone at once: its conversation, its drafts and its focus
/// are torn down as it closes, so a slide out would draw an empty pane.
#[test]
fn closing_leaves_no_pane_at_all() {
    let start = Instant::now();
    let mut motion = PaneMotion {
        fraction: 1.0,
        target: 1.0,
        ..PaneMotion::default()
    };
    assert_eq!(motion.advance(false, start, true), (0.0, false));
}

#[test]
fn the_slide_is_eased_so_it_starts_and_ends_slowly() {
    let start = Instant::now();
    let at = |motion: &mut PaneMotion, nanos: u64| {
        motion
            .advance(true, start + Duration::from_nanos(nanos), true)
            .0
    };
    let mut motion = PaneMotion::default();
    let quarter = at(&mut motion, SLIDE_DURATION.as_nanos() as u64 / 4);
    // A quarter of the time is less than a quarter of the distance.
    assert!(quarter < 0.25, "{quarter}");
}

#[test]
fn without_animation_the_pane_is_where_it_belongs_at_once() {
    let now = Instant::now();
    let mut motion = PaneMotion::default();
    assert_eq!(motion.advance(true, now, false), (1.0, false));
    assert_eq!(motion.advance(false, now, false), (0.0, false));
}
