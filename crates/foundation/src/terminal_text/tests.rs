use super::{complete_prefix_len, render_plain_text, MAX_HELD_SEQUENCE_BYTES};

#[test]
fn colour_and_style_sequences_are_removed() {
    assert_eq!(
        render_plain_text(b"\x1b[1;31merror\x1b[0m: \x1b[38;5;208mwarm\x1b[38;2;1;2;3mrgb\x1b[m\n"),
        "error: warmrgb\n"
    );
}

#[test]
fn titles_and_hyperlinks_keep_only_their_visible_text() {
    assert_eq!(
        render_plain_text(
            b"\x1b]0;window title\x07\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ end"
        ),
        "link end"
    );
}

#[test]
fn device_and_application_strings_are_removed() {
    assert_eq!(
        render_plain_text(b"a\x1bP1$r0m\x1b\\b\x1b_payload\x1b\\c\x1b^private\x1b\\d"),
        "abcd"
    );
}

#[test]
fn charset_and_keypad_escapes_leave_no_stray_final_byte() {
    assert_eq!(render_plain_text(b"\x1b(B\x1b=text\x1b>\x1b)0"), "text");
}

#[test]
fn private_mode_sequences_are_removed() {
    assert_eq!(
        render_plain_text(b"\x1b[?25l\x1b[?2004hprompt\x1b[?25h"),
        "prompt"
    );
}

#[test]
fn carriage_return_progress_keeps_only_the_final_state() {
    assert_eq!(
        render_plain_text(b"  10%|#         |\r  55%|#####     |\r 100%|##########|\ndone\n"),
        " 100%|##########|\ndone\n"
    );
}

#[test]
fn erase_line_clears_a_longer_previous_state() {
    assert_eq!(
        render_plain_text(b"Downloading a long name\r\x1b[2KDone\n"),
        "Done\n"
    );
    assert_eq!(
        render_plain_text(b"Downloading a long name\rDone\x1b[K\n"),
        "Done\n"
    );
    assert_eq!(render_plain_text(b"abcdef\x1b[3D\x1b[1K\n"), "    ef\n");
}

#[test]
fn windows_line_endings_become_newlines() {
    assert_eq!(render_plain_text(b"one\r\ntwo\r\n"), "one\ntwo\n");
}

#[test]
fn backspace_and_horizontal_cursor_movement_overwrite_in_place() {
    assert_eq!(render_plain_text(b"abc\x08\x08XY"), "aXY");
    assert_eq!(render_plain_text(b"abcdef\x1b[4D12"), "ab12ef");
    assert_eq!(render_plain_text(b"ab\x1b[3Ccd"), "ab   cd");
    assert_eq!(render_plain_text(b"abcdef\x1b[2GX"), "aXcdef");
}

#[test]
fn tabs_are_kept_and_other_control_characters_dropped() {
    assert_eq!(render_plain_text(b"a\tb\x00\x07\x7fc"), "a\tbc");
}

#[test]
fn non_ascii_text_and_invalid_bytes_survive_as_text() {
    assert_eq!(
        render_plain_text("\x1b[32m模型\x1b[0m ✓".as_bytes()),
        "模型 ✓"
    );
    assert_eq!(render_plain_text(b"a\xffb"), "a\u{fffd}b");
}

#[test]
fn a_sequence_cut_off_by_the_end_of_input_is_dropped() {
    assert_eq!(render_plain_text(b"text\x1b"), "text");
    assert_eq!(render_plain_text(b"text\x1b[38;5"), "text");
    assert_eq!(render_plain_text(b"text\x1b]0;title"), "text");
}

#[test]
fn a_malformed_sequence_cannot_swallow_the_following_line() {
    assert_eq!(render_plain_text(b"a\x1b[12\nnext\n"), "a\nnext\n");
}

#[test]
fn a_forged_cursor_count_cannot_grow_a_line_without_bound() {
    let rendered = render_plain_text(b"\x1b[99999999999999999999Ca\x1b[999999999Cb");
    assert!(rendered.len() <= 4098);
    assert!(rendered.ends_with('b'));
}

#[test]
fn complete_prefix_holds_back_an_unfinished_sequence() {
    assert_eq!(complete_prefix_len("plain"), 5);
    assert_eq!(complete_prefix_len("ok\u{1b}"), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}[3"), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}[31m"), 7);
    assert_eq!(complete_prefix_len("ok\u{1b}]0;title"), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}]0;title\u{7}"), 12);
    assert_eq!(complete_prefix_len("ok\u{1b}]0;t\u{1b}\\"), 9);
    assert_eq!(complete_prefix_len("ok\u{1b}("), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}(B"), 5);
    assert_eq!(complete_prefix_len("ok\u{1b}7"), 4);

    // An unfinished sequence longer than the hold limit is let through.
    let long = format!("\u{1b}]0;{}", "x".repeat(MAX_HELD_SEQUENCE_BYTES));
    assert_eq!(complete_prefix_len(&long), long.len());
}
