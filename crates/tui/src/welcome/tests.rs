use std::time::{Duration, Instant};

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use super::{renderer, Welcome};

fn dots(buf: &Buffer) -> usize {
    buf.content
        .iter()
        .filter(|cell| {
            cell.symbol()
                .chars()
                .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
        })
        .count()
}

#[test]
fn fades_reverse_from_the_current_opacity_and_dismissal_is_latched() {
    let area = Rect::new(0, 0, 100, 35);
    let now = Instant::now();
    let mut mascot = Welcome::new(true);
    let draw = |mascot: &mut Welcome, elapsed: u64, empty: bool| {
        let mut buf = Buffer::empty(area);
        mascot.render(area, &mut buf, empty, now + Duration::from_millis(elapsed));
        dots(&buf)
    };
    assert_eq!(draw(&mut mascot, 0, true), 0);
    let partial = draw(&mut mascot, 225, true);
    let full = draw(&mut mascot, 450, true);
    assert!(partial > 0 && full >= partial);
    let fading = draw(&mut mascot, 675, false);
    assert!(fading > 0 && fading <= full);
    assert!(draw(&mut mascot, 900, true) >= fading);
    mascot.dismiss();
    assert_eq!(draw(&mut mascot, 1400, true), 0);
    assert_eq!(draw(&mut mascot, 2000, true), 0);
    assert!(!mascot.needs_space());
}

#[test]
fn layout_avoids_notices_centers_the_mascot_and_anchors_its_hands() {
    let area = Rect::new(4, 3, 100, 40);
    let mut buf = Buffer::empty(Rect::new(0, 0, 110, 50));
    buf[(4, 8)].set_symbol("Header");
    let stage = renderer::stage(area, &buf).unwrap();
    assert!(stage.y > 8);
    assert_eq!(stage.bottom(), area.bottom());
    assert!((stage.x - area.x).abs_diff(area.right() - stage.right()) <= 1);
    assert!(renderer::stage(Rect::new(0, 0, 50, 40), &buf).is_none());
    buf[(4, 30)].set_symbol("Important notice");
    assert!(renderer::stage(area, &buf).is_none());
}

#[test]
fn atlas_has_colored_animation_and_loops_without_touching_the_composer() {
    let area = Rect::new(3, 4, 64, 36);
    let mut renderer = renderer::Renderer::default();
    let paint = |renderer: &mut renderer::Renderer, milliseconds| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 75, 45));
        buf[(3, 40)].set_symbol("composer");
        renderer.paint(area, &mut buf, Duration::from_millis(milliseconds), 1.0);
        assert_eq!(buf[(3, 40)].symbol(), "composer");
        assert!(buf
            .content
            .iter()
            .any(|cell| matches!(cell.fg, Color::Rgb(_, _, _))));
        buf
    };
    let first = paint(&mut renderer, 0);
    if let Some(path) = std::env::var_os("HANAFUBUKI_AGENT_WELCOME_PREVIEW") {
        let cells: Vec<_> = first
            .content
            .iter()
            .map(|cell| {
                let color = match cell.fg {
                    Color::Rgb(r, g, b) => Some([r, g, b]),
                    _ => None,
                };
                serde_json::json!({"symbol": cell.symbol(), "color": color})
            })
            .collect();
        std::fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "width": first.area.width, "height": first.area.height, "cells": cells,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    assert!(dots(&first) > 100);
    assert_ne!(first, paint(&mut renderer, 2400));
    assert_eq!(first, paint(&mut renderer, 8000));
}
