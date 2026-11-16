use std::fmt::Write;
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
struct Screen {
    width: u32,
    height: u32,
    cells: Vec<Cell>,
}

#[derive(Deserialize)]
struct Cell {
    symbol: String,
    color: Option<[u8; 3]>,
}

pub(super) fn run(input: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let screen: Screen = serde_json::from_slice(&fs::read(input)?)?;
    if screen.width == 0
        || screen.height == 0
        || u64::from(screen.width) * u64::from(screen.height) != screen.cells.len() as u64
    {
        return Err("terminal dimensions must be positive and match the number of cells".into());
    }
    fs::write(output, render(&screen))?;
    println!("Wrote colored terminal preview: {}", output.display());
    Ok(())
}

fn render(screen: &Screen) -> String {
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">\n<rect width=\"100%\" height=\"100%\" fill=\"#20232b\"/>\n<g font-family=\"monospace\" font-size=\"18\">\n",
        u64::from(screen.width) * 12, u64::from(screen.height) * 24
    );
    for (index, cell) in screen.cells.iter().enumerate() {
        let x = index as u64 % u64::from(screen.width) * 12;
        let y = index as u64 / u64::from(screen.width) * 24;
        let [r, g, b] = cell.color.unwrap_or([220, 225, 233]);
        let color = format!("#{r:02x}{g:02x}{b:02x}");
        match cell.symbol.chars().next() {
            Some(c @ '\u{2800}'..='\u{28ff}') => {
                for (dx, dy, bit) in [
                    (0, 0, 1),
                    (0, 1, 2),
                    (0, 2, 4),
                    (1, 0, 8),
                    (1, 1, 16),
                    (1, 2, 32),
                    (0, 3, 64),
                    (1, 3, 128),
                ] {
                    if (u32::from(c) - 0x2800) & bit != 0 {
                        writeln!(
                            svg,
                            "<circle cx=\"{}\" cy=\"{}\" r=\"2\" fill=\"{color}\"/>",
                            x + dx * 6 + 3,
                            y + dy * 6 + 3
                        )
                        .unwrap();
                    }
                }
            }
            Some(_) if !cell.symbol.trim().is_empty() => {
                let text = cell
                    .symbol
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                writeln!(
                    svg,
                    "<text x=\"{x}\" y=\"{}\" fill=\"{color}\">{text}</text>",
                    y + 19
                )
                .unwrap();
            }
            _ => {}
        }
    }
    svg.push_str("</g>\n</svg>\n");
    svg
}
