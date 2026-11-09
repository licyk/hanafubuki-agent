// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `resolveDecompress`, `toFilePointer`, `refoldHint`, `coveredMessages`,
// `resolveDecompressRange` and `resolveImageRestore` in
// `src/decompress-shared.ts`, and `describeRestorable` in
// `src/image-restore.ts`, at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the Agent keeps the full history, so a
// range is restored from it with no content store and returned as the
// call's output (no injection channel); `toFile` asks for the session's
// export file instead of naming a path, and nothing is spilled to a file
// unasked; images come back attached to the output, from the history's own
// attachments, instead of as files.

//! `decompress`: a block's content inline, to a file, a range of it, or the
//! images of a message.

use std::collections::{HashMap, HashSet};

use hanafubuki_agent_compression::{
    collect_block_content, index_to_ref, mark_block_restored_inline, parse_block_id_arg,
    parse_boundary, Block, BoundaryKind, InlineRestore, Kind, Message,
};

use super::*;

/// How many images `imageRef: "list"` names at most.
const LISTED_IMAGES: usize = 50;

pub(super) fn decompress(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    arguments: &Value,
    images_supported: bool,
) -> CompressionToolRun {
    // An image restore is chosen by a non-blank `imageRef` alone: models
    // send "" or null for the optional fields they do not mean.
    match arguments.get("imageRef") {
        None | Some(Value::Null) => {}
        Some(Value::String(raw)) if raw.trim().is_empty() => {}
        Some(Value::String(raw)) => {
            return restore_images(log, state, raw.trim(), images_supported)
        }
        Some(_) => return failed(
            "[decompress FAILED: imageRef must be a string (an mNNNNN ref or \"list\") — omit it to restore a text block]"
                .to_owned(),
        ),
    }
    let Some(raw) = arguments
        .get("blockId")
        .and_then(Value::as_str)
        .filter(|raw| !raw.trim().is_empty())
    else {
        return failed("[decompress FAILED: blockId is required]".to_owned());
    };
    let raw = raw.trim();
    let Some(block) = parse_block_id_arg(raw).and_then(|id| state.block(&id)) else {
        return failed(format!("[Block {raw} not found]"));
    };
    let to_file = arguments
        .get("toFile")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty());
    let start = blank_to_none(arguments.get("startId"));
    let end = blank_to_none(arguments.get("endId"));
    if start.is_some() || end.is_some() {
        return restore_range(log, state, block, (start, end), to_file, images_supported);
    }
    let full = matches!(arguments.get("full"), Some(Value::Bool(true)))
        || arguments.get("full").and_then(Value::as_str) == Some("true");
    restore_block(log, state, block, full, to_file, images_supported)
}

/// The whole block, one tier up or `full`. An inline restore of an active
/// block flags it restored inline, so a later `compress` over its span
/// refolds it in place; a file or an inactive block changes nothing.
fn restore_block(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    block: &Block,
    full: bool,
    to_file: bool,
    images_supported: bool,
) -> CompressionToolRun {
    let block_id = block.block_id.clone();
    let collected = collect_block_content(state, block, &log.messages, full);
    let body = if collected.text.is_empty() {
        block.summary.clone()
    } else {
        collected.text
    };
    let header = format!(
        "[Block {block_id} content — {} item(s){}]",
        collected.count,
        if full { ", full" } else { "" }
    );
    if to_file {
        let suffix = if full { "-full" } else { "" };
        return exported(
            format!("decompress-{}{suffix}.txt", safe(&block_id)),
            header,
            body,
        );
    }
    // Every original comes back in full, its images too.
    let images = if full && images_supported {
        images_of(log, block.effective_message_ids.iter().map(String::as_str))
    } else {
        Vec::new()
    };
    if !block.active {
        return with_images(
            text_output(format!("{header}\n{body}"), false),
            None,
            images,
        );
    }
    // The content is now in the conversation: a later compress over the same
    // span refolds the block in place. Only the first restore changes the
    // state; the hint is the same for every repeat.
    let first_restore = !block.restored_inline;
    let mut restored = state.clone();
    let marked = mark_block_restored_inline(&mut restored, &block_id);
    with_images(
        text_output(
            format!("{header}\n{body}\n\n{}", refold_hint(&block_id, marked)),
            false,
        ),
        first_restore.then_some(restored),
        images,
    )
}

fn refold_hint(block_id: &str, restore: Option<InlineRestore>) -> String {
    match restore.and_then(|restore| restore.restored_start_ref.zip(restore.restored_end_ref)) {
        Some((start, end)) => format!(
            "Re-fold: call compress(\"{start}–{end}\", <fresh summary>) → updates block {block_id} in place (same id, new summary)."
        ),
        None => format!(
            "Re-fold: call compress over the restored messages with a fresh summary → updates block {block_id} in place (same id, new summary)."
        ),
    }
}

/// Only the block's messages whose refs fall within the range, read from
/// the history; the block stays as it is.
fn restore_range(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    block: &Block,
    (start, end): (Option<&str>, Option<&str>),
    to_file: bool,
    images_supported: bool,
) -> CompressionToolRun {
    let (Some(start), Some(end)) = (start, end) else {
        return failed("[decompress FAILED: startId and endId must be given together — pass both mNNNNN refs, or omit both to restore the whole block]".to_owned());
    };
    let message_number = |raw: &str| {
        parse_boundary(raw)
            .filter(|boundary| boundary.kind == BoundaryKind::Message)
            .map(|boundary| boundary.numeric_id)
    };
    let (Some(first), Some(last)) = (message_number(start), message_number(end)) else {
        return failed(format!("[decompress FAILED: startId/endId must be mNNNNN message refs (got \"{start}\", \"{end}\") — block ids (bN) are not valid here]"));
    };
    if first > last {
        return failed(format!(
            "[decompress FAILED: startId {start} is after endId {end} — swap them]"
        ));
    }
    let Some(covered) = covered_messages(state, block) else {
        return failed(format!("[decompress FAILED: {} has no per-message coverage recorded (older block) — use plain decompress {} for the whole block]", block.block_id, block.block_id));
    };
    let picked = covered
        .raws
        .iter()
        .filter(|(_, number)| (first..=last).contains(number))
        .map(|(raw, _)| raw.as_str())
        .collect::<Vec<_>>();
    if picked.is_empty() {
        return failed(format!(
            "[decompress FAILED: {} covers no messages in {start}–{end} (its coverage is {})]",
            block.block_id, covered.text
        ));
    }
    let messages: HashMap<&str, &Message> = log
        .messages
        .iter()
        .map(|message| (message.id.as_str(), message))
        .collect();
    let parts = picked
        .iter()
        .filter_map(|raw| messages.get(raw))
        .map(|message| format_message(message))
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return failed(format!("[decompress FAILED: originals for {start}–{end} are no longer in this session's context window — whole-block decompress may still return the block's summary]"));
    }
    let header = format!(
        "[Block {} content — {start}–{end} — {} item(s)]",
        block.block_id,
        parts.len()
    );
    let body = parts.join("\n\n");
    if to_file {
        return exported(
            format!(
                "decompress-{}-{}-{}.txt",
                safe(&block.block_id),
                safe(start),
                safe(end)
            ),
            header,
            body,
        );
    }
    let images = if images_supported {
        images_of(log, picked)
    } else {
        Vec::new()
    };
    with_images(
        text_output(format!("{header}\n{body}"), false),
        None,
        images,
    )
}

/// A message's images, by its ref or by a handle a summary's MEDIA REFS
/// lists, attached to the output; or `list`, what can be restored.
fn restore_images(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    raw: &str,
    supported: bool,
) -> CompressionToolRun {
    let by_message = log.images_by_message();
    if raw.eq_ignore_ascii_case("list") {
        return read_only(list_images(state, &by_message));
    }
    let message_ref = parse_boundary(raw)
        .filter(|boundary| boundary.kind == BoundaryKind::Message)
        .and_then(|boundary| index_to_ref(boundary.numeric_id));
    let images = match message_ref {
        Some(message_ref) => state
            .message_refs
            .by_ref
            .get(&message_ref)
            .and_then(|id| by_message.get(id.as_str()))
            .map(|images| images.to_vec())
            .unwrap_or_default(),
        None => by_message
            .values()
            .flat_map(|images| images.iter())
            .find(|image| image.attachment_id.as_str().eq_ignore_ascii_case(raw))
            .cloned()
            .into_iter()
            .collect(),
    };
    if images.is_empty() {
        return failed(format!("[decompress FAILED: no restorable image for ref \"{raw}\" (it carries no image, or is not in this session's context window). Call decompress({{ imageRef: \"list\" }}) to see what is available.]"));
    }
    if !supported {
        return failed(format!("[decompress FAILED: the current model takes no image input — the image(s) for \"{raw}\" cannot be shown to it.]"));
    }
    with_images(
        text_output(
            format!(
                "[Restored {} image(s) for {raw} — attached to this result.]",
                images.len()
            ),
            false,
        ),
        None,
        images,
    )
}

fn list_images(
    state: &AgentCompressionState,
    by_message: &HashMap<&str, &[AgentImageAttachmentRef]>,
) -> String {
    let mut listed = by_message
        .iter()
        .filter_map(|(id, images)| {
            let message_ref = state.message_refs.by_raw.get(*id)?;
            let number = parse_boundary(message_ref)
                .filter(|boundary| boundary.kind == BoundaryKind::Message)?
                .numeric_id;
            Some((number, message_ref, *images))
        })
        .collect::<Vec<_>>();
    if listed.is_empty() {
        return "[No restorable images right now — no message in this session's context window carries an image.]".to_owned();
    }
    listed.sort_by_key(|(number, ..)| *number);
    let lines = listed
        .into_iter()
        .flat_map(|(_, message_ref, images)| {
            images.iter().enumerate().map(move |(index, image)| {
                let kind = image
                    .mime_type
                    .rsplit_once('/')
                    .map_or(image.mime_type.as_str(), |(_, kind)| kind);
                let kilobytes = ((image.byte_length as f64 / 1024.0).round() as u64).max(1);
                let suffix = if index > 0 {
                    format!("[-{index}]")
                } else {
                    String::new()
                };
                format!(
                    "{message_ref}{suffix} [{kind} {}x{} · {kilobytes}KB]",
                    image.width, image.height
                )
            })
        })
        .take(LISTED_IMAGES)
        .collect::<Vec<_>>();
    format!(
        "[Restorable images ({}):]\n{}\nRestore one with decompress({{ imageRef: \"<ref>\" }}); its pixels are attached to the result.",
        lines.len(),
        lines.join("\n")
    )
}

/// The refs a block covers, by ref number, and as compact text: runs of
/// consecutive refs (`m00044–m00097`), the first five and a count of the
/// rest.
pub(super) struct CoveredRefs {
    pub(super) raws: Vec<(String, u32)>,
    pub(super) text: String,
}

pub(super) fn covered_messages(
    state: &AgentCompressionState,
    block: &Block,
) -> Option<CoveredRefs> {
    let mut raws = block
        .effective_message_ids
        .iter()
        .filter_map(|raw| {
            let number = parse_boundary(state.message_refs.by_raw.get(raw)?)
                .filter(|boundary| boundary.kind == BoundaryKind::Message)?
                .numeric_id;
            Some((raw.clone(), number))
        })
        .collect::<Vec<_>>();
    if raws.is_empty() {
        return None;
    }
    raws.sort_by_key(|(_, number)| *number);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for (_, number) in &raws {
        match runs.last_mut() {
            Some(run) if *number == run.1 + 1 => run.1 = *number,
            _ => runs.push((*number, *number)),
        }
    }
    let head = &runs[..runs.len().min(5)];
    let shown = head
        .iter()
        .map(|(first, last)| (last - first + 1) as usize)
        .sum::<usize>();
    let rest = raws.len().saturating_sub(shown);
    let mut text = head
        .iter()
        .map(|(first, last)| {
            if first == last {
                format!("m{first:05}")
            } else {
                format!("m{first:05}–m{last:05}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    if rest > 0 {
        text.push_str(&format!(" …+{rest} more"));
    }
    Some(CoveredRefs { raws, text })
}

/// The images of the messages `ids` names, in history order, each once.
fn images_of<'a>(
    log: &MessageLog<'_>,
    ids: impl IntoIterator<Item = &'a str>,
) -> Vec<AgentImageAttachmentRef> {
    let ids = ids.into_iter().collect::<HashSet<_>>();
    let by_message = log.images_by_message();
    let mut images: Vec<AgentImageAttachmentRef> = Vec::new();
    for message in log
        .messages
        .iter()
        .filter(|message| ids.contains(message.id.as_str()))
    {
        for image in by_message
            .get(message.id.as_str())
            .copied()
            .unwrap_or_default()
        {
            if !images.contains(image) {
                images.push(image.clone());
            }
        }
    }
    images
}

fn format_message(message: &Message) -> String {
    match message
        .tool_name
        .as_deref()
        .filter(|name| !name.is_empty() && message.kind != Kind::Text)
    {
        Some(tool) => format!("[{} • {tool}]\n{}", message.role.name(), message.text),
        None => format!("[{}]\n{}", message.role.name(), message.text),
    }
}

fn exported(file_name: String, header: String, body: String) -> CompressionToolRun {
    CompressionToolRun {
        export: Some(ToolExport::restore(file_name, header, body)),
        ..output_run(text_output(String::new(), false), None)
    }
}

fn with_images(
    output: ToolOutput,
    state: Option<AgentCompressionState>,
    images: Vec<AgentImageAttachmentRef>,
) -> CompressionToolRun {
    CompressionToolRun {
        images,
        ..output_run(output, state)
    }
}

fn failed(text: String) -> CompressionToolRun {
    output_run(text_output(text, true), None)
}

fn blank_to_none(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// `text` as part of a file name, as billion-context's host makes a block id
/// one.
fn safe(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect()
}
