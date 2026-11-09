// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `issue5620-ref-recoverable-media.test.ts` and the carry-forward
// filter case of `media-payload.test.ts`. A message's handles are its
// `images`, so the per-sidecar extraction matrix, the pinning split and the
// legacy `BLOCKED` migration (no message is pinned for media here) have no
// counterpart.

use hanafubuki_agent_compression::{
    append_media_refs_section, collect_fold_media_refs, extract_media_refs,
    looks_ephemeral_signed_url, parse_media_refs_section, Block, Message, MEDIA_REFS_MARKER,
};

use crate::fixtures::{apply, assistant, config, range, state_for, user, with_topic};

const DATA_URL: &str =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAASUVORK5CYII=";
const REMOTE_URL: &str = "https://example.com/screenshot.png";
const FILE_ID: &str = "file-api-abc123";
const SIGNED: &str =
    "https://bucket.s3.amazonaws.com/img.png?X-Amz-Expires=3600&X-Amz-Signature=abc";

fn with_images(id: &str, images: &[&str]) -> Message {
    let mut message = user(id, "");
    message.images = images.iter().map(|handle| (*handle).to_owned()).collect();
    message
}

fn strings(handles: &[&str]) -> Vec<String> {
    handles.iter().map(|handle| (*handle).to_owned()).collect()
}

fn summarized(summary: &str) -> Block {
    Block {
        block_id: "b1".to_owned(),
        summary: summary.to_owned(),
        ..Block::default()
    }
}

#[test]
fn a_message_s_handles_exclude_inline_and_expiring_ones() {
    let message = with_images(
        "s",
        &[SIGNED, REMOTE_URL, DATA_URL, FILE_ID, REMOTE_URL, ""],
    );
    assert_eq!(
        extract_media_refs(&message),
        strings(&[REMOTE_URL, FILE_ID])
    );
    assert!(extract_media_refs(&with_images("s1", &[SIGNED])).is_empty());
    assert!(extract_media_refs(&with_images(
        "s3",
        &["https://cdn.example.com/i.png?token=abc"]
    ))
    .is_empty());
}

#[test]
fn appending_the_section_is_idempotent_and_replaces_a_stale_twin() {
    let base = "Work summary bullets.";
    let refs = strings(&[REMOTE_URL]);
    let once = append_media_refs_section(base, &refs);
    assert_eq!(
        once,
        format!("{base}\n\n{MEDIA_REFS_MARKER}\n- {REMOTE_URL}")
    );
    assert_eq!(append_media_refs_section(&once, &refs), once);
    let stale = format!("{base}\n\n{MEDIA_REFS_MARKER}\n- https://old.example.com/a.png");
    let replaced = append_media_refs_section(&stale, &refs);
    assert!(!replaced.contains("https://old.example.com/a.png"));
    assert_eq!(replaced, once);
    assert_eq!(append_media_refs_section(base, &[]), base);
    assert_eq!(
        append_media_refs_section("", &refs),
        format!("{MEDIA_REFS_MARKER}\n- {REMOTE_URL}")
    );
}

#[test]
fn the_section_parses_back_and_is_optional() {
    let summary = append_media_refs_section("x", &strings(&[REMOTE_URL, FILE_ID]));
    assert_eq!(
        parse_media_refs_section(&summary),
        strings(&[REMOTE_URL, FILE_ID])
    );
    assert!(parse_media_refs_section("no section here").is_empty());
}

#[test]
fn a_fold_s_handles_are_its_messages_then_its_consumed_blocks() {
    let direct = with_images("d", &[REMOTE_URL]);
    let carried = summarized(&format!("old\n\n{MEDIA_REFS_MARKER}\n- {FILE_ID}"));
    assert_eq!(
        collect_fold_media_refs([&direct], [&carried]),
        strings(&[REMOTE_URL, FILE_ID])
    );
    assert!(collect_fold_media_refs([], []).is_empty());
}

#[test]
fn a_carried_handle_that_expires_is_dropped() {
    let durable = "https://cdn.example.com/img.png";
    let old = summarized(&format!(
        "analysis summarized\n\n{MEDIA_REFS_MARKER}\n- {SIGNED}\n- {durable}"
    ));
    assert_eq!(collect_fold_media_refs([], [&old]), strings(&[durable]));
}

#[test]
fn a_fold_records_its_images_handles_in_the_summary() {
    let messages = [
        user("u", "the task"),
        with_images("img", &[REMOTE_URL]),
        assistant("t", "analyzing"),
    ];
    let outcome = apply(
        &[with_topic(range("m00001", "m00003", "folded work"), "t")],
        &messages,
        &state_for(&messages),
        &config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(
        outcome.state.blocks[0].summary,
        format!("folded work\n\n{MEDIA_REFS_MARKER}\n- {REMOTE_URL}")
    );
}

#[test]
fn a_block_boundary_merge_carries_every_source_block_s_handles() {
    let messages = [
        user("u1", "first task"),
        with_images("img1", &[REMOTE_URL]),
        user("u2", "second task"),
        with_images("file2", &[FILE_ID]),
    ];
    let config = config();
    let first = apply(
        &[with_topic(range("m00001", "m00002", "first fold"), "a")],
        &messages,
        &state_for(&messages),
        &config,
    );
    let second = apply(
        &[with_topic(range("m00003", "m00004", "second fold"), "b")],
        &messages,
        &first.state,
        &config,
    );
    let merged = apply(
        &[with_topic(range("b1", "b2", "distilled both"), "c")],
        &messages,
        &second.state,
        &config,
    );
    assert!(
        merged.result.errors.is_empty(),
        "{:?}",
        merged.result.errors
    );
    let block = merged.state.active_blocks().next().unwrap();
    assert_eq!(block.direct_block_ids, ["b1", "b2"]);
    assert_eq!(
        parse_media_refs_section(&block.summary),
        strings(&[REMOTE_URL, FILE_ID])
    );
}

#[test]
fn signed_and_expiring_urls_are_recognized() {
    for signed in [
        "https://bucket.s3.amazonaws.com/img.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AK%2F2026%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20260101T000000Z&X-Amz-Expires=3600&X-Amz-Signature=abc123",
        "https://bucket.s3.amazonaws.com/img.png?AWSAccessKeyId=AK&Expires=1799999999&Signature=abc",
        "https://storage.googleapis.com/b/i.png?X-Goog-Expires=3600&X-Goog-Signature=abc",
        "https://acct.blob.core.windows.net/c/i.png?sp=r&st=2026-01-01T00:00:00Z&se=2026-01-02T00:00:00Z&sig=abc",
        "https://cdn.example.com/i.png?token=abc123",
        "http://intra.example.com/i.png?expiry=1799999999",
        "https://cdn.example.com/i.png?ttl=3600",
        "https://cdn.example.com/i.png?sig=abc",
    ] {
        assert!(looks_ephemeral_signed_url(signed), "{signed}");
    }
    for durable in [
        REMOTE_URL,
        "https://example.com/i.png?w=100&format=png&key=folder/a.png",
        "gs://bucket/img.png",
        FILE_ID,
        DATA_URL,
    ] {
        assert!(!looks_ephemeral_signed_url(durable), "{durable}");
    }
}
