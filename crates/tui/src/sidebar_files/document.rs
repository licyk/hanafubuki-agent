// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use image::{DynamicImage, ImageDecoder, ImageFormat};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    layout::{Margin, Position, Rect},
    style::Stylize,
    text::Line,
    widgets::{Paragraph, Widget},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{image::Preview, FileTarget, ImagePlacement, Location};
use crate::app_event::{AppCommand, AppEvent, AppEventSender};
use crate::footer_hint::{shortcut, HintLine};
use crate::history_cell::{raw_lines_from_source, HistoryCell};
use crate::keymap::{self, Action};
use crate::terminal_hyperlinks::{visible_lines, HyperlinkLine};
use crate::transcript_view::TranscriptView;
use crate::transcript_view::{DocumentLink, ViewAction};
use std::time::{Duration, Instant};

pub(crate) enum DocumentContent {
    Text(String),
    Image {
        pixels: image::RgbaImage,
        label: String,
    },
}

impl DocumentContent {
    pub(crate) fn decode(bytes: Vec<u8>, path: &Path) -> Result<Self, String> {
        let image = image::ImageReader::new(Cursor::new(bytes.as_slice()))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        if let Some(
            format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP),
        ) = image.format()
        {
            let mut decoder = image.into_decoder().map_err(|e| e.to_string())?;
            let (w, h) = decoder.dimensions();
            if u64::from(w) * u64::from(h) > hanafubuki_agent::MAXIMUM_IMAGE_PIXELS {
                return Err("Image exceeds the preview pixel budget".into());
            }
            let orientation = decoder.orientation().map_err(|e| e.to_string())?;
            let mut raster = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
            raster.apply_orientation(orientation);
            let label = format!(
                "{} · {} × {}",
                format.extensions_str()[0].to_uppercase(),
                raster.width(),
                raster.height()
            );
            let pixels = if raster.width() > 768 || raster.height() > 768 {
                raster.thumbnail(768, 768)
            } else {
                raster
            }
            .into_rgba8();
            Ok(Self::Image { pixels, label })
        } else {
            // The reader lays out the complete source for wrapping, selection and search.
            // Codex's 1 MiB source budget bounds that layout separately from image transfers.
            if bytes.len() > 1024 * 1024 {
                return Err("Text preview exceeds 1 MiB".into());
            }
            let text = String::from_utf8(bytes).map_err(|_| {
                format!(
                    "{} is not a supported image or UTF-8 text file",
                    path.display()
                )
            })?;
            if text.contains('\0') {
                return Err("Binary file has no text preview".into());
            }
            Ok(Self::Text(text))
        }
    }
}

#[derive(Debug)]
struct FileCell {
    source: Arc<str>,
    language: Option<String>,
    parent: Option<String>,
}

impl HistoryCell for FileCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }
    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        if let Some(language) = &self.language {
            return crate::render::highlight::highlight_code_to_lines(&self.source, language)
                .into_iter()
                .map(HyperlinkLine::new)
                .collect();
        }
        crate::markdown_render::render_document_lines(&self.source, width, self.parent.as_deref())
    }
    fn retained_hyperlink_lines(&self, width: u16, _detailed: bool) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }
    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }
    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.source)
    }
}

pub(super) struct Document {
    pub(super) path: PathBuf,
    pub(super) request: u64,
    reader: TranscriptView,
    content: State,
    canonical: PathBuf,
    line: Option<Location>,
    link: Option<DocumentLink>,
    pub(super) cursor: Option<Position>,
    pressed_link: Option<(u16, u16)>,
    tx: AppEventSender,
    watch: tokio::task::AbortHandle,
}

enum State {
    Loading(Instant),
    Failed(String),
    Text {
        cells: [Arc<dyn HistoryCell>; 1],
        source: Arc<str>,
        starts: Vec<usize>,
        target: Option<(usize, usize)>,
    },
    Image {
        preview: Preview,
        label: String,
    },
}

impl Drop for Document {
    fn drop(&mut self) {
        self.watch.abort();
    }
}

impl Document {
    pub(super) fn new(
        path: PathBuf,
        line: Option<Location>,
        request: u64,
        watch: tokio::task::AbortHandle,
        tx: AppEventSender,
    ) -> Self {
        Self {
            path: path.clone(),
            canonical: path,
            line,
            request,
            watch,
            tx,
            link: None,
            cursor: None,
            pressed_link: None,
            reader: TranscriptView::default(),
            content: State::Loading(Instant::now()),
        }
    }

    pub(super) fn loaded(&mut self, result: Result<(PathBuf, DocumentContent), String>) {
        self.link = None;
        self.pressed_link = None;

        self.content = match result {
            Err(error) => State::Failed(crate::text_formatting::strip_terminal_controls(&error)),
            Ok((canonical, DocumentContent::Image { pixels, label })) => {
                self.canonical = canonical;
                State::Image {
                    preview: Preview::new(pixels),
                    label,
                }
            }
            Ok((canonical, DocumentContent::Text(source))) => {
                self.canonical = canonical;
                let source = source.replace("\r\n", "\n");
                let ext = self
                    .canonical
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let language = (!super::markdown_path(&self.path) || self.line.is_some())
                    .then(|| ext.to_owned());
                let starts = if language.is_some() {
                    source
                        .lines()
                        .scan(0, |offset, line| {
                            let start = *offset;
                            *offset += line.len() + 1;
                            Some(start)
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let target = self
                    .line
                    .and_then(|line| source_target(&source, &starts, line));
                let offset = if matches!(self.content, State::Text { .. }) {
                    self.reader.document_offset()
                } else {
                    Some(target.map_or(0, |(_, offset)| offset))
                };
                let source: Arc<str> = source.into();
                let cells: [Arc<dyn HistoryCell>; 1] = [Arc::new(FileCell {
                    source: source.clone(),
                    language,
                    parent: self
                        .canonical
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned()),
                })];
                self.reader = TranscriptView::default();
                if let Some(offset) = offset {
                    self.reader.jump_to_document_offset(&cells, offset)
                }

                State::Text {
                    source,
                    cells,
                    starts,
                    target,
                }
            }
        }
    }
    pub(super) fn go_to(&mut self, location: Option<Location>) {
        let located = self.line.is_some();
        self.line = location;
        self.link = None;
        let State::Text {
            source,
            cells,
            starts,
            target,
        } = &mut self.content
        else {
            return;
        };
        let ext = self
            .canonical
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "md" | "mdx" | "markdown") && located != location.is_some() {
            let text = source.to_string();
            self.content = State::Loading(Instant::now());
            self.loaded(Ok((self.canonical.clone(), DocumentContent::Text(text))));
        } else {
            *target = location.and_then(|line| source_target(source, starts, line));
            if let Some((_, offset)) = *target {
                self.reader = TranscriptView::default();
                self.reader.jump_to_document_offset(cells, offset);
            }
        }
    }

    pub(super) fn paint(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        resize_hint: Option<&str>,
        animations: bool,
    ) -> Option<ImagePlacement> {
        self.cursor = None;
        let body = area.inner(Margin::new(1, 0));
        if body.is_empty() {
            return None;
        }
        let bottom = Rect::new(body.x, body.bottom() - 1, body.width, 1);
        match &mut self.content {
            State::Loading(started) => {
                let spinner = if animations {
                    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                    self.tx.send(AppEvent::ScheduleFrame(
                        Instant::now() + Duration::from_millis(100),
                    ));
                    FRAMES[((started.elapsed().as_millis() / 100) % 10) as usize]
                } else {
                    "◌"
                };
                Line::from(vec![
                    spinner.fg(crate::style::accent_color()),
                    " ".into(),
                    "Opening…".dim(),
                ])
                .render(body, buf);
            }
            State::Failed(error) => {
                let mut lines = error_lines(&self.path, error, body.width);
                if lines.len() + 2 < usize::from(body.height) {
                    lines.insert(0, Line::default());
                    if resize_hint.is_none() {
                        shortcut("Esc", "back").render(bottom, buf);
                    }
                }
                Paragraph::new(lines).render(body, buf);
            }
            State::Image { preview, label } => {
                Line::from(label.as_str()).dim().render(body, buf);
                if body.height > 1 {
                    if let Some(hint) = resize_hint {
                        Line::from(hint).dim().render(bottom, buf);
                    } else {
                        HintLine::new(
                            [shortcut("←/→", "tabs"), shortcut("Esc", "back")],
                            body.width,
                        )
                        .line
                        .render(bottom, buf);
                    }
                }
                let rows = body.height.saturating_sub(2 + u16::from(body.height > 3));
                return preview.paint(Rect::new(body.x, body.y + 1, body.width, rows), buf);
            }
            State::Text {
                cells,
                starts,
                target,
                ..
            } => {
                if self.reader.tick_selection(cells) {
                    self.tx.send(AppEvent::ScheduleFrame(
                        Instant::now() + Duration::from_millis(45),
                    ));
                }
                if self.reader.advance_search(cells) {
                    self.tx.send(AppEvent::ScheduleFrame(Instant::now()));
                }
                let active = self.reader.has_active_interaction();
                let footer = active.then(|| self.reader.footer(body.width)).flatten();
                let (lines, cursor) = if let Some(hint) = resize_hint {
                    (vec![Line::from(hint).dim()], None)
                } else if let Some(footer) = footer {
                    (footer.lines, footer.cursor_column)
                } else {
                    (vec![reader_hints(body.width, target.is_some())], None)
                };
                let height = (lines.len() as u16).min(body.height.saturating_sub(1));
                let gap = u16::from(height > 0 && body.height - height > 1);
                let content = Rect {
                    height: body.height - height - gap,
                    ..body
                };
                let viewport = Rect {
                    width: content.width - u16::from(content.width > 2),
                    ..content
                };
                self.reader.render_document(
                    viewport,
                    buf,
                    cells,
                    starts,
                    target.filter(|_| !active),
                    self.link.as_ref(),
                );
                self.reader
                    .paint_document_scroll_indicators(content, buf, cells);
                if height > 0 {
                    let row = content.bottom() + gap;
                    Paragraph::new(lines).render(Rect::new(body.x, row, body.width, height), buf);
                    self.cursor = cursor.map(|c| Position::new(body.x + c, row));
                }
                return None;
            }
        }
        if let Some(hint) = resize_hint {
            Line::from(hint).dim().render(bottom, buf);
        }
        None
    }

    pub(super) fn captures_key(&self, key: KeyEvent) -> bool {
        matches!(self.content, State::Text { .. }) && self.reader.has_active_interaction()
            || (key.code == KeyCode::Esc
                && (self.link.is_some()
                    || matches!(
                        self.content,
                        State::Text {
                            target: Some(_),
                            ..
                        }
                    )))
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Option<FileTarget> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let State::Text { cells, .. } = &self.content else {
            return None;
        };
        if key.code == KeyCode::Esc && !self.reader.has_active_interaction() {
            if self.link.take().is_none() {
                if let State::Text { target, .. } = &mut self.content {
                    *target = None;
                }
            }
            return None;
        }
        if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter) {
            self.link = None;
        }
        if !self.reader.has_active_interaction() {
            if keymap::matches(Action::SelectTranscript, &key) {
                self.reader.begin_select_mode(cells);

                return None;
            }
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.link = self.reader.step_document_link(
                        cells,
                        self.link.as_ref().map(|link| link.index),
                        key.code == KeyCode::Tab
                            && !key
                                .modifiers
                                .contains(ratatui::crossterm::event::KeyModifiers::SHIFT),
                    );
                    return None;
                }
                KeyCode::Enter => {
                    return self
                        .link
                        .as_ref()
                        .and_then(|link| self.relative_link(link.target.clone()));
                }
                KeyCode::Char('/') => self.reader.begin_search(),
                KeyCode::Up | KeyCode::Char('k') => self.reader.scroll(cells, -1),
                KeyCode::Down | KeyCode::Char('j') => self.reader.scroll(cells, 1),
                KeyCode::Home => self.reader.jump_to_beginning(cells),
                KeyCode::End => self.reader.jump_to_latest(),
                _ => {
                    copy_action(&self.tx, self.reader.handle_key(key, cells));
                }
            }
        } else {
            copy_action(&self.tx, self.reader.handle_key(key, cells));
        }

        None
    }

    pub(super) fn mouse(&mut self, mouse: MouseEvent) -> Option<FileTarget> {
        let State::Text { cells, .. } = &self.content else {
            return None;
        };
        let point = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left)
                if !self.reader.has_active_interaction()
                    && self.reader.file_link_at(point.0, point.1).is_some() =>
            {
                self.pressed_link = Some(point);
                return None;
            }
            MouseEventKind::Up(MouseButton::Left) if self.pressed_link.take() == Some(point) => {
                if let Some(target) = self.reader.file_link_at(point.0, point.1) {
                    return self.relative_link(target);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((column, row)) = self.pressed_link.take() {
                    self.reader.handle_mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column,
                            row,
                            ..mouse
                        },
                        cells,
                    );
                }
            }
            _ => {}
        }
        copy_action(&self.tx, self.reader.handle_mouse(mouse, cells));

        None
    }
    pub(super) fn canonical_key(&self) -> hanafubuki_agent::PathKey {
        hanafubuki_agent::PathKey::of(&self.canonical)
    }

    fn relative_link(&self, target: FileTarget) -> Option<FileTarget> {
        let FileTarget::Document { path, line } = target else {
            return None;
        };
        Some(FileTarget::Document {
            path: self.canonical.parent()?.join(path),
            line,
        })
    }
    pub(super) fn paste(&mut self, text: &str) {
        self.reader.paste_search(text);
    }
}

fn source_target(source: &str, starts: &[usize], loc: Location) -> Option<(usize, usize)> {
    use unicode_segmentation::UnicodeSegmentation;
    let index = loc
        .line
        .saturating_sub(1)
        .min(starts.len().saturating_sub(1));
    let start = *starts.get(index)?;
    let line = source[start..].split('\n').next()?;
    let byte = line
        .char_indices()
        .nth(loc.column.unwrap_or(1).saturating_sub(1))
        .map_or(line.len(), |(n, _)| n);
    let byte = if byte < line.len() {
        line.grapheme_indices(true)
            .take_while(|(n, _)| *n <= byte)
            .last()
            .map_or(0, |(n, _)| n)
    } else {
        byte
    };
    Some((index + 1, start + byte))
}

fn reader_hints(width: u16, target: bool) -> Line<'static> {
    let keys = keymap::current();
    let first = |action| keys.primary_label(action);
    HintLine::new(
        [
            Some(shortcut("↑/↓", "read")),
            Some(shortcut(
                first(Action::FindTranscript).as_deref().unwrap_or("/"),
                "find",
            )),
            Some(shortcut("Tab", "link")),
            first(Action::SelectTranscript).map(|key| shortcut(&key, "select")),
            Some(shortcut(
                "Esc",
                if target { "clear highlight" } else { "back" },
            )),
        ]
        .into_iter()
        .flatten(),
        width,
    )
    .line
}

fn error_lines(path: &Path, error: &str, width: u16) -> Vec<Line<'static>> {
    let reason = error
        .split_once(" (os error ")
        .map_or(error, |(reason, _)| reason);
    // Resolution errors include the attempted path; a read error carries the OS reason directly.
    let reason = if reason.starts_with("resolve filesystem target: path does not exist: ") {
        "No such file or directory"
    } else {
        reason.rsplit(": ").next().unwrap_or(reason)
    };
    let (heading, hint) = match reason {
        "No such file or directory"
        | "The system cannot find the file specified."
        | "The system cannot find the path specified." => (
            "File not found".yellow().bold(),
            "This file may have moved since the link was added.",
        ),
        "Permission denied" | "Operation not permitted" | "Access is denied." => (
            "Permission denied".red().bold(),
            "Check your read access, then reopen the link.",
        ),
        _ => ("Could not open file".red().bold(), error),
    };
    let path = crate::text_formatting::strip_terminal_controls(&path.to_string_lossy());
    let (dir, name) = path
        .rfind(['/', '\\'])
        .map_or(("", path.as_str()), |end| path.split_at(end + 1));
    let options = textwrap::Options::new(usize::from(width))
        .word_splitter(textwrap::WordSplitter::NoHyphenation);
    let mut lines = vec![Line::from(heading), Line::default()];
    lines.extend(
        textwrap::wrap(name, &options)
            .into_iter()
            .map(|s| Line::from(s.into_owned()).bold()),
    );
    if !dir.is_empty() {
        let path_options = options
            .clone()
            .word_separator(textwrap::WordSeparator::Custom(|s| {
                Box::new(
                    s.split_inclusive(['/', '\\'])
                        .map(textwrap::core::Word::from),
                )
            }));
        lines.extend(
            textwrap::wrap(&format!("in {dir}"), path_options)
                .into_iter()
                .map(|s| Line::from(s.into_owned()).dim()),
        );
    }
    lines.push(Line::default());
    lines.extend(
        textwrap::wrap(hint, options)
            .into_iter()
            .map(|s| Line::from(s.into_owned())),
    );
    lines
}

fn copy_action(tx: &AppEventSender, action: Option<ViewAction>) {
    if let Some(ViewAction::Copy(text) | ViewAction::CopyAndFollow(text)) = action {
        tx.send(AppEvent::Command(AppCommand::Copy(text)));
    }
}
