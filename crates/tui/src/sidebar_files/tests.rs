use super::*;
use crate::app_event::AppEventSender;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};

fn reader(path: &Path, bytes: Vec<u8>, location: Option<Location>) -> Document {
    let (tx, _) = std::sync::mpsc::channel();
    let task = tokio::spawn(std::future::pending::<()>());
    let mut doc = Document::new(
        path.into(),
        location,
        1,
        task.abort_handle(),
        AppEventSender::new(tx),
    );
    doc.loaded(Ok((
        path.into(),
        DocumentContent::decode(bytes, path).unwrap(),
    )));
    doc
}

#[tokio::test]
async fn linked_document_reflows_and_jumps_to_source_after_click_or_tab() {
    let path = std::env::temp_dir().join("nested/readme.md");
    let mut source = "[Sibling](other%20file.rs#L3C2)\r\n\r\n".to_string();
    for i in 3..32 {
        source.push_str(&format!("line {i}\r\n"));
    }
    let mut doc = reader(&path, source.into(), None);
    let area = Rect::new(2, 3, 48, 6);
    let mut buf = Buffer::empty(area);
    doc.paint(area, &mut buf, None, false);
    let row = buf
        .content
        .chunks(usize::from(area.width))
        .position(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Sibling")
        })
        .unwrap();
    let pointer = |kind| MouseEvent {
        kind,
        column: area.x + 1,
        row: area.y + row as u16,
        modifiers: KeyModifiers::NONE,
    };
    doc.mouse(pointer(MouseEventKind::Down(MouseButton::Left)));
    let target = FileTarget::Document {
        path: path.parent().unwrap().join("other file.rs"),
        line: Some(Location {
            line: 3,
            column: Some(2),
        }),
    };
    assert_eq!(
        doc.mouse(pointer(MouseEventKind::Up(MouseButton::Left))),
        Some(target.clone())
    );
    doc.key(KeyEvent::from(KeyCode::Tab));
    assert_eq!(doc.key(KeyEvent::from(KeyCode::Enter)), Some(target));
    doc.go_to(Some(Location {
        line: 20,
        column: None,
    }));
    let area = Rect::new(0, 0, 24, 4);
    let mut buf = Buffer::empty(area);
    doc.paint(area, &mut buf, None, false);
    let top = buf.content[..24]
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(top.contains("line 20"), "{top}");
    doc.go_to(None);
    let mut buf = Buffer::empty(area);
    doc.paint(area, &mut buf, None, false);
    assert!(painted_rows(&buf).starts_with(" Sibling"));
}

#[tokio::test]
async fn image_content_is_recognised_by_bytes_and_fits_the_panel_after_resize() {
    let mut png = std::io::Cursor::new(Vec::new());
    let image = ::image::RgbaImage::from_pixel(8, 4, ::image::Rgba([20, 130, 240, 255]));
    ::image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, ::image::ImageFormat::Png)
        .unwrap();
    let mut doc = reader(Path::new("no-extension"), png.into_inner(), None);
    for area in [Rect::new(3, 2, 30, 8), Rect::new(7, 1, 14, 4)] {
        let mut buf = Buffer::empty(area);
        let native = doc.paint(area, &mut buf, None, false);
        let label = buf.content[..area.width as usize]
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(label.contains("PNG · 8 × 4"), "{label}");
        assert!(
            native.is_some()
                || buf
                    .content
                    .iter()
                    .any(|c| c.symbol() == "▀" && c.fg == ratatui::style::Color::Rgb(20, 130, 240))
        );
    }
    assert!(DocumentContent::decode(vec![0, 1, 2], Path::new("binary")).is_err());
}

#[test]
fn history_file_links_decode_locations_and_skip_non_files_and_duplicates() {
    let mut seen = std::collections::HashSet::new();
    let mut entries = vec![];
    let root = std::env::temp_dir();
    let item = hanafubuki_agent::ConversationItem::UserMessage {
        images: vec![],
        text: "[doc](notes.md:8:2) [same](notes.md) [web](https://example.org/x) `src/other.rs`"
            .into(),
    };
    history_references(item, &root, &mut entries, &mut seen, |_| {
        unreachable!("text links do not invoke the patch parser")
    });
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].target,
        FileTarget::Document {
            path: root.join("notes.md"),
            line: Some(Location {
                line: 8,
                column: Some(2)
            })
        }
    );
    assert_eq!(
        entries[1].target,
        FileTarget::document(root.join("src/other.rs"))
    );
}

fn painted_rows(buf: &Buffer) -> String {
    buf.content
        .chunks(usize::from(buf.area.width))
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn source_reader_matches_sidebar_margins_gutter_scroll_arrows_and_search_caret() {
    let path = Path::new("sample.rs");
    let contents = (1..=18)
        .map(|n| format!("let value_{n} = {n};"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut doc = reader(
        path,
        contents.into(),
        Some(Location {
            line: 9,
            column: Some(5),
        }),
    );
    let area = Rect::new(3, 2, 54, 8);
    let mut buf = Buffer::empty(area);
    doc.paint(area, &mut buf, None, false);
    let source = painted_rows(&buf);
    let target_x = area.x + 1 + 3;
    assert_eq!(
        buf[(target_x, area.y)].modifier & ratatui::style::Modifier::REVERSED,
        ratatui::style::Modifier::REVERSED
    );
    assert!(!buf[(target_x + 4, area.y)]
        .modifier
        .contains(ratatui::style::Modifier::REVERSED));
    doc.key(KeyEvent::from(KeyCode::Esc));
    doc.key(KeyEvent::from(KeyCode::Char('/')));
    doc.paste("value");
    let mut buf = Buffer::empty(area);
    doc.paint(area, &mut buf, None, false);
    let caret = doc.cursor.expect("active search has a terminal cursor");
    assert_eq!(caret.x, area.x + 1 + 6 + 5);
    assert_eq!(caret.y, area.bottom() - 2);
    insta::assert_snapshot!(
        "sidebar_source_and_find",
        format!("{source}\n\n{}", painted_rows(&buf))
    );
}

#[tokio::test]
async fn markdown_loading_errors_and_image_footer_use_the_reader_body() {
    let path = Path::new("src/notes.md");
    let (tx, _) = std::sync::mpsc::channel();
    let task = tokio::spawn(std::future::pending::<()>());
    let mut doc = Document::new(
        path.into(),
        None,
        1,
        task.abort_handle(),
        AppEventSender::new(tx),
    );
    let area = Rect::new(0, 0, 38, 12);
    let paint = |doc: &mut Document| {
        let mut buf = Buffer::empty(area);
        doc.paint(area, &mut buf, None, false);
        painted_rows(&buf)
    };
    let opening = paint(&mut doc);
    assert!(opening.starts_with(" ◌ Opening…"));
    doc.loaded(Err("No such file or directory (os error 2)".into()));
    let error = paint(&mut doc);
    doc.loaded(Ok((
        path.into(),
        DocumentContent::Text("Read [the manual](manual.md) first.".into()),
    )));
    let markdown = paint(&mut doc);
    assert!(!markdown.contains("manual.md"));
    doc.key(KeyEvent::from(KeyCode::Tab));
    let mut buf = Buffer::empty(area);
    doc.paint(area, &mut buf, None, false);
    assert!(buf[(6, 0)]
        .modifier
        .contains(ratatui::style::Modifier::REVERSED));
    insta::assert_snapshot!(
        "sidebar_markdown_and_missing_file",
        format!("{markdown}\n\n{error}")
    );

    doc.loaded(Ok((
        path.into(),
        DocumentContent::Image {
            pixels: ::image::RgbaImage::new(3, 3),
            label: "PNG · 3 × 3".into(),
        },
    )));
    let image = paint(&mut doc);
    assert!(image.starts_with(" PNG · 3 × 3"));
    assert_eq!(image.lines().last(), Some(" ←/→ tabs · Esc back"));
    assert_eq!(image.lines().nth(10), Some(""));
}
