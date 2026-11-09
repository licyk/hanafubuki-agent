use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::*;
use crate::render::renderable::Renderable;

const WIDTH: u16 = 80;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(character: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
}

fn type_text(form: &mut FormView, text: &str) {
    for character in text.chars() {
        assert_eq!(
            form.handle_key(key(KeyCode::Char(character))),
            FormEvent::None
        );
    }
}

fn screen(form: &FormView) -> Vec<String> {
    let area = Rect::new(0, 0, WIDTH, form.desired_height(WIDTH));
    let mut buffer = Buffer::empty(area);
    form.render(area, &mut buffer);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn line_with(form: &FormView, label: &str) -> String {
    screen(form)
        .into_iter()
        .find(|line| line.contains(label))
        .unwrap_or_else(|| panic!("no line with {label:?}:\n{}", screen(form).join("\n")))
}

/// A name, a token, a choice and an action.
fn provider_like() -> FormView {
    FormView::new(
        "Add a Provider",
        vec![
            FormItem::text("name", "Name", "").placeholder("My gateway"),
            FormItem::text("token", "API token", "").secret(),
            FormItem::choice("protocol", "Protocol", &["responses", "chat"], 0),
            FormItem::toggle("enabled", "Enabled", true),
            FormItem::action("save", "Save"),
        ],
    )
}

#[test]
fn a_secret_shows_dots_with_only_its_last_character_while_typed() {
    let mut form = provider_like();
    form.handle_key(key(KeyCode::Tab));
    type_text(&mut form, "sk-secret");
    let focused = line_with(&form, "API token");
    assert!(focused.ends_with("••••••••t"), "{focused}");
    assert!(!focused.contains("sk-"), "{focused}");
    form.handle_key(key(KeyCode::Tab));
    let unfocused = line_with(&form, "API token");
    assert!(unfocused.ends_with("•••••••••"), "{unfocused}");
    // The value read is the one typed.
    assert_eq!(form.text("token"), "sk-secret");
}

#[test]
fn text_killed_in_a_secret_cannot_be_yanked_in_another_field() {
    let mut form = provider_like();
    form.handle_key(key(KeyCode::Tab));
    type_text(&mut form, "sk-secret");
    form.handle_key(ctrl('u'));
    assert_eq!(form.text("token"), "");
    form.handle_key(key(KeyCode::Up));
    form.handle_key(ctrl('y'));
    assert_eq!(form.text("name"), "");
    // The field's own kill buffer still has it.
    form.handle_key(key(KeyCode::Down));
    form.handle_key(ctrl('y'));
    assert_eq!(form.text("token"), "sk-secret");
}

#[test]
fn tab_shift_tab_and_the_arrows_move_between_the_stops_and_wrap() {
    let mut form = provider_like();
    assert!(line_with(&form, "Name").starts_with("› "));
    form.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(line_with(&form, "Save").starts_with("› "));
    form.handle_key(key(KeyCode::Down));
    assert!(line_with(&form, "Name").starts_with("› "));
    // Enter moves on from a field, and runs the action.
    form.handle_key(key(KeyCode::Enter));
    form.handle_key(key(KeyCode::Enter));
    assert!(line_with(&form, "Protocol").starts_with("› "));
    form.handle_key(key(KeyCode::Right));
    assert_eq!(form.choice("protocol"), 1);
    form.handle_key(key(KeyCode::Right));
    assert_eq!(form.choice("protocol"), 0);
    form.handle_key(key(KeyCode::Tab));
    form.handle_key(key(KeyCode::Char(' ')));
    assert!(!form.toggle("enabled"));
    form.handle_key(key(KeyCode::Tab));
    assert_eq!(
        form.handle_key(key(KeyCode::Enter)),
        FormEvent::Action("save")
    );
    assert_eq!(form.handle_key(key(KeyCode::Esc)), FormEvent::Cancel);
}

#[test]
fn a_paste_goes_to_the_focused_field_without_line_breaks() {
    let mut form = provider_like();
    assert!(form.handle_paste("first\r\nsecond"));
    assert_eq!(form.text("name"), "firstsecond");
    form.handle_key(key(KeyCode::Tab));
    form.handle_key(key(KeyCode::Tab));
    assert!(!form.handle_paste("ignored"), "a choice takes no text");
}

#[test]
fn an_error_is_shown_under_its_field_which_takes_the_focus() {
    let mut form = FormView::new(
        "Server",
        vec![
            FormItem::text("id", "Id", ""),
            FormItem::text("timeout", "Timeout", "x").advanced(),
            FormItem::action("save", "Save"),
        ],
    );
    assert!(!screen(&form).join("\n").contains("Timeout"));
    form.set_error("timeout", "Enter a whole number.");
    assert_eq!(form.error("timeout"), Some("Enter a whole number."));
    let lines = screen(&form);
    let at = lines
        .iter()
        .position(|line| line.contains("Timeout"))
        .expect("the advanced part opens");
    assert!(lines[at].starts_with("› "), "{lines:?}");
    assert!(lines[at + 1].contains("Enter a whole number."), "{lines:?}");
    form.clear_errors();
    assert_eq!(form.error("timeout"), None);
}

#[test]
fn a_field_shown_for_one_choice_leaves_the_stops_for_another() {
    let mut form = FormView::new(
        "Server",
        vec![
            FormItem::choice("transport", "Transport", &["stdio", "http"], 0),
            FormItem::text("program", "Program", "").shown_if("transport", 0),
            FormItem::text("url", "URL", "").shown_if("transport", 1),
        ],
    );
    let shown = screen(&form).join("\n");
    assert!(
        shown.contains("Program") && !shown.contains("URL"),
        "{shown}"
    );
    form.handle_key(key(KeyCode::Right));
    let shown = screen(&form).join("\n");
    assert!(
        !shown.contains("Program") && shown.contains("URL"),
        "{shown}"
    );
    form.handle_key(key(KeyCode::Tab));
    type_text(&mut form, "https://example.com");
    assert_eq!(form.text("url"), "https://example.com");
    assert_eq!(form.text("program"), "");
}

#[test]
fn rows_are_added_marked_secret_and_read_without_the_nameless_ones() {
    let mut form = FormView::new(
        "Server",
        vec![
            FormItem::rows(
                "env",
                "Environment",
                vec![RowValue {
                    name: "MODE".into(),
                    value: "fast".into(),
                    secret: false,
                }],
                "Add a variable",
            )
            .with_secret_column(),
            FormItem::action("save", "Save"),
        ],
    );
    // name, value, secret switch, then "Add a variable".
    for _ in 0..3 {
        form.handle_key(key(KeyCode::Tab));
    }
    form.handle_key(key(KeyCode::Enter));
    type_text(&mut form, "API_KEY");
    form.handle_key(key(KeyCode::Tab));
    type_text(&mut form, "hunter2");
    form.handle_key(key(KeyCode::Tab));
    form.handle_key(key(KeyCode::Char(' ')));
    let row = line_with(&form, "API_KEY");
    assert!(
        row.contains("•••••••") && row.ends_with("[secret]"),
        "{row}"
    );
    // A row whose name is cleared is dropped.
    form.handle_key(key(KeyCode::Tab));
    form.handle_key(key(KeyCode::Enter));
    type_text(&mut form, "  ");
    assert_eq!(
        form.rows("env"),
        vec![
            RowValue {
                name: "MODE".into(),
                value: "fast".into(),
                secret: false,
            },
            RowValue {
                name: "API_KEY".into(),
                value: "hunter2".into(),
                secret: true,
            },
        ]
    );
}

#[test]
fn the_cursor_is_in_the_focused_field_and_the_status_is_shown() {
    let mut form = provider_like();
    type_text(&mut form, "abc");
    let area = Rect::new(0, 0, WIDTH, form.desired_height(WIDTH));
    let (x, y) = form.cursor_pos(area).unwrap();
    let lines = screen(&form);
    assert!(lines[usize::from(y)].contains("Name"), "{lines:?}");
    assert_eq!(usize::from(x), lines[usize::from(y)].chars().count());
    form.set_status(vec![Line::from("✓ The Provider lists 2 models.")]);
    assert!(screen(&form).join("\n").contains("lists 2 models"));
}
