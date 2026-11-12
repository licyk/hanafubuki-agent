use std::time::Instant;

use hanafubuki_agent_foundation::terminal_text::render_plain_text;

use super::*;

#[test]
fn tags_and_offsets_streams() {
    let mut buf = OutputBuffer::new(1024, None, None);
    buf.push(OutputStream::Stdout, b"out1");
    buf.push(OutputStream::Stderr, b"err1");
    buf.push(OutputStream::Stdout, b"out2");

    let (deltas, start, next, truncated) = buf.read_from(0, None);
    assert_eq!(start, 0);
    assert_eq!(next, 12);
    assert!(!truncated);
    assert_eq!(deltas.len(), 3);
    assert_eq!(deltas[0].stream, OutputStream::Stdout);
    assert_eq!(deltas[0].data, b"out1");
    assert_eq!(deltas[1].stream, OutputStream::Stderr);
    assert_eq!(deltas[2].data, b"out2");
}

#[test]
fn reads_incrementally_from_offset() {
    let mut buf = OutputBuffer::new(1024, None, None);
    buf.push(OutputStream::Merged, b"hello");
    let (_, _, next, _) = buf.read_from(0, None);
    assert_eq!(next, 5);
    buf.push(OutputStream::Merged, b"world");
    let (deltas, start, next2, _) = buf.read_from(next, None);
    assert_eq!(start, 5);
    assert_eq!(next2, 10);
    assert_eq!(deltas[0].data, b"world");
}

#[test]
fn caps_and_reports_truncation() {
    let mut buf = OutputBuffer::new(8, None, None);
    buf.push(OutputStream::Merged, b"0123456789"); // 10 > cap 8
                                                   // The buffer only retains a tail; reading from 0 is truncated.
    let (_, start, _, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert!(start > 0);
}

#[test]
fn respects_max_bytes() {
    let mut buf = OutputBuffer::new(1024, None, None);
    buf.push(OutputStream::Merged, b"abcdef");
    let (deltas, _, next, _) = buf.read_from(0, Some(3));
    assert_eq!(next, 3);
    assert_eq!(deltas[0].data, b"abc");
}

#[test]
fn single_oversized_chunk_retains_tail() {
    let mut buf = OutputBuffer::new(4, None, None);
    buf.push(OutputStream::Merged, b"0123456789"); // 10 bytes, cap 4
    assert_eq!(buf.next_offset(), 10);
    let (deltas, start, next, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert_eq!(start, 6); // last 4 bytes: offsets 6..10
    assert_eq!(next, 10);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].data, b"6789");
}

#[test]
fn mixed_streams_preserve_tags_after_truncation() {
    let mut buf = OutputBuffer::new(5, None, None);
    buf.push(OutputStream::Stdout, b"AAAA"); // offsets 0..4
    buf.push(OutputStream::Stderr, b"BBBB"); // offsets 4..8; total 8 > cap 5
                                             // Retained tail is the newest 5 bytes: last of "AAAA" + all "BBBB".
    let (deltas, start, next, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert_eq!(start, 3);
    assert_eq!(next, 8);
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0].stream, OutputStream::Stdout);
    assert_eq!(deltas[0].data, b"A");
    assert_eq!(deltas[1].stream, OutputStream::Stderr);
    assert_eq!(deltas[1].data, b"BBBB");
}

#[test]
fn incremental_read_offsets_correct_after_truncation() {
    let mut buf = OutputBuffer::new(6, None, None);
    buf.push(OutputStream::Merged, b"012345"); // fills cap; offsets 0..6
    let (_, _, n1, _) = buf.read_from(0, None);
    assert_eq!(n1, 6);
    buf.push(OutputStream::Merged, b"6789"); // offsets 6..10; drops "0123"
    let (deltas, start, n2, truncated) = buf.read_from(n1, None);
    assert!(!truncated); // reading from 6, oldest is 4
    assert_eq!(start, 6);
    assert_eq!(n2, 10);
    assert_eq!(deltas[0].data, b"6789");
    // Reading from 0 now reports truncation and returns the retained tail.
    let (_, start0, _, trunc0) = buf.read_from(0, None);
    assert!(trunc0);
    assert_eq!(start0, 4);
}

#[test]
fn pipe_output_has_no_terminal_size() {
    let mut buf = OutputBuffer::new(1024, None, None);
    buf.push(OutputStream::Stdout, b"line\n");
    assert_eq!(buf.terminal_size(), None);
    assert!(!buf.record_resize(TerminalSize::new(120, 30)));
    assert_eq!(buf.terminal_sizes(0, 5), (None, Vec::new()));
}

#[test]
fn records_pty_resizes_at_the_next_output_offset() {
    let narrow = TerminalSize::DEFAULT;
    let wide = TerminalSize::new(120, 30);
    let mut buf = OutputBuffer::new(1024, Some(narrow), None);
    buf.push(OutputStream::Merged, b"0123456789");
    assert!(buf.record_resize(wide));
    assert!(!buf.record_resize(wide));
    buf.push(OutputStream::Merged, b"abcde");

    assert_eq!(buf.terminal_size(), Some(wide));
    assert_eq!(
        buf.terminal_sizes(0, 15),
        (
            Some(narrow),
            vec![TerminalResize {
                offset: 10,
                size: wide
            }]
        )
    );
    // A read starting at the change reports it as its starting size.
    assert_eq!(buf.terminal_sizes(10, 15), (Some(wide), Vec::new()));
}

#[test]
fn trimming_keeps_the_size_of_the_oldest_retained_byte() {
    let narrow = TerminalSize::DEFAULT;
    let wide = TerminalSize::new(120, 30);
    let mut buf = OutputBuffer::new(8, Some(narrow), None);
    buf.push(OutputStream::Merged, b"0123");
    buf.record_resize(wide);
    buf.push(OutputStream::Merged, b"456789"); // trims offsets 0..2

    let (_, start, next, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert_eq!((start, next), (2, 10));
    assert_eq!(
        buf.terminal_sizes(start, next),
        (
            Some(narrow),
            vec![TerminalResize {
                offset: 4,
                size: wide
            }]
        )
    );

    buf.push(OutputStream::Merged, b"ab"); // trims through the change
    let (_, start, next, _) = buf.read_from(0, None);
    assert_eq!(buf.terminal_sizes(start, next), (Some(wide), Vec::new()));
}

#[test]
fn capped_reads_end_on_a_character_boundary() {
    let mut buf = OutputBuffer::new(1024, None, None);
    buf.push(OutputStream::Stdout, "中文".as_bytes()); // two 3-byte scalars
    buf.push(OutputStream::Stdout, b"!");

    // A 4-byte budget would cut the second scalar; the read stops before it
    // and the next read resumes there.
    let (deltas, start, next, _) = buf.read_from(0, Some(4));
    assert_eq!((start, next), (0, 3));
    assert_eq!(deltas[0].data, "中".as_bytes());
    let (deltas, _, next, _) = buf.read_from(next, Some(4));
    assert_eq!(next, 7);
    assert_eq!(deltas[0].data, "文!".as_bytes());

    // A budget smaller than one scalar still makes progress.
    let (_, _, next, _) = buf.read_from(0, Some(2));
    assert_eq!(next, 2);
}

#[test]
fn trimming_never_keeps_the_tail_of_a_cut_character() {
    let mut buf = OutputBuffer::new(5, None, None);
    buf.push(OutputStream::Merged, "a中".as_bytes());
    buf.push(OutputStream::Merged, "文".as_bytes()); // 7 bytes, cap 5

    let (deltas, start, _, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert_eq!(start, 4);
    assert_eq!(deltas[0].data, "文".as_bytes());

    let mut single = OutputBuffer::new(4, None, None);
    single.push(OutputStream::Merged, "中文".as_bytes()); // one oversized chunk
    let (deltas, start, _, _) = single.read_from(0, None);
    assert_eq!(start, 3);
    assert_eq!(deltas[0].data, "文".as_bytes());
}

#[test]
fn without_a_fallback_decoding_holds_only_an_unfinished_scalar() {
    let now = Instant::now();
    let mut decoder = RunDecoder::new(None);
    let mut out = Vec::new();
    let bytes = "a中".as_bytes();
    decoder.push(&bytes[..2], now, &mut out);
    assert_eq!(out, b"a");
    decoder.push(&bytes[2..], now, &mut out);
    assert_eq!(out, "a中".as_bytes());

    // Malformed bytes are not held back, time never releases an unfinished
    // scalar, and the end of the stream passes whatever is left.
    let mut out = Vec::new();
    decoder.push(&[0xff, b'x', 0xe4, 0xb8], now, &mut out);
    assert!(!decoder.is_due(now + decode::HOLD));
    decoder.finish(&mut out);
    assert_eq!(out, [0xff, b'x', 0xe4, 0xb8]);
}

#[test]
fn reads_never_start_inside_a_character() {
    let mut buf = OutputBuffer::new(1024, None, None);
    buf.push(OutputStream::Merged, "中文".as_bytes());
    let (deltas, start, next, _) = buf.read_from(1, None);
    assert_eq!((start, next), (3, 6));
    assert_eq!(deltas[0].data, "文".as_bytes());
}

#[test]
#[ignore = "reproduces §4.9 step 6, a read inside a sequence; fixed in E3"]
fn a_read_after_trimming_never_starts_inside_an_escape_sequence() {
    let mut buf = OutputBuffer::new(13, None, None);
    buf.push(OutputStream::Merged, b"ab\x1b[31m");
    buf.push(OutputStream::Merged, b"red\nnext\n"); // 16 bytes, cap 13

    let (deltas, _, _, truncated) = buf.read_from(0, None);
    assert!(truncated);
    let bytes = deltas
        .iter()
        .flat_map(|delta| delta.data.clone())
        .collect::<Vec<_>>();
    let rendered = render_plain_text(&bytes);
    assert!(rendered.ends_with("next\n"), "{rendered:?}");
    // Only the rest of the cut sequence can leave an `m`.
    assert!(!rendered.contains('m'), "{rendered:?}");
}

/// Decodes one stream's `reads`, in order, as a pipe reader does with
/// `code_page` as the fallback (D15).
fn decode_stream(reads: &[&[u8]], code_page: u32) -> String {
    let mut decoder = RunDecoder::new(decode::code_page_encoding(code_page));
    let mut decoded = Vec::new();
    for read in reads {
        decoder.push(read, Instant::now(), &mut decoded);
    }
    decoder.finish(&mut decoded);
    String::from_utf8(decoded).expect("decoded output is UTF-8")
}

#[test]
fn gbk_output_decodes_with_code_page_936() {
    assert_eq!(decode_stream(&[b"\xd6\xd0\xce\xc4\n"], 936), "中文\n");
}

#[test]
fn utf8_and_gbk_in_one_stream_decode_each_by_its_own_encoding() {
    let mixed = [
        b"PowerShell \xe4\xb8\xad\xe6\x96\x87, cmd ".as_slice(),
        b"\xd6\xd0\xce\xc4\n",
    ];
    assert_eq!(decode_stream(&mixed, 936), "PowerShell 中文, cmd 中文\n");
    assert_eq!(
        decode_stream(&[&mixed.concat()], 936),
        "PowerShell 中文, cmd 中文\n"
    );
}

#[test]
fn a_gbk_lead_byte_takes_its_trail_byte_even_in_the_ascii_range_and_across_reads() {
    // 錯誤 is E5 65 D5 60 and 乗 is 81 5C: every trail byte is ASCII.
    assert_eq!(decode_stream(&[b"\xe5", b"e\xd5", b"`\n"], 936), "錯誤\n");
    assert_eq!(decode_stream(&[b"\x81\\\n"], 936), "乗\n");
}

#[test]
fn cp866_output_decodes_with_code_page_866() {
    assert_eq!(
        decode_stream(&[b"\x8f\xe0\xa8\xa2\xa5\xe2\n"], 866),
        "Привет\n"
    );
}

#[test]
fn utf8_output_stays_utf8_whatever_the_fallback_code_page() {
    let text = "café ü 中文\n";
    for code_page in [936, 866, 1252] {
        assert_eq!(decode_stream(&[text.as_bytes()], code_page), text);
    }
}

#[test]
fn a_cmd_dir_listing_in_code_page_936_decodes() {
    // `dir` run by the Cmd shell on Chinese Windows, as reported: cmd's
    // built-ins write the console's code page, 936.
    let listing: &[u8] = b" \xc7\xfd\xb6\xaf\xc6\xf7 D \xd6\xd0\xb5\xc4\xbe\xed\xc3\xbb\xd3\xd0\xb1\xea\xc7\xa9\xa1\xa3\r\n \
        \xbe\xed\xb5\xc4\xd0\xf2\xc1\xd0\xba\xc5\xca\xc7 410D-D6D5\r\n\
        2025/06/12  15:24           245,113 \xc6\xc1\xc4\xbb\xbd\xd8\xcd\xbc 2025-06-12 152455.png\r\n\
        \x20            270 \xb8\xf6\xce\xc4\xbc\xfe  2,270,726,673 \xd7\xd6\xbd\xda\r\n";
    let expected = " 驱动器 D 中的卷没有标签。\r\n 卷的序列号是 410D-D6D5\r\n\
        2025/06/12  15:24           245,113 屏幕截图 2025-06-12 152455.png\r\n\
        \x20            270 个文件  2,270,726,673 字节\r\n";
    assert_eq!(decode_stream(&[listing], 936), expected);
    // Read in pieces that split a character's two bytes.
    let (first, rest) = listing.split_at(3);
    let (second, third) = rest.split_at(40);
    assert_eq!(decode_stream(&[first, second, third], 936), expected);
}

#[test]
fn a_utf8_run_keeps_the_ascii_byte_after_it() {
    // `中文` in UTF-8 ends on 0x87, which GBK would pair with `e`.
    assert_eq!(decode_stream(&["中文e\n".as_bytes()], 936), "中文e\n");
    assert_eq!(decode_stream(&["中文".as_bytes(), b"e\n"], 936), "中文e\n");
}

#[test]
fn an_invalid_sequence_never_swallows_the_ascii_byte_after_it() {
    // 0x81 0x30 begins a four-byte GB18030 sequence that never ends.
    assert_eq!(decode_stream(&[b"\x810\n"], 936), "\u{fffd}0\n");
}

#[test]
fn a_held_run_is_stored_once_no_read_has_continued_it_for_the_hold() {
    let mut buf = OutputBuffer::new(1024, None, decode::code_page_encoding(936));
    // A prompt that ends in a run, then waits: `请输入：` in GBK.
    buf.push_read(OutputStream::Stdout, b"> \xc7\xeb\xca\xe4\xc8\xeb\xa3\xba");
    let read_at = Instant::now();
    assert_eq!(buf.next_offset(), 2);

    buf.release_held(read_at);
    assert_eq!(buf.next_offset(), 2);
    buf.release_held(read_at + decode::HOLD);
    let (deltas, _, _, _) = buf.read_from(0, None);
    assert_eq!(deltas[0].data, "> 请输入：".as_bytes());

    // UTF-8 is released whole but for an unfinished scalar, which waits for
    // its read.
    let mut buf = OutputBuffer::new(1024, None, decode::code_page_encoding(936));
    buf.push_read(OutputStream::Stdout, &"中文".as_bytes()[..5]);
    buf.release_held(Instant::now() + decode::HOLD);
    assert_eq!(buf.next_offset(), 3);
    buf.push_read(OutputStream::Stdout, &"中文".as_bytes()[5..]);
    buf.finish_stream(OutputStream::Stdout);
    let (deltas, _, _, _) = buf.read_from(0, None);
    assert_eq!(deltas[0].data, "中文".as_bytes());
}

#[test]
fn each_stream_has_its_own_decoder() {
    let mut buf = OutputBuffer::new(1024, None, decode::code_page_encoding(936));
    buf.push_read(OutputStream::Stdout, b"\xd6");
    buf.push_read(OutputStream::Stderr, b"err\n");
    buf.push_read(OutputStream::Stdout, b"\xd0\n");
    buf.finish_stream(OutputStream::Stdout);
    buf.finish_stream(OutputStream::Stderr);
    let (deltas, _, _, _) = buf.read_from(0, None);
    assert_eq!(
        deltas
            .iter()
            .map(|delta| (delta.stream, String::from_utf8(delta.data.clone()).unwrap()))
            .collect::<Vec<_>>(),
        [
            (OutputStream::Stderr, "err\n".to_string()),
            (OutputStream::Stdout, "中\n".to_string()),
        ]
    );
}
