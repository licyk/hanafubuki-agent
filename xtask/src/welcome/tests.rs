use std::fs;
use std::path::PathBuf;

use image::{Rgba, RgbaImage};

use super::{animation, export, options, preview, FRAMES};

#[test]
fn native_frames_loop_blink_and_keep_the_forearm_stationary() {
    let opened = RgbaImage::from_fn(384, 432, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, 10, 255])
    });
    let mut closed = opened.clone();
    for y in 170..210 {
        for x in 120..170 {
            closed.put_pixel(x, y, Rgba([20, 30, 240, 255]));
        }
    }
    let first = animation::frame_at(&opened, &closed, 0);
    assert_eq!(first, animation::frame_at(&opened, &closed, FRAMES));
    let swayed = animation::frame_at(&opened, &closed, 12);
    assert_ne!(first, swayed);
    for index in [12, 29, 54, 75, 95] {
        let frame = animation::frame_at(&opened, &closed, index);
        assert_eq!(
            &frame.as_raw()[396 * 384 * 4..],
            &first.as_raw()[396 * 384 * 4..]
        );
    }
    assert_eq!(
        animation::frame_at(&opened, &closed, 29).get_pixel(145, 190)[2],
        240
    );
    assert_eq!(
        animation::frame_at(&opened, &closed, 75).get_pixel(145, 190)[2],
        240
    );
    assert_eq!(
        animation::frame_at(&opened, &closed, 48).get_pixel(145, 190)[2],
        10
    );
}

#[test]
fn keyframes_replace_only_the_eye_regions_and_validate_landmark_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    RgbaImage::from_pixel(1254, 1254, Rgba([240, 20, 20, 255]))
        .save(dir.path().join("base.png"))
        .unwrap();
    RgbaImage::from_pixel(1254, 1254, Rgba([20, 20, 240, 255]))
        .save(dir.path().join("blink.png"))
        .unwrap();
    let (opened, closed) = animation::source_frames(dir.path()).unwrap();
    assert_eq!(opened.dimensions(), (384, 432));
    assert_eq!(opened.get_pixel(100, 100), closed.get_pixel(100, 100));
    assert_ne!(opened.get_pixel(135, 201), closed.get_pixel(135, 201));
    assert_eq!(opened.get_pixel(0, 0)[3], 0);
    RgbaImage::new(100, 100)
        .save(dir.path().join("blink.png"))
        .unwrap();
    assert!(animation::source_frames(dir.path())
        .unwrap_err()
        .to_string()
        .contains("1254x1254"));
}

#[test]
fn indexed_atlas_preserves_color_transparency_and_is_reproducible() {
    let source = RgbaImage::from_fn(32, 32, |x, _| {
        if x < 16 {
            Rgba([0; 4])
        } else {
            Rgba([40, 100, 240, 255])
        }
    });
    let bytes = export::indexed_png(&source).unwrap();
    assert_eq!(bytes, export::indexed_png(&source).unwrap());
    let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
    assert_eq!(decoded.dimensions(), source.dimensions());
    assert_eq!(decoded.get_pixel(0, 0)[3], 0);
    let color = decoded.get_pixel(31, 31);
    assert!(color[2] > 220 && color[3] > 240);
}

#[test]
fn svg_preview_uses_cell_colors_and_escapes_text() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("cells.json");
    let output = dir.path().join("preview.svg");
    fs::write(&input, r#"{"width":2,"height":1,"future":true,"cells":[{"symbol":"⠁","color":[20,100,240]},{"symbol":"<&>","color":null}]}"#).unwrap();
    preview::run(&input, &output).unwrap();
    let svg = fs::read_to_string(&output).unwrap();
    assert!(svg.contains("viewBox=\"0 0 24 24\""));
    assert!(svg.contains("cx=\"3\" cy=\"3\""));
    assert!(svg.contains("#1464f0"));
    assert!(svg.contains("&lt;&amp;&gt;"));
    fs::write(&input, r#"{"width":2,"height":1,"cells":[]}"#).unwrap();
    assert!(preview::run(&input, &output).is_err());
    assert_eq!(fs::read_to_string(&output).unwrap(), svg);
}

#[test]
fn command_options_support_explicit_paths_and_the_repository_atlas_default() {
    let default = PathBuf::from("repo/crates/tui/assets/welcome");
    let args = ["--source", "audit directory"].map(String::from);
    assert_eq!(
        options(&args, "--source", Some(default.clone())).unwrap(),
        (PathBuf::from("audit directory"), default)
    );
    assert!(options(&args, "--input", None).is_err());
    assert!(options(&["--source".into()], "--source", None).is_err());
    let args = ["--output", "preview.svg", "--input", "cells.json"].map(String::from);
    assert_eq!(
        options(&args, "--input", None).unwrap(),
        (PathBuf::from("cells.json"), PathBuf::from("preview.svg"))
    );
}
