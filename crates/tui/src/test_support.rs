// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A terminal for tests: crossterm's backend writing into a vt100 screen,
//! the bytes it was given kept; the OSC 8 links in those bytes; an
//! external editor that edits no file interactively; and patches as the
//! applier reads them.

use std::fmt::{self};
use std::io::Write;
use std::io::{self};
use std::path::PathBuf;

use hanafubuki_agent::{
    AgentPatchFile, AgentPatchHunkStart, AgentPatchLine, AgentPatchLineKind, AgentPatchOperation,
    AgentPatchPreview,
};
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
    crossterm_backend: CrosstermBackend<Recorder>,
}

/// The vt100 screen, and every byte written to it since the last
/// [`VT100Backend::take_written`]: vt100 ignores OSC 8, so the bytes show
/// the hyperlinks.
struct Recorder {
    parser: vt100::Parser,
    written: Vec<u8>,
}

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        self.parser.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.parser.flush()
    }
}

impl VT100Backend {
    /// Creates a new `TestBackend` with the specified width and height.
    pub fn new(width: u16, height: u16) -> Self {
        Self::with_scrollback(width, height, 0)
    }

    /// A screen that keeps the last `rows` rows scrolled off its top.
    pub fn with_scrollback(width: u16, height: u16, rows: usize) -> Self {
        ratatui::crossterm::style::force_color_output(true);
        Self {
            crossterm_backend: CrosstermBackend::new(Recorder {
                parser: vt100::Parser::new(height, width, rows),
                written: Vec::new(),
            }),
        }
    }

    pub fn vt100_mut(&mut self) -> &mut vt100::Parser {
        &mut self.crossterm_backend.writer_mut().parser
    }

    pub fn vt100(&self) -> &vt100::Parser {
        &self.crossterm_backend.writer().parser
    }

    /// The bytes written since the last call, as text.
    pub(crate) fn take_written(&mut self) -> String {
        let written = std::mem::take(&mut self.crossterm_backend.writer_mut().written);
        String::from_utf8_lossy(&written).into_owned()
    }

    /// The window resized, its screen cleared (the cursor left where it
    /// is): vt100 cuts a wide character at the new edge and then panics on
    /// it, where a real terminal reflows or cuts its rows. What the app
    /// draws after a resize is tested, not the terminal's own reflow.
    pub fn resize(&mut self, width: u16, height: u16) {
        let parser = &mut self.crossterm_backend.writer_mut().parser;
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
        write!(f, "{}", self.vt100().screen().contents())
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

/// The OSC 8 links `written` opens, in order: each destination and the text
/// printed while it was open. Panics when a link is still open at a cursor
/// move, a line feed or the end, or is closed without being open: a link
/// is opened and closed on one row.
pub(crate) fn osc8_links(written: &str) -> Vec<(String, String)> {
    let mut links: Vec<(String, String)> = Vec::new();
    let mut open = false;
    let mut rest = written;
    while let Some(ch) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("\x1b]8;;") {
            let (destination, after) = after.split_once('\x07').expect("an OSC 8 ends with BEL");
            if destination.is_empty() {
                assert!(open, "a link closed that was not open: {written:?}");
                open = false;
            } else {
                assert!(!open, "a link opened inside another: {written:?}");
                links.push((destination.to_string(), String::new()));
                open = true;
            }
            rest = after;
            continue;
        }
        if let Some(after) = rest.strip_prefix("\x1b[") {
            let end = after
                .find(|c: char| ('\x40'..='\x7e').contains(&c))
                .expect("a CSI ends with its final byte");
            let final_byte = after.as_bytes()[end];
            assert!(
                !(open && matches!(final_byte, b'H' | b'r' | b'J' | b'K')),
                "a link open across `CSI {}`: {written:?}",
                &after[..=end]
            );
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('\x1b') {
            assert!(!open, "a link open across an escape: {written:?}");
            rest = &after[after.chars().next().map_or(0, char::len_utf8)..];
            continue;
        }
        assert!(
            !(open && (ch == '\n' || ch == '\r')),
            "a link open across a line: {written:?}"
        );
        if open {
            if let Some((_, text)) = links.last_mut() {
                text.push(ch);
            }
        }
        rest = &rest[ch.len_utf8()..];
    }
    assert!(!open, "a link left open: {written:?}");
    links
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

// Patches as the transcript shows them (Codex's diff tests' files).

/// A file of a patch from a diff-like body: `@@ OLD NEW` starts a chunk
/// (its starts, `@@` alone for none), then ` `, `+` and `-` rows.
pub(crate) fn patch_file(
    operation: &str,
    path: &str,
    moved_to: Option<&str>,
    body: &str,
) -> AgentPatchFile {
    let operation = match operation {
        "add" => AgentPatchOperation::Add,
        "delete" => AgentPatchOperation::Delete,
        _ => AgentPatchOperation::Update,
    };
    let mut lines = Vec::new();
    let mut hunk_starts = Vec::new();
    let (mut added, mut removed) = (0, 0);
    for row in body.lines() {
        let (kind, text) = match row.split_at(row.len().min(1)) {
            ("@", rest) => {
                let starts: Vec<u64> = rest
                    .trim_start_matches('@')
                    .split_whitespace()
                    .map(|number| number.parse().unwrap())
                    .collect();
                if let [old_line, new_line] = starts[..] {
                    hunk_starts.push(AgentPatchHunkStart { old_line, new_line });
                }
                (AgentPatchLineKind::Hunk, "")
            }
            ("+", text) => {
                added += 1;
                (AgentPatchLineKind::Added, text)
            }
            ("-", text) => {
                removed += 1;
                (AgentPatchLineKind::Removed, text)
            }
            (" ", text) => (AgentPatchLineKind::Context, text),
            ("", _) => (AgentPatchLineKind::Context, ""),
            ("*", _) => (AgentPatchLineKind::Marker, row),
            (_, _) => panic!("row {row:?}"),
        };
        lines.push(AgentPatchLine {
            kind,
            text: text.to_string(),
        });
    }
    AgentPatchFile {
        operation,
        path: path.to_string(),
        moved_to: moved_to.map(str::to_string),
        lines,
        added,
        removed,
        omitted_lines: 0,
        hunk_starts,
    }
}

pub(crate) fn patch_preview(files: Vec<AgentPatchFile>) -> AgentPatchPreview {
    AgentPatchPreview { files, error: None }
}

/// Codex's `apply_update_block`: `line two` changed in a file of three.
pub(crate) fn update_block() -> AgentPatchPreview {
    patch_preview(vec![patch_file(
        "update",
        "example.txt",
        None,
        "@@ 1 1\n line one\n-line two\n+line two changed\n line three",
    )])
}

/// Codex's diff gallery: updates (one renamed), additions, deletions.
pub(crate) fn diff_gallery() -> AgentPatchPreview {
    patch_preview(vec![
        patch_file(
            "add",
            "assets/banner.txt",
            None,
            "+HEADER\tVALUE\n+rocket\t🚀\n+city\t東京",
        ),
        patch_file(
            "add",
            "examples/new_sample.rs",
            None,
            "+pub fn greet(name: &str) {\n+    println!(\"Hello, {name}!\");\n+}",
        ),
        patch_file("delete", "legacy/old_script.py", None, ""),
        patch_file(
            "update",
            "scripts/calc.txt",
            Some("scripts/calc.py"),
            "@@ 1 1\n def add(a, b):\n-\treturn a + b\n+\treturn a + b + 42\n \n print(add(1, 2))",
        ),
        patch_file(
            "update",
            "src/lib.rs",
            None,
            "@@ 1 1\n fn greet(name: &str) {\n-    println!(\"hello\");\n-    println!(\"bye\");\n+    println!(\"hello {name}\");\n+    println!(\"emoji: 🚀✨ and CJK: 你好世界\");\n }",
        ),
        patch_file("delete", "tmp/obsolete.log", None, ""),
    ])
}
