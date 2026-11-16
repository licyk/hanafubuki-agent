use std::time::{Duration, Instant};

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use super::{renderer, Welcome};
use hanafubuki_agent::{AgentTuiPreferences, AgentWelcomeTheme};

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
    let stage = renderer::Renderer::new(AgentWelcomeTheme::Center)
        .stage(area, &buf)
        .unwrap();
    assert!(stage.y > 8);
    assert_eq!(stage.bottom(), area.bottom());
    assert!((stage.x - area.x).abs_diff(area.right() - stage.right()) <= 1);
    assert!(renderer::Renderer::new(AgentWelcomeTheme::Center)
        .stage(Rect::new(0, 0, 50, 40), &buf)
        .is_none());
    buf[(4, 30)].set_symbol("Important notice");
    assert!(renderer::Renderer::new(AgentWelcomeTheme::Center)
        .stage(area, &buf)
        .is_none());
}

#[test]
fn atlas_has_colored_animation_and_loops_without_touching_the_composer() {
    let area = Rect::new(3, 4, 64, 36);
    let mut renderer = renderer::Renderer::new(AgentWelcomeTheme::Center);
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

#[test]
fn every_theme_places_colored_looping_art_inside_the_free_region() {
    let area = Rect::new(3, 2, 120, 48);
    for theme in [
        AgentWelcomeTheme::Center,
        AgentWelcomeTheme::LeftPeek,
        AgentWelcomeTheme::RightSleep,
        AgentWelcomeTheme::RightRead,
        AgentWelcomeTheme::LowerLeft,
    ] {
        let mut renderer = renderer::Renderer::new(theme);
        let mut buf = Buffer::empty(Rect::new(0, 0, 130, 55));
        buf[(3, 5)].set_symbol("Header");
        buf[(3, 50)].set_symbol("Composer");
        let stage = renderer.stage(area, &buf).unwrap();
        assert!(stage.y > 5 && stage.bottom() <= area.bottom());
        match theme {
            AgentWelcomeTheme::LeftPeek | AgentWelcomeTheme::LowerLeft => {
                assert_eq!(stage.x, area.x)
            }
            AgentWelcomeTheme::RightSleep | AgentWelcomeTheme::RightRead => {
                assert_eq!(stage.right(), area.right())
            }
            _ => assert!((stage.x - area.x).abs_diff(area.right() - stage.right()) <= 1),
        }
        renderer.paint(stage, &mut buf, Duration::ZERO, 1.0);
        assert!(dots(&buf) > 80, "{theme:?}");
        assert_eq!(buf[(3, 5)].symbol(), "Header");
        assert_eq!(buf[(3, 50)].symbol(), "Composer");
        let mut end = Buffer::empty(buf.area);
        end[(3, 5)].set_symbol("Header");
        end[(3, 50)].set_symbol("Composer");
        renderer.paint(stage, &mut end, Duration::from_secs(8), 1.0);
        assert_eq!(buf, end, "{theme:?}");
        if let Some(root) = std::env::var_os("HANAFUBUKI_AGENT_WELCOME_THEMES_PREVIEW") {
            let cells: Vec<_> = buf
                .content
                .iter()
                .map(|cell| {
                    let color = match cell.fg {
                        Color::Rgb(r, g, b) => Some([r, g, b]),
                        _ => None,
                    };
                    serde_json::json!({"symbol":cell.symbol(),"color":color})
                })
                .collect();
            std::fs::write(std::path::Path::new(&root).join(format!("{theme:?}.json")),
                serde_json::to_vec(&serde_json::json!({"width":buf.area.width,"height":buf.area.height,"cells":cells})).unwrap()).unwrap();
        }
    }
}

#[test]
fn preferences_disable_refresh_keep_random_stable_and_cannot_revive_a_completed_conversation() {
    let mut welcome = Welcome::new(true);
    let random = welcome.random_theme;
    let mut preferences = AgentTuiPreferences::default();
    welcome.configure(&preferences);
    assert_eq!(welcome.random_theme, random);
    preferences.welcome_enabled = false;
    welcome.configure(&preferences);
    let area = Rect::new(0, 0, 100, 40);
    let mut buf = Buffer::empty(area);
    assert!(!welcome.needs_space());
    assert_eq!(welcome.render(area, &mut buf, true, Instant::now()), None);
    assert_eq!(dots(&buf), 0);
    preferences.welcome_enabled = true;
    preferences.welcome_theme = AgentWelcomeTheme::RightSleep;
    welcome.configure(&preferences);
    assert!(welcome.needs_space());
    preferences.welcome_theme = AgentWelcomeTheme::Random;
    welcome.configure(&preferences);
    assert_eq!(welcome.random_theme, random);
    welcome.dismiss();
    preferences.welcome_theme = AgentWelcomeTheme::LowerLeft;
    welcome.configure(&preferences);
    assert!(!welcome.needs_space());
}
