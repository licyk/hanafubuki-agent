// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::cell::Cell;

use base64::Engine;

use super::*;

#[test]
fn osc52_encoding_round_trips() {
    let text = "# Hello\n\n```rust\nfn main() {}\n```\n";
    let sequence = osc52_sequence(text).unwrap();
    let encoded = sequence
        .strip_prefix("\u{1b}]52;c;")
        .and_then(|rest| rest.strip_suffix('\u{7}'))
        .unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(decoded, text.as_bytes());
    assert_eq!(
        osc52_sequence("hello").unwrap(),
        "\u{1b}]52;c;aGVsbG8=\u{7}"
    );
}

#[test]
fn osc52_refuses_a_payload_over_its_limit() {
    let text = "x".repeat(OSC52_MAX_RAW_BYTES + 1);
    assert_eq!(
        osc52_sequence(&text),
        Err(format!(
            "OSC 52 payload too large ({} bytes; max {OSC52_MAX_RAW_BYTES})",
            OSC52_MAX_RAW_BYTES + 1
        ))
    );
}

#[test]
fn the_escape_is_written_verbatim() {
    let mut output = Vec::new();
    write_osc52_to_writer(&mut output, "\u{1b}]52;c;aGVsbG8=\u{7}").unwrap();
    assert_eq!(output, b"\x1b]52;c;aGVsbG8=\x07");
}

#[test]
fn a_native_copy_is_confirmed_and_the_terminal_is_left_alone_locally() {
    let terminal_calls = Cell::new(0);
    let status = copy_with(
        "text",
        false,
        |_| Ok(()),
        |_| {
            terminal_calls.set(terminal_calls.get() + 1);
            Ok(())
        },
    );
    assert_eq!(status, Ok(CopyStatus::Confirmed));
    assert_eq!(terminal_calls.get(), 0);
}

#[test]
fn a_failed_native_copy_falls_back_to_the_terminal_unconfirmed() {
    let status = copy_with("text", false, |_| Err("no display".into()), |_| Ok(()));
    assert_eq!(status, Ok(CopyStatus::Unconfirmed));
    let error = copy_with(
        "text",
        false,
        |_| Err("no display".into()),
        |_| Err("no tty".into()),
    )
    .unwrap_err();
    assert_eq!(
        error,
        "native clipboard: no display; terminal clipboard: no tty"
    );
}

#[test]
fn over_ssh_the_terminal_is_asked_once_even_after_a_native_copy() {
    let terminal_calls = Cell::new(0);
    let terminal = |_: &str| {
        terminal_calls.set(terminal_calls.get() + 1);
        Ok(())
    };
    assert_eq!(
        copy_with("text", true, |_| Ok(()), terminal),
        Ok(CopyStatus::Confirmed)
    );
    assert_eq!(
        copy_with("text", true, |_| Err("remote".into()), terminal),
        Ok(CopyStatus::Unconfirmed)
    );
    assert_eq!(terminal_calls.get(), 2);
}

#[test]
fn nothing_is_copied_from_an_empty_selection() {
    let status = copy_with("", true, |_| panic!("native"), |_| panic!("terminal"));
    assert!(status.is_err());
}
