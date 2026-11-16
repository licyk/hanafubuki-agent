//! Native asset production and audit previews for the welcome animation.

mod animation;
mod export;
mod preview;

use std::path::{Path, PathBuf};

use serde::Serialize;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 216;
const FPS: u32 = 12;
const FRAMES: u32 = 96;
const COLUMNS: u32 = 8;

#[derive(Serialize)]
struct Metadata {
    width: u32,
    height: u32,
    fps: u32,
    frames: u32,
    columns: u32,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            width: WIDTH,
            height: HEIGHT,
            fps: FPS,
            frames: FRAMES,
            columns: COLUMNS,
        }
    }
}

pub(crate) fn generate(root: &Path, args: &[String]) -> Result<(), String> {
    let (source, output) = options(
        args,
        "--source",
        Some(root.join("crates/tui/assets/welcome")),
    )?;
    export::generate(&source, &output).map_err(|error| error.to_string())
}

pub(crate) fn preview(args: &[String]) -> Result<(), String> {
    let (input, output) = options(args, "--input", None)?;
    preview::run(&input, &output).map_err(|error| error.to_string())
}

fn options(
    args: &[String],
    input_flag: &str,
    default_output: Option<PathBuf>,
) -> Result<(PathBuf, PathBuf), String> {
    let mut input = None;
    let mut output = default_output;
    let mut arguments = args.iter();
    while let Some(flag) = arguments.next() {
        if flag != input_flag && flag != "--output" {
            return Err(format!("unexpected argument `{flag}`"));
        }
        let path = arguments
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires a path"))?;
        if flag == input_flag {
            input = Some(PathBuf::from(path));
        } else {
            output = Some(PathBuf::from(path));
        }
    }
    Ok((
        input.ok_or_else(|| format!("{input_flag} is required"))?,
        output.ok_or("--output is required")?,
    ))
}

#[cfg(test)]
mod tests;
