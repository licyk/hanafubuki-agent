// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn paragraph_block(label: &str, lines: usize) -> Box<dyn Renderable> {
    let text = Text::from(
        (0..lines)
            .map(|i| Line::from(format!("{label}{i}")))
            .collect::<Vec<_>>(),
    );
    Box::new(Paragraph::new(text)) as Box<dyn Renderable>
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
