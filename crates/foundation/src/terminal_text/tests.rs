use super::{
    aligned_start, complete_prefix_len, end_point, render_plain_text, resume_point,
    ALIGN_SEARCH_BYTES, MAX_HELD_SEQUENCE_BYTES,
};

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
    assert_eq!(complete_prefix_len("plain".as_bytes()), 5);
    assert_eq!(complete_prefix_len("ok\u{1b}".as_bytes()), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}[3".as_bytes()), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}[31m".as_bytes()), 7);
    assert_eq!(complete_prefix_len("ok\u{1b}]0;title".as_bytes()), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}]0;title\u{7}".as_bytes()), 12);
    assert_eq!(complete_prefix_len("ok\u{1b}]0;t\u{1b}\\".as_bytes()), 9);
    assert_eq!(complete_prefix_len("ok\u{1b}(".as_bytes()), 2);
    assert_eq!(complete_prefix_len("ok\u{1b}(B".as_bytes()), 5);
    assert_eq!(complete_prefix_len("ok\u{1b}7".as_bytes()), 4);

    // An unfinished sequence longer than the hold limit is let through.
    let long = format!("\u{1b}]0;{}", "x".repeat(MAX_HELD_SEQUENCE_BYTES));
    assert_eq!(complete_prefix_len(long.as_bytes()), long.len());
}

// A wide character takes two cells, so a column a terminal moves to after CJK
// text is not its count of characters.

#[test]
fn an_absolute_column_after_cjk_text_counts_two_cells_per_wide_character() {
    assert_eq!(render_plain_text("中文\x1b[5Gx\n".as_bytes()), "中文x\n");
}

#[test]
fn a_carriage_return_redraw_over_cjk_text_moves_by_cells() {
    assert_eq!(
        render_plain_text("中文abc\r\x1b[5CX\n".as_bytes()),
        "中文aXc\n"
    );
}

#[test]
fn a_wide_character_overwritten_by_half_leaves_a_blank() {
    // `x` lands on the second cell of 中, `y` on the first cell of 文.
    assert_eq!(render_plain_text("中文\x1b[2Gx\n".as_bytes()), " x文\n");
    assert_eq!(render_plain_text("中文\x1b[3Gy\n".as_bytes()), "中y \n");
}

#[test]
fn zero_width_characters_stay_with_the_character_before_them() {
    assert_eq!(
        render_plain_text("e\u{301}x\x1b[2Gy\n".as_bytes()),
        "e\u{301}y\n"
    );
}

#[test]
fn complete_prefix_works_on_output_that_is_not_utf8() {
    assert_eq!(complete_prefix_len(b"ok\x1b]0;\xffti"), 2);
    assert_eq!(complete_prefix_len(b"\xff\x1b[31m"), 6);
}

#[test]
fn a_cut_stream_resumes_after_its_first_line_break() {
    assert_eq!(resume_point(b"1mred\nnext\n"), 6);
    assert_eq!(resume_point(b"plain"), 0);
    // The rest of a cut character is skipped.
    assert_eq!(resume_point("文x".as_bytes().split_at(1).1), 2);
}

#[test]
fn a_cut_stream_without_a_line_break_resumes_after_a_complete_sequence() {
    let bytes = format!("1mred\x1b[0m{}", "x".repeat(ALIGN_SEARCH_BYTES));
    assert_eq!(resume_point(bytes.as_bytes()), 9);
    // A line break too far away does not count.
    let far = format!("{}\nnext", "x".repeat(ALIGN_SEARCH_BYTES));
    assert_eq!(resume_point(far.as_bytes()), 0);
}

#[test]
fn a_bounded_read_ends_after_a_line_break_or_before_a_cut_sequence() {
    assert_eq!(end_point(b"one\ntwo\nthree", 10), 8);
    assert_eq!(end_point(b"short", 10), 5);
    let long = format!("{}\x1b[31mred", "x".repeat(ALIGN_SEARCH_BYTES + 8));
    assert_eq!(
        end_point(long.as_bytes(), ALIGN_SEARCH_BYTES + 11),
        ALIGN_SEARCH_BYTES + 8
    );
    let cjk = format!("{}文", "x".repeat(ALIGN_SEARCH_BYTES + 1));
    assert_eq!(
        end_point(cjk.as_bytes(), ALIGN_SEARCH_BYTES + 2),
        ALIGN_SEARCH_BYTES + 1
    );
}

#[test]
fn a_cut_at_the_start_of_a_line_keeps_that_line() {
    assert_eq!(aligned_start(b"one\ntwo\nthree\n", 4), 4);
    assert_eq!(aligned_start(b"one\ntwo\nthree\n", 5), 8);
    assert_eq!(aligned_start(b"one", 0), 0);
}
