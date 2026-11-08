use super::WindowsTtyInputNormalizer;

#[test]
fn normalizes_terminal_input_without_changing_text_or_ctrl_c() {
    let mut normalizer = WindowsTtyInputNormalizer::default();
    assert_eq!(normalizer.normalize(b"first\n"), b"first\r");
    assert_eq!(normalizer.normalize(b"second\r"), b"second\r");
    // A leading LF that continues a CR from the previous write is dropped, and
    // the trailing CRLF collapses to a single CR.
    assert_eq!(normalizer.normalize(b"\nthird\r\n"), b"third\r");

    let mut input_with_controls = "cafeé 漢字".as_bytes().to_vec();
    input_with_controls.extend_from_slice(b"\x08\x03");
    let mut expected = "cafeé 漢字".as_bytes().to_vec();
    expected.extend_from_slice(b"\x7f\x03");
    assert_eq!(normalizer.normalize(&input_with_controls), expected);
}

#[test]
fn split_crlf_across_writes_collapses_to_single_cr() {
    let mut normalizer = WindowsTtyInputNormalizer::default();
    // CR arrives at the end of one write...
    assert_eq!(normalizer.normalize(b"line\r"), b"line\r");
    // ...and the matching LF at the start of the next: it must be dropped.
    assert_eq!(normalizer.normalize(b"\nnext"), b"next");
}

#[test]
fn bare_lf_becomes_cr() {
    let mut normalizer = WindowsTtyInputNormalizer::default();
    assert_eq!(normalizer.normalize(b"a\nb\n"), b"a\rb\r");
}
