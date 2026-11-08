use super::SseDecoder;

#[test]
fn arbitrary_fragmentation_preserves_multiline_utf8_events() {
    let payload = b": heartbeat\r\nevent: response\r\ndata: {\"text\":\"\xe4\xbd\xa0\r\ndata: \xe5\xa5\xbd\"}\r\n\r\n";
    let mut decoder = SseDecoder::new();
    let mut events = Vec::new();
    for byte in payload {
        events.extend(decoder.feed(&[*byte]).unwrap());
    }
    decoder.finish().unwrap();
    assert_eq!(events, vec!["{\"text\":\"你\n好\"}".as_bytes()]);
}

#[test]
fn incomplete_and_oversized_events_fail_closed() {
    let mut incomplete = SseDecoder::new();
    incomplete.feed(b"data: unfinished").unwrap();
    assert!(incomplete.finish().is_err());

    let mut oversized = SseDecoder::new();
    assert!(oversized.feed(&vec![b'x'; 1024 * 1024 + 1]).is_err());
}

#[test]
fn one_large_http_chunk_may_contain_many_bounded_events() {
    let event = b"data: {}\n\n";
    let payload = event.repeat(100_000);
    let mut decoder = SseDecoder::new();
    assert_eq!(decoder.feed(&payload).unwrap().len(), 100_000);
    decoder.finish().unwrap();
}

#[test]
fn cr_lf_crlf_and_split_crlf_are_all_line_boundaries() {
    let mut decoder = SseDecoder::new();
    let mut events = decoder
        .feed(b"data: one\r\rdata: two\n\ndata: three\r")
        .unwrap();
    events.extend(decoder.feed(b"\n\r\n").unwrap());
    decoder.finish().unwrap();
    assert_eq!(
        events,
        vec![b"one".as_slice(), b"two".as_slice(), b"three".as_slice()]
    );
}

#[test]
fn an_unlabelled_body_is_decoded_unless_it_is_clearly_not_sse() {
    let mut decoder = SseDecoder::for_unlabelled_body();
    assert!(decoder.feed(b"\n").unwrap().is_empty());
    assert_eq!(
        decoder.feed(b"data: {}\n\n").unwrap(),
        vec![b"{}".as_slice()]
    );
    assert!(decoder.feed(b"{").is_ok(), "only the body start is checked");

    for body in [b"  {\"id\":1}".as_slice(), b"[]", b"<html>"] {
        assert!(SseDecoder::for_unlabelled_body().feed(body).is_err());
    }
}
