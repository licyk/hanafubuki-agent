use super::*;

#[test]
fn tags_and_offsets_streams() {
    let mut buf = OutputBuffer::new(1024, None);
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
    let mut buf = OutputBuffer::new(1024, None);
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
    let mut buf = OutputBuffer::new(8, None);
    buf.push(OutputStream::Merged, b"0123456789"); // 10 > cap 8
                                                   // The buffer only retains a tail; reading from 0 is truncated.
    let (_, start, _, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert!(start > 0);
}

#[test]
fn respects_max_bytes() {
    let mut buf = OutputBuffer::new(1024, None);
    buf.push(OutputStream::Merged, b"abcdef");
    let (deltas, _, next, _) = buf.read_from(0, Some(3));
    assert_eq!(next, 3);
    assert_eq!(deltas[0].data, b"abc");
}

#[test]
fn single_oversized_chunk_retains_tail() {
    let mut buf = OutputBuffer::new(4, None);
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
    let mut buf = OutputBuffer::new(5, None);
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
    let mut buf = OutputBuffer::new(6, None);
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
    let mut buf = OutputBuffer::new(1024, None);
    buf.push(OutputStream::Stdout, b"line\n");
    assert_eq!(buf.terminal_size(), None);
    assert!(!buf.record_resize(TerminalSize::new(120, 30)));
    assert_eq!(buf.terminal_sizes(0, 5), (None, Vec::new()));
}

#[test]
fn records_pty_resizes_at_the_next_output_offset() {
    let narrow = TerminalSize::DEFAULT;
    let wide = TerminalSize::new(120, 30);
    let mut buf = OutputBuffer::new(1024, Some(narrow));
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
    let mut buf = OutputBuffer::new(8, Some(narrow));
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
    let mut buf = OutputBuffer::new(1024, None);
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
    let mut buf = OutputBuffer::new(5, None);
    buf.push(OutputStream::Merged, "a中".as_bytes());
    buf.push(OutputStream::Merged, "文".as_bytes()); // 7 bytes, cap 5

    let (deltas, start, _, truncated) = buf.read_from(0, None);
    assert!(truncated);
    assert_eq!(start, 4);
    assert_eq!(deltas[0].data, "文".as_bytes());

    let mut single = OutputBuffer::new(4, None);
    single.push(OutputStream::Merged, "中文".as_bytes()); // one oversized chunk
    let (deltas, start, _, _) = single.read_from(0, None);
    assert_eq!(start, 3);
    assert_eq!(deltas[0].data, "文".as_bytes());
}

#[test]
fn utf8_carry_holds_only_an_unfinished_scalar() {
    let mut carry = Utf8Carry::default();
    let mut out = Vec::new();
    let bytes = "a中".as_bytes();
    carry.push(&bytes[..2], |chunk| out.push(chunk.to_vec()));
    carry.push(&bytes[2..], |chunk| out.push(chunk.to_vec()));
    assert_eq!(out, vec![b"a".to_vec(), "中".as_bytes().to_vec()]);

    // Malformed bytes are not held back, and the end of the stream flushes
    // whatever is left.
    let mut out = Vec::new();
    carry.push(&[0xff, b'x', 0xe4, 0xb8], |chunk| out.push(chunk.to_vec()));
    carry.finish(|chunk| out.push(chunk.to_vec()));
    assert_eq!(out, vec![vec![0xff, b'x'], vec![0xe4, 0xb8]]);
}

#[test]
fn reads_never_start_inside_a_character() {
    let mut buf = OutputBuffer::new(1024, None);
    buf.push(OutputStream::Merged, "中文".as_bytes());
    let (deltas, start, next, _) = buf.read_from(1, None);
    assert_eq!((start, next), (3, 6));
    assert_eq!(deltas[0].data, "文".as_bytes());
}
