// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/media-refs.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: a message's handles are its `images`, not
// the five wire sidecars, and no message is pinned for its media: the host
// keeps the full history, so decompress always restores the original.

use crate::model::{Block, Message};

/// The header of the section appended to a block's summary at a fold; the
/// prompts tell the model to keep it verbatim, and appending is idempotent
/// on it.
pub const MEDIA_REFS_MARKER: &str =
    "MEDIA REFS (verbatim re-access handles — preserve when distilling):";

/// Query parameter names that mark an HTTP(S) URL as pre-signed or
/// expiring: the cloud providers' signing prefixes, expiry keys and
/// credential keys.
const SIGNING_PREFIXES: [&str; 10] = [
    "amz", "goog", "ms", "cf", "r2", "oss", "azure", "obs", "cos", "bce",
];
const EXPIRING_PARAMS: [&str; 11] = [
    "expires",
    "expire",
    "expiry",
    "exp",
    "ttl",
    "signature",
    "sig",
    "session-token",
    "security-token",
    "token",
    "auth",
];

/// Whether `handle` is an HTTP(S) URL whose query carries a signing or
/// expiry parameter: it dies before a folded summary is read again, so it
/// is no re-access handle.
pub fn looks_ephemeral_signed_url(handle: &str) -> bool {
    let lower = handle.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    let Some((_, query)) = lower.split_once('?') else {
        return false;
    };
    query.split('&').any(|pair| {
        let name = pair.split_once('=').map_or(pair, |(name, _)| name);
        is_signing_param(name)
    })
}

/// The handles a message's images can be reached by again: no inline
/// `data:` URL and no expiring URL, each once, in order.
pub fn extract_media_refs(message: &Message) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    for handle in &message.images {
        if handle.is_empty() || is_inline_data_url(handle) || looks_ephemeral_signed_url(handle) {
            continue;
        }
        if !refs.contains(handle) {
            refs.push(handle.clone());
        }
    }
    refs
}

/// The handles of a summary's MEDIA REFS section; none without one.
pub fn parse_media_refs_section(summary: &str) -> Vec<String> {
    let lines: Vec<&str> = summary.split('\n').collect();
    let Some(marker) = lines
        .iter()
        .position(|line| line.trim() == MEDIA_REFS_MARKER)
    else {
        return Vec::new();
    };
    lines[marker + 1..]
        .iter()
        .map_while(|line| line.strip_prefix("- "))
        .map(|handle| handle.trim().to_owned())
        .collect()
}

/// `summary` with its MEDIA REFS section replaced by one listing `refs`,
/// so a fold applied again never lists a handle twice; unchanged when
/// `refs` is empty.
pub fn append_media_refs_section(summary: &str, refs: &[String]) -> String {
    if refs.is_empty() {
        return summary.to_owned();
    }
    let stripped = strip_media_refs_section(summary);
    let base = stripped.trim_end_matches('\n');
    let entries: Vec<String> = refs.iter().map(|handle| format!("- {handle}")).collect();
    let section = format!("{MEDIA_REFS_MARKER}\n{}", entries.join("\n"));
    if base.is_empty() {
        section
    } else {
        format!("{base}\n\n{section}")
    }
}

/// The handles a fold records: those of the messages it folds, then those
/// its consumed blocks recorded (still durable), each once.
pub fn collect_fold_media_refs<'a>(
    direct_messages: impl IntoIterator<Item = &'a Message>,
    consumed_blocks: impl IntoIterator<Item = &'a Block>,
) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let mut add = |handle: String| {
        if !refs.contains(&handle) {
            refs.push(handle);
        }
    };
    for message in direct_messages {
        extract_media_refs(message).into_iter().for_each(&mut add);
    }
    // A summary written before the expiry filter may still list a signed URL
    // no live message holds.
    for block in consumed_blocks {
        parse_media_refs_section(&block.summary)
            .into_iter()
            .filter(|handle| !looks_ephemeral_signed_url(handle))
            .for_each(&mut add);
    }
    refs
}

fn strip_media_refs_section(summary: &str) -> String {
    let mut lines: Vec<&str> = summary.split('\n').collect();
    let Some(marker) = lines
        .iter()
        .position(|line| line.trim() == MEDIA_REFS_MARKER)
    else {
        return summary.to_owned();
    };
    let end = marker
        + 1
        + lines[marker + 1..]
            .iter()
            .take_while(|line| line.starts_with("- "))
            .count();
    lines.drain(marker..end);
    lines.join("\n")
}

fn is_signing_param(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("x-") {
        return SIGNING_PREFIXES.iter().any(|prefix| {
            rest.strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with('-'))
        });
    }
    EXPIRING_PARAMS.contains(&name)
}

/// `data:<type>[;base64],<payload>`: the only media whose bytes the message
/// itself holds.
fn is_inline_data_url(url: &str) -> bool {
    let Some(rest) = url
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("data:"))
        .map(|_| &url[5..])
    else {
        return false;
    };
    let Some(type_end) = rest.find([';', ',']) else {
        return false;
    };
    if type_end == 0 {
        return false;
    }
    let mut after = &rest[type_end..];
    if after.starts_with(';') {
        match after.get(..8) {
            Some(base64) if base64.eq_ignore_ascii_case(";base64,") => after = &after[7..],
            _ => return false,
        }
    }
    let payload = &after[1..];
    !payload.is_empty() && !payload.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
}
