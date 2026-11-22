// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use image::{DynamicImage, ImageDecoder, ImageFormat};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
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
use crate::history_cell::{raw_lines_from_source, HistoryCell};
use crate::terminal_hyperlinks::{visible_lines, HyperlinkLine};
use crate::transcript_view::TranscriptView;
use crate::transcript_view::ViewAction;

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
        crate::markdown::render_markdown_with_list_spacing(
            &self.source,
            Some(width as usize),
            self.parent.as_deref(),
            crate::markdown_render::ListSpacing::AfterMultiline,
        )
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
    pub(super) editing_search: bool,
    reader: TranscriptView,
    content: State,
    canonical: PathBuf,
    line: Option<Location>,
    link: Option<(usize, FileTarget)>,
    pressed_link: Option<(u16, u16)>,
    tx: AppEventSender,
    watch: tokio::task::AbortHandle,
}

enum State {
    Notice(String),
    Text {
        cells: [Arc<dyn HistoryCell>; 1],
        source: Arc<str>,
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
            pressed_link: None,
            reader: TranscriptView::default(),
            content: State::Notice("Loading…".into()),
            editing_search: false,
        }
    }

    pub(super) fn loaded(&mut self, result: Result<(PathBuf, DocumentContent), String>) {
        self.link = None;
        self.pressed_link = None;
        self.editing_search = false;
        self.content = match result {
            Err(error) => State::Notice(crate::text_formatting::strip_terminal_controls(&error)),
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
                let language =
                    (!matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx")
                        || self.line.is_some())
                    .then(|| ext.to_owned());
                let offset = if matches!(self.content, State::Text { .. }) {
                    self.reader.document_offset()
                } else {
                    Some(self.line.map_or(0, |loc| line_offset(&source, loc)))
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
                self.editing_search = false;
                State::Text { source, cells }
            }
        }
    }
    pub(super) fn go_to(&mut self, line: Location) {
        if self.line.replace(line).is_none() {
            if let State::Text { source, .. } = &self.content {
                let source = source.to_string();
                self.loaded(Ok((self.canonical.clone(), DocumentContent::Text(source))));
            }
        }
        if let State::Text { source, cells } = &self.content {
            self.reader
                .jump_to_document_offset(cells, line_offset(source, line));
        }
    }

    pub(super) fn paint(&mut self, area: Rect, buf: &mut Buffer) -> Option<ImagePlacement> {
        if area.is_empty() {
            return None;
        }
        match &mut self.content {
            State::Notice(text) => Paragraph::new(text.as_str())
                .wrap(ratatui::widgets::Wrap { trim: false })
                .render(area, buf),
            State::Text { cells, .. } => {
                if self.reader.advance_search(cells) {
                    self.tx
                        .send(AppEvent::ScheduleFrame(std::time::Instant::now()));
                }
                let footer = self
                    .reader
                    .footer(area.width)
                    .map(|f| f.lines)
                    .unwrap_or_else(|| {
                        vec![
                            if let Some((_, FileTarget::Document { path, .. })) = &self.link {
                                Line::from(format!("Enter opens {}", path.display())).dim()
                            } else {
                                Line::from("↑/↓ scroll · / find").dim()
                            },
                        ]
                    });
                let height = (footer.len() as u16).min(area.height);
                let body = Rect {
                    height: area.height - height,
                    ..area
                };
                self.reader.render(body, buf, cells);
                Paragraph::new(footer)
                    .render(Rect::new(area.x, body.bottom(), area.width, height), buf);
            }
            State::Image { preview, label } => {
                Line::from(label.as_str()).dim().render(area, buf);
                return preview.paint(
                    Rect::new(area.x, area.y + 1, area.width, area.height - 1),
                    buf,
                );
            }
        }
        None
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Option<FileTarget> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let State::Text { cells, .. } = &self.content else {
            return None;
        };
        if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter) {
            self.link = None;
        }
        if !self.reader.has_active_interaction() {
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.link = self.reader.step_document_link(
                        cells,
                        self.link.as_ref().map(|(at, _)| *at),
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
                        .and_then(|(_, target)| self.relative_link(target.clone()));
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
        self.editing_search = self.reader.has_active_interaction();
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
        self.editing_search = self.reader.has_active_interaction();
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

fn line_offset(source: &str, loc: Location) -> usize {
    let start = source
        .split_inclusive('\n')
        .take(loc.line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let line = source[start..].split('\n').next().unwrap_or_default();
    start
        + line
            .chars()
            .take(loc.column.unwrap_or(1).saturating_sub(1))
            .map(char::len_utf8)
            .sum::<usize>()
}

fn copy_action(tx: &AppEventSender, action: Option<ViewAction>) {
    if let Some(ViewAction::Copy(text) | ViewAction::CopyAndFollow(text)) = action {
        tx.send(AppEvent::Command(AppCommand::Copy(text)));
    }
}
