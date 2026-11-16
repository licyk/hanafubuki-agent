use std::path::Path;

use image::{imageops, GrayImage, Rgba, RgbaImage};

use super::{FPS, FRAMES};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(super) fn source_frames(source: &Path) -> Result<(RgbaImage, RgbaImage)> {
    let base = image::open(source.join("base.png"))?.into_rgba8();
    let blink = image::open(source.join("blink.png"))?.into_rgba8();
    if base.dimensions() != (1254, 1254) || blink.dimensions() != base.dimensions() {
        return Err("eye landmarks require matching 1254x1254 base.png and blink.png".into());
    }
    let eyes = GrayImage::from_fn(1254, 1254, |x, y| {
        let inside = [(429., 465., 589., 613.), (652., 431., 808., 585.)]
            .iter()
            .any(|&(left, top, right, bottom)| {
                let dx = (f64::from(x) - (left + right) / 2.) / ((right - left) / 2.);
                let dy = (f64::from(y) - (top + bottom) / 2.) / ((bottom - top) / 2.);
                dx * dx + dy * dy <= 1.
            });
        image::Luma([if inside { 255 } else { 0 }])
    });
    let eyes = imageops::blur(&eyes, 5.0);
    let closed = RgbaImage::from_fn(1254, 1254, |x, y| {
        let weight = u32::from(eyes.get_pixel(x, y)[0]);
        let open = base.get_pixel(x, y);
        let shut = blink.get_pixel(x, y);
        Rgba(std::array::from_fn(|c| {
            ((u32::from(open[c]) * (255 - weight) + u32::from(shut[c]) * weight + 127) / 255) as u8
        }))
    });
    Ok((prepare(base), prepare(closed)))
}

fn prepare(mut source: RgbaImage) -> RgbaImage {
    for pixel in source.pixels_mut() {
        if pixel[3] < 16 {
            *pixel = Rgba([0; 4]);
        } else if pixel[3] > 240 {
            pixel[3] = 255;
        }
    }
    let cropped = imageops::crop_imm(&source, 90, 0, 1060, 1242).to_image();
    let scaled = imageops::resize(&cropped, 348, 408, imageops::FilterType::Lanczos3);
    let mut canvas = RgbaImage::new(384, 432);
    imageops::overlay(&mut canvas, &scaled, 18, 24);
    canvas
}

pub(super) fn frame_at(opened: &RgbaImage, closed: &RgbaImage, frame: u32) -> RgbaImage {
    let seconds = f64::from(frame % FRAMES) / f64::from(FPS);
    let phase = seconds / (f64::from(FRAMES) / f64::from(FPS)) * std::f64::consts::TAU;
    let blinking = [2.42, 6.25]
        .iter()
        .any(|center| (seconds - center).abs() <= 0.145);
    let source = if blinking { closed } else { opened };
    let twitch = (-((seconds - 4.5) / 0.22).powi(2)).exp() * ((seconds - 4.5) * 17.).sin();
    let (width, height) = source.dimensions();
    let mut output = RgbaImage::new(width, height);
    for top in (0..height).step_by(16) {
        for left in (0..width).step_by(16) {
            let right = (left + 16).min(width);
            let bottom = (top + 16).min(height);
            let corners = [(left, top), (right, top), (left, bottom), (right, bottom)]
                .map(|(x, y)| mapped(x, y, width, height, phase, twitch));
            for y in top..bottom {
                for x in left..right {
                    let u = f64::from(x - left) / f64::from(right - left);
                    let v = f64::from(y - top) / f64::from(bottom - top);
                    let [sx, sy] = std::array::from_fn(|c| {
                        (corners[0][c] * (1. - u) + corners[1][c] * u) * (1. - v)
                            + (corners[2][c] * (1. - u) + corners[3][c] * u) * v
                    });
                    let pixel = imageops::interpolate_bilinear(source, sx as f32, sy as f32)
                        .unwrap_or(Rgba([0; 4]));
                    output.put_pixel(x, y, pixel);
                }
            }
        }
    }
    output
}

fn mapped(x: u32, y: u32, width: u32, height: u32, phase: f64, twitch: f64) -> [f64; 2] {
    let u = f64::from(x) / f64::from(width);
    let v = f64::from(y) / f64::from(height);
    let anchored = ((0.85 - v) / 0.6).clamp(0., 1.);
    let hair = (-((v - 0.51) / 0.19).powi(2)).exp()
        * ((-((u - 0.17) / 0.11).powi(2)).exp() + (-((u - 0.83) / 0.11).powi(2)).exp());
    let ear = (-((v - 0.12) / 0.1).powi(2)).exp()
        * ((-((u - 0.22) / 0.12).powi(2)).exp() - (-((u - 0.73) / 0.12).powi(2)).exp());
    let sway =
        (0.012 * phase.sin() + 0.007 * (phase * 2.).sin() * hair + 0.013 * twitch * ear) * anchored;
    let breath = 0.003 * (1. - (phase * 2.).cos()) * anchored;
    [
        f64::from(x) - sway * f64::from(width),
        f64::from(y) - breath * f64::from(height),
    ]
}
