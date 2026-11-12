use super::{omission_marker, output_window, WindowPart};

fn numbered(lines: usize) -> String {
    (1..=lines)
        .map(|number| format!("line {number}\n"))
        .collect()
}

#[test]
fn output_within_the_bound_is_returned_whole() {
    let bytes = numbered(3);
    assert_eq!(
        output_window(bytes.as_bytes(), 10, 10, 1024),
        [WindowPart::Text {
            bytes: bytes.as_bytes(),
            offset: 10
        }]
    );
    assert!(output_window(b"", 0, 0, 1024).is_empty());
}

#[test]
fn output_past_the_bound_keeps_its_head_and_tail_on_line_breaks() {
    let bytes = numbered(1000);
    let parts = output_window(bytes.as_bytes(), 0, 0, 4096);
    let [WindowPart::Text { bytes: head, .. }, WindowPart::Omitted(omitted), WindowPart::Text {
        bytes: tail,
        offset,
    }] = parts.as_slice()
    else {
        panic!("{parts:?}");
    };
    assert!(head.starts_with(b"line 1\n") && head.ends_with(b"\n"));
    assert!(tail.ends_with(b"line 1000\n"));
    assert!(tail.starts_with(b"line "), "{:?}", &tail[..16]);
    assert!(head.len() + tail.len() <= 4096);
    assert_eq!(head.len() as u64 + omitted, *offset);
    assert_eq!(*offset as usize + tail.len(), bytes.len());
}

#[test]
fn a_read_whose_start_was_dropped_says_how_much_and_keeps_the_tail() {
    let bytes = numbered(3);
    assert_eq!(
        output_window(bytes.as_bytes(), 4, 10, 1024),
        [
            WindowPart::Omitted(6),
            WindowPart::Text {
                bytes: bytes.as_bytes(),
                offset: 10
            }
        ]
    );

    let bytes = numbered(1000);
    let parts = output_window(bytes.as_bytes(), 0, 100, 4096);
    let [WindowPart::Omitted(omitted), WindowPart::Text {
        bytes: tail,
        offset,
    }] = parts.as_slice()
    else {
        panic!("{parts:?}");
    };
    // From the poll's offset, 0, to the tail.
    assert_eq!(*omitted, *offset);
    assert!(tail.starts_with(b"line ") && tail.ends_with(b"line 1000\n"));
}

#[test]
fn one_long_line_is_cut_on_character_boundaries() {
    let bytes = format!("head\n{}\ntail\n", "文".repeat(40_000));
    let parts = output_window(bytes.as_bytes(), 0, 0, 4096);
    let [WindowPart::Text { bytes: head, .. }, WindowPart::Omitted(_), WindowPart::Text { bytes: tail, .. }] =
        parts.as_slice()
    else {
        panic!("{parts:?}");
    };
    assert!(std::str::from_utf8(head).unwrap().starts_with("head\n文"));
    assert!(std::str::from_utf8(tail).unwrap().ends_with("文\ntail\n"));
}

#[test]
fn the_marker_names_the_bytes_left_out() {
    assert_eq!(omission_marker(1234), "[… 1234 bytes omitted …]");
}
