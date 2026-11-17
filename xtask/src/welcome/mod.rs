//! Native asset production and audit previews for the welcome animation.

mod animation;
mod export;
mod pose;
mod preview;

use std::path::Path;

use serde::{Deserialize, Serialize};

const WIDTH: u32 = 192;
const HEIGHT: u32 = 216;
const FPS: u32 = 12;
const FRAMES: u32 = 96;
const COLUMNS: u32 = 8;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
struct Metadata {
    width: u32,
    height: u32,
    fps: u32,
    frames: u32,
    columns: u32,
    layout: pose::Layout,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            width: WIDTH,
            height: HEIGHT,
            fps: FPS,
            frames: FRAMES,
            columns: COLUMNS,
            layout: pose::Layout::default(),
        }
    }
}

pub(crate) fn generate(source: &Path, output: &Path) -> Result<(), String> {
    export::generate(source, output).map_err(|error| error.to_string())
}

pub(crate) fn preview(input: &Path, output: &Path) -> Result<(), String> {
    preview::run(input, output).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
