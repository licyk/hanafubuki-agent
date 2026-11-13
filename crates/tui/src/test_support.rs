// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A terminal for tests: crossterm's backend writing into a vt100 screen;
//! and an external editor that edits no file interactively.

use std::fmt::{self};
use std::io::Write;
use std::io::{self};
use std::path::PathBuf;

use ratatui::backend::CrosstermBackend;

use ratatui::backend::Backend;
use ratatui::backend::ClearType;
use ratatui::backend::WindowSize;
use ratatui::buffer::Cell;
use ratatui::layout::Position;
use ratatui::layout::Size;

/// This wraps a CrosstermBackend and a vt100::Parser to mock
/// a "real" terminal.
///
/// Importantly, this wrapper avoids calling any crossterm methods
/// which write to stdout regardless of the writer. This includes:
/// - getting the terminal size
/// - getting the cursor position
pub struct VT100Backend {
    crossterm_backend: CrosstermBackend<vt100::Parser>,
}

impl VT100Backend {
    /// Creates a new `TestBackend` with the specified width and height.
    pub fn new(width: u16, height: u16) -> Self {
        ratatui::crossterm::style::force_color_output(true);
        Self {
            crossterm_backend: CrosstermBackend::new(vt100::Parser::new(height, width, 0)),
        }
    }

    pub fn vt100(&self) -> &vt100::Parser {
        self.crossterm_backend.writer()
    }

    /// The window resized, its screen cleared (the cursor left where it
    /// is): vt100 cuts a wide character at the new edge and then panics on
    /// it, where a real terminal reflows or cuts its rows. What the app
    /// draws after a resize is tested, not the terminal's own reflow.
    pub fn resize(&mut self, width: u16, height: u16) {
        let parser = self.crossterm_backend.writer_mut();
        parser.process(b"\x1b[2J");
        parser.screen_mut().set_size(height, width);
    }
}

impl Write for VT100Backend {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.crossterm_backend.writer_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.crossterm_backend.writer_mut().flush()
    }
}

impl fmt::Display for VT100Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.crossterm_backend.writer().screen().contents())
    }
}

impl Backend for VT100Backend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.crossterm_backend.draw(content)?;
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.crossterm_backend.hide_cursor()?;
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.crossterm_backend.show_cursor()?;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.vt100().screen().cursor_position().into())
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.crossterm_backend.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.crossterm_backend.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.crossterm_backend.clear_region(clear_type)
    }

    fn append_lines(&mut self, line_count: u16) -> io::Result<()> {
        self.crossterm_backend.append_lines(line_count)
    }

    fn size(&self) -> io::Result<Size> {
        let (rows, cols) = self.vt100().screen().size();
        Ok(Size::new(cols, rows))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.vt100().screen().size().into(),
            // Arbitrary size, we don't rely on this in testing.
            pixels: Size {
                width: 640,
                height: 480,
            },
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.crossterm_backend.writer_mut().flush()
    }

    fn scroll_region_up(&mut self, region: std::ops::Range<u16>, scroll_by: u16) -> io::Result<()> {
        self.crossterm_backend.scroll_region_up(region, scroll_by)
    }

    fn scroll_region_down(
        &mut self,
        region: std::ops::Range<u16>,
        scroll_by: u16,
    ) -> io::Result<()> {
        self.crossterm_backend.scroll_region_down(region, scroll_by)
    }
}

/// An "editor" for Ctrl+G's tests: a script that keeps the file it was
/// given as `seen.md`, replaces it with `result.md` and exits with its
/// code; `sh` runs it on Unix, and it is a `.cmd` on Windows.
pub(crate) struct FakeEditor {
    dir: tempfile::TempDir,
}

impl FakeEditor {
    pub(crate) fn new(result: &[u8], code: i32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("seen.md");
        let result_path = dir.path().join("result.md");
        std::fs::write(&result_path, result).unwrap();
        let script = if cfg!(windows) {
            format!(
                "@echo off\r\ncopy /y \"%~1\" \"{}\" >nul\r\ncopy /y \"{}\" \"%~1\" >nul\r\nexit /b {code}\r\n",
                seen.display(),
                result_path.display()
            )
        } else {
            format!(
                "cp \"$1\" '{}'\ncp '{}' \"$1\"\nexit {code}\n",
                seen.display(),
                result_path.display()
            )
        };
        let editor = Self { dir };
        std::fs::write(editor.script(), script).unwrap();
        editor
    }

    /// The command line `VISUAL` would hold.
    pub(crate) fn command_line(&self) -> String {
        if cfg!(windows) {
            format!("\"{}\"", self.script().display())
        } else {
            format!("sh '{}'", self.script().display())
        }
    }

    /// What the editor was given; `None` before it ran.
    pub(crate) fn seen(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("seen.md")).ok()
    }

    fn script(&self) -> PathBuf {
        let name = if cfg!(windows) { "edit.cmd" } else { "edit.sh" };
        self.dir.path().join(name)
    }
}
