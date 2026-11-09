// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Cache reuse, bounded lifetime, and invalidation across frames.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

#[derive(Debug, Default)]
struct Cell {
    renders: AtomicUsize,
    tick: AtomicUsize,
    mutable: bool,
}

impl HistoryCell for Cell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.renders.fetch_add(1, Ordering::Relaxed);
        vec![format!("tick {}", self.tick.load(Ordering::Relaxed)).into()]
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn has_stable_transcript_height(&self) -> bool {
        !self.mutable
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        Some(self.tick.load(Ordering::Relaxed) as u64)
    }
}

const PLAIN: CellPresentation = CellPresentation {
    separated: false,
    expanded: false,
    disclosure: false,
    detailed: false,
};

fn get(cache: &mut LayoutCache, cell: &Arc<dyn HistoryCell>, width: u16) -> Arc<TextLayout> {
    cache.get(cell, width, PLAIN, || {
        TextLayout::new(cell.transcript_lines(width), width)
    })
}

#[test]
fn layouts_refresh_for_width_animation_and_mutable_frames() {
    for mutable in [false, true] {
        let cell = Arc::new(Cell {
            mutable,
            ..Cell::default()
        });
        let history = cell.clone() as Arc<dyn HistoryCell>;
        let mut cache = LayoutCache::default();
        cache.begin_frame();
        get(&mut cache, &history, 20);
        get(&mut cache, &history, 20);
        assert_eq!(cell.renders.load(Ordering::Relaxed), 1);
        cache.begin_frame();
        get(&mut cache, &history, 20);
        assert_eq!(
            cell.renders.load(Ordering::Relaxed),
            1 + usize::from(mutable)
        );
        cell.tick.store(2, Ordering::Relaxed);
        cache.begin_frame();
        assert_eq!(get(&mut cache, &history, 20).text(), "tick 2");
        get(&mut cache, &history, 10);
        assert_eq!(
            cell.renders.load(Ordering::Relaxed),
            3 + usize::from(mutable)
        );
    }
}

#[test]
fn recent_entries_are_reused_and_old_entries_are_evicted() {
    let cells: Vec<_> = (0..100).map(|_| Arc::new(Cell::default())).collect();
    let mut cache = LayoutCache::default();
    cache.begin_frame();
    for cell in &cells {
        get(&mut cache, &(cell.clone() as Arc<dyn HistoryCell>), 20);
    }
    for index in [99, 98, 0] {
        get(
            &mut cache,
            &(cells[index].clone() as Arc<dyn HistoryCell>),
            20,
        );
    }
    assert_eq!(
        [99, 98, 0].map(|index| cells[index].renders.load(Ordering::Relaxed)),
        [1, 1, 2]
    );
}

#[test]
fn an_activity_control_names_what_its_details_add() {
    let lines = |disclosure| ActivityTranscriptLines {
        activity: vec![Line::from("• Called shell")],
        disclosure,
    };
    let label = |layout: TextLayout| {
        let area = Rect::new(0, 0, 30, 2);
        let mut buffer = Buffer::empty(area);
        layout.render(area, &mut buffer, 1);
        (0..30)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim()
            .to_owned()
    };
    assert_eq!(
        label(activity_layout(
            lines(Some(ActivityDisclosure::OutputLines(1))),
            30,
            false
        )),
        "+ 1 line"
    );
    assert_eq!(
        label(activity_layout(
            lines(Some(ActivityDisclosure::OutputLines(3))),
            30,
            true
        )),
        "− Show less"
    );
    assert_eq!(
        label(activity_layout(
            lines(Some(ActivityDisclosure::Generic)),
            30,
            false
        )),
        "+ Show details"
    );
    // Nothing to disclose, no control.
    assert_eq!(activity_layout(lines(None), 30, false).row_count(), 1);
}
