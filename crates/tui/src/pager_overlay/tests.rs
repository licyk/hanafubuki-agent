// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[derive(Debug)]
struct TestCell {
    lines: Vec<Line<'static>>,
}

impl HistoryCell for TestCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.lines.clone()
    }
}

fn paragraph_block(label: &str, lines: usize) -> Box<dyn Renderable> {
    let text = Text::from(
        (0..lines)
            .map(|i| Line::from(format!("{label}{i}")))
            .collect::<Vec<_>>(),
    );
    Box::new(Paragraph::new(text)) as Box<dyn Renderable>
}

#[test]
fn transcript_overlay_snapshot_basic() {
    // Prepare a transcript overlay with a few lines
    let mut overlay = TranscriptOverlay::new(vec![
        Arc::new(TestCell {
            lines: vec![Line::from("alpha")],
        }),
        Arc::new(TestCell {
            lines: vec![Line::from("beta")],
        }),
        Arc::new(TestCell {
            lines: vec![Line::from("gamma")],
        }),
    ]);
    let mut term = Terminal::new(TestBackend::new(40, 10)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

#[test]
fn transcript_overlay_keeps_scroll_pinned_at_bottom() {
    let mut overlay = TranscriptOverlay::new(
        (0..20)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line{i}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    let mut term = Terminal::new(TestBackend::new(40, 12)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");

    assert!(
        overlay.view.is_scrolled_to_bottom(),
        "expected initial render to leave view at bottom"
    );

    overlay.insert_cell(Arc::new(TestCell {
        lines: vec!["tail".into()],
    }));

    assert_eq!(overlay.view.scroll_offset, usize::MAX);
}

#[test]
fn transcript_overlay_preserves_manual_scroll_position() {
    let mut overlay = TranscriptOverlay::new(
        (0..20)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line{i}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    let mut term = Terminal::new(TestBackend::new(40, 12)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");

    overlay.view.scroll_offset = 0;

    overlay.insert_cell(Arc::new(TestCell {
        lines: vec!["tail".into()],
    }));

    assert_eq!(overlay.view.scroll_offset, 0);
}

#[test]
fn static_overlay_snapshot_basic() {
    // Prepare a static overlay with a few lines and a title
    let mut overlay = StaticOverlay::with_title(
        vec!["one".into(), "two".into(), "three".into()],
        "S T A T I C".to_string(),
    );
    let mut term = Terminal::new(TestBackend::new(40, 10)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

/// Render transcript overlay and return visible line numbers (`line-NN`) in order.
fn transcript_line_numbers(overlay: &mut TranscriptOverlay, area: Rect) -> Vec<usize> {
    let mut buf = Buffer::empty(area);
    overlay.render(area, &mut buf);

    let top_h = area.height.saturating_sub(3);
    let top = Rect::new(area.x, area.y, area.width, top_h);
    let content_area = overlay.view.content_area(top);

    let mut nums = Vec::new();
    for y in content_area.y..content_area.bottom() {
        let mut line = String::new();
        for x in content_area.x..content_area.right() {
            line.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        if let Some(n) = line
            .split_whitespace()
            .find_map(|w| w.strip_prefix("line-"))
            .and_then(|s| s.parse().ok())
        {
            nums.push(n);
        }
    }
    nums
}

#[test]
fn transcript_overlay_paging_is_continuous_and_round_trips() {
    let mut overlay = TranscriptOverlay::new(
        (0..50)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line-{i:02}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    let area = Rect::new(0, 0, 40, 15);

    // Prime layout so last_content_height is populated and paging uses the real content height.
    let mut buf = Buffer::empty(area);
    overlay.view.scroll_offset = 0;
    overlay.render(area, &mut buf);
    let page_height = overlay.view.page_height(area);

    // Scenario 1: starting from the top, PageDown should show the next page of content.
    overlay.view.scroll_offset = 0;
    let page1 = transcript_line_numbers(&mut overlay, area);
    let page1_len = page1.len();
    let expected_page1: Vec<usize> = (0..page1_len).collect();
    assert_eq!(
        page1, expected_page1,
        "first page should start at line-00 and show a full page of content"
    );

    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_add(page_height);
    let page2 = transcript_line_numbers(&mut overlay, area);
    assert_eq!(
        page2.len(),
        page1_len,
        "second page should have the same number of visible lines as the first page"
    );
    let expected_page2_first = *page1.last().unwrap() + 1;
    assert_eq!(
        page2[0], expected_page2_first,
        "second page after PageDown should immediately follow the first page"
    );

    // Scenario 2: from an interior offset (start=3), PageDown then PageUp should round-trip.
    let interior_offset = 3usize;
    overlay.view.scroll_offset = interior_offset;
    let before = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_add(page_height);
    let _ = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_sub(page_height);
    let after = transcript_line_numbers(&mut overlay, area);
    assert_eq!(
        before, after,
        "PageDown+PageUp from interior offset ({interior_offset}) should round-trip"
    );

    // Scenario 3: from the top of the second page, PageUp then PageDown should round-trip.
    overlay.view.scroll_offset = page_height;
    let before2 = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_sub(page_height);
    let _ = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_add(page_height);
    let after2 = transcript_line_numbers(&mut overlay, area);
    assert_eq!(
        before2, after2,
        "PageUp+PageDown from the top of the second page should round-trip"
    );
}

#[test]
fn static_overlay_wraps_long_lines() {
    let mut overlay = StaticOverlay::with_title(
        vec![
            "a very long line that should wrap when rendered within a narrow pager overlay width"
                .into(),
        ],
        "S T A T I C".to_string(),
    );
    let mut term = Terminal::new(TestBackend::new(24, 8)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

#[test]
fn pager_view_content_height_counts_renderables() {
    let pv = PagerView::new(
        vec![paragraph_block("a", 2), paragraph_block("b", 3)],
        "T".to_string(),
        0,
    );

    assert_eq!(pv.content_height(80), 5);
}

#[test]
fn pager_view_is_scrolled_to_bottom_accounts_for_wrapped_height() {
    let mut pv = PagerView::new(vec![paragraph_block("a", 10)], "T".to_string(), 0);
    let area = Rect::new(0, 0, 20, 8);
    let mut buf = Buffer::empty(area);

    pv.render(area, &mut buf);

    assert!(
        !pv.is_scrolled_to_bottom(),
        "expected view to report not at bottom when offset < max"
    );

    pv.scroll_offset = usize::MAX;
    pv.render(area, &mut buf);

    assert!(
        pv.is_scrolled_to_bottom(),
        "expected view to report at bottom after scrolling to end"
    );
}
