// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the `clampPrefix` and `clampWindow` cases of
// `truncate-surrogates.test.ts`: a cut through an astral character drops the
// whole character, since a Rust string cannot hold half of one.

use super::{clamp_prefix, clamp_suffix};

const EMOJI: &str = "\u{1F980}";

#[test]
fn a_prefix_of_ascii_or_short_text_is_cut_plainly() {
    assert_eq!(clamp_prefix("abcdef", 3), "abc");
    assert_eq!(clamp_prefix("abc", 5), "abc");
    assert_eq!(clamp_prefix("", 3), "");
}

#[test]
fn a_prefix_cut_inside_a_pair_drops_the_character() {
    let text = format!("ab{EMOJI}");
    assert_eq!(clamp_prefix(&text, 3), "ab");
    assert_eq!(clamp_prefix(&text, 4), text);
}

#[test]
fn a_suffix_cut_inside_a_pair_drops_the_character() {
    let text = format!("ab{EMOJI}cd");
    assert_eq!(clamp_suffix(&text, 3), "cd");
    assert_eq!(clamp_suffix(&text, 4), format!("{EMOJI}cd"));
    assert_eq!(clamp_suffix(&text, 6), text);
    assert_eq!(clamp_suffix("abc", 0), "");
}
