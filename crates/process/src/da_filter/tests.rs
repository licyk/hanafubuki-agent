use super::*;

fn run(filter: &mut DaFilter, input: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut out = Vec::new();
    let mut replies = Vec::new();
    filter.process(input, &mut out, |reply| replies.push(reply.to_vec()));
    (out, replies)
}

#[test]
fn da1_bare_is_answered() {
    let mut filter = DaFilter::new();
    let (out, replies) = run(&mut filter, b"\x1b[c");

    assert!(out.is_empty());
    assert_eq!(replies, vec![DA1_REPLY.to_vec()]);
}

#[test]
fn da2_secondary_is_answered() {
    let mut filter = DaFilter::new();
    let (out, replies) = run(&mut filter, b"\x1b[>c");

    assert!(out.is_empty());
    assert_eq!(replies, vec![DA2_REPLY.to_vec()]);
}

#[test]
fn plain_text_passes_through() {
    let mut filter = DaFilter::new();
    let (out, replies) = run(&mut filter, b"hello world\n");

    assert_eq!(out, b"hello world\n");
    assert!(replies.is_empty());
}

#[test]
fn startup_cursor_position_query_is_answered_once() {
    let mut filter = DaFilter::new();
    let (out, replies) = run(&mut filter, b"\x1b[6n");

    assert!(out.is_empty());
    assert_eq!(replies, vec![DSR_CPR_REPLY.to_vec()]);

    let (out, replies) = run(&mut filter, b"\x1b[6n");
    assert_eq!(out, b"\x1b[6n");
    assert!(replies.is_empty());
}

#[test]
fn split_da_sequence_is_answered() {
    let mut filter = DaFilter::new();
    let (out1, replies1) = run(&mut filter, b"\x1b");
    let (out2, replies2) = run(&mut filter, b"[");
    let (out3, replies3) = run(&mut filter, b"c");

    assert!(out1.is_empty() && out2.is_empty() && out3.is_empty());
    assert!(replies1.is_empty() && replies2.is_empty());
    assert_eq!(replies3, vec![DA1_REPLY.to_vec()]);
}
