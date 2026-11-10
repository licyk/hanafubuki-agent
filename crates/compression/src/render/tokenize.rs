// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/tokenize.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: no BPE tokenizer and no host-projected
// thinking tokens.

use crate::model::Message;
use crate::text::utf16_len;

/// The kernel's token estimate: one token per CJK character (they tokenize
/// about one to one) and a quarter token per other UTF-16 code unit,
/// rounded up.
pub fn default_count_tokens(text: &str) -> u64 {
    if text.is_ascii() {
        return text.len().div_ceil(4) as u64;
    }
    let cjk = text.chars().filter(|character| is_cjk(*character)).count();
    let rest = utf16_len(text) - cjk;
    (cjk + rest.div_ceil(4)) as u64
}

/// A message's size as every count (ranges, blocks, the zone, tags) sees it.
pub fn count_message_tokens(message: &Message) -> u64 {
    default_count_tokens(&message.text)
}

fn is_cjk(character: char) -> bool {
    matches!(
        character,
        '\u{4e00}'..='\u{9fff}' | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}'
    )
}
