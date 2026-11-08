use std::collections::BTreeSet;

use hanafubuki_agent_tool_contracts::{
    ToolContractError, ToolOutput, ToolOutputContent, MAX_TOOL_OUTPUT_ITEMS,
};

use crate::AgentToolOutputTruncation;

pub(crate) const MINIMUM_CANONICAL_TOOL_OUTPUT_BYTES: usize = 512;

pub(crate) fn canonicalize_tool_output(
    output: ToolOutput,
    maximum_bytes: usize,
) -> (ToolOutput, Option<AgentToolOutputTruncation>) {
    match bound_tool_output(output, maximum_bytes) {
        Ok(result) => result,
        Err(error) => {
            let fallback = ToolOutput {
                content: vec![ToolOutputContent::Structured {
                    value: serde_json::json!({
                    "status": "outcome_unknown",
                    "message": "The tool ran, but its result violated the canonical output contract and could not be recorded. Do not automatically replay this call.",
                    "outputContractError": error.to_string(),
                    }),
                }],
                structured_content: None,
                is_error: true,
            };
            debug_assert!(encoded_bytes(&fallback).is_ok_and(|bytes| bytes <= maximum_bytes));
            (fallback, None)
        }
    }
}

pub(crate) fn canonicalize_agent_tool_output(
    mut output: crate::AgentToolOutput,
    maximum_bytes: usize,
) -> (crate::AgentToolOutput, Option<AgentToolOutputTruncation>) {
    let unbounded = std::mem::replace(
        &mut output.output,
        ToolOutput {
            content: Vec::new(),
            structured_content: None,
            is_error: false,
        },
    );
    let (canonical, truncation) = canonicalize_tool_output(unbounded, maximum_bytes);
    output.output = canonical;
    (output, truncation)
}

pub(crate) fn bound_tool_output(
    output: ToolOutput,
    maximum_bytes: usize,
) -> Result<(ToolOutput, Option<AgentToolOutputTruncation>), ToolContractError> {
    output.validate()?;
    let original_bytes = encoded_bytes(&output)?;
    if original_bytes <= maximum_bytes {
        return Ok((output, None));
    }

    let mut original_items = output.content.clone();
    if original_items.is_empty() {
        if let Some(value) = &output.structured_content {
            original_items.push(ToolOutputContent::Structured {
                value: value.clone(),
            });
        }
    }
    let mut retained = vec![None; original_items.len()];
    let mut fully_retained = vec![false; original_items.len()];

    for index in original_items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| is_binary_content(item).then_some(index))
    {
        try_retain_whole(
            index,
            &original_items,
            &mut retained,
            &mut fully_retained,
            output.is_error,
            output.structured_content.is_some(),
            original_bytes,
            maximum_bytes,
        )?;
    }

    for index in 0..original_items.len() {
        if fully_retained[index] {
            continue;
        }
        if try_retain_whole(
            index,
            &original_items,
            &mut retained,
            &mut fully_retained,
            output.is_error,
            output.structured_content.is_some(),
            original_bytes,
            maximum_bytes,
        )? {
            continue;
        }
        retain_largest_preview(
            index,
            &original_items,
            &mut retained,
            &fully_retained,
            output.is_error,
            output.structured_content.is_some(),
            original_bytes,
            maximum_bytes,
        )?;
    }

    let (bounded, truncation) = build_output(
        &original_items,
        &retained,
        &fully_retained,
        output.is_error,
        output.structured_content.is_some(),
        original_bytes,
    )?;
    if encoded_bytes(&bounded)? > maximum_bytes {
        return Err(ToolContractError::OutputTooLarge);
    }
    bounded.validate()?;
    Ok((bounded, Some(truncation)))
}

#[allow(clippy::too_many_arguments)]
fn try_retain_whole(
    index: usize,
    original_items: &[ToolOutputContent],
    retained: &mut [Option<ToolOutputContent>],
    fully_retained: &mut [bool],
    is_error: bool,
    had_structured_content: bool,
    original_bytes: usize,
    maximum_bytes: usize,
) -> Result<bool, ToolContractError> {
    let previous = retained[index].replace(original_items[index].clone());
    fully_retained[index] = true;
    let fits = candidate_fits(
        original_items,
        retained,
        fully_retained,
        is_error,
        had_structured_content,
        original_bytes,
        maximum_bytes,
    )?;
    if !fits {
        retained[index] = previous;
        fully_retained[index] = false;
    }
    Ok(fits)
}

#[allow(clippy::too_many_arguments)]
fn retain_largest_preview(
    index: usize,
    original_items: &[ToolOutputContent],
    retained: &mut [Option<ToolOutputContent>],
    fully_retained: &[bool],
    is_error: bool,
    had_structured_content: bool,
    original_bytes: usize,
    maximum_bytes: usize,
) -> Result<(), ToolContractError> {
    let Some(source) = preview_source(&original_items[index]) else {
        return Ok(());
    };
    let mut low = 0usize;
    let mut high = source.len();
    let mut best = None;
    while low <= high {
        let middle = low.saturating_add(high.saturating_sub(low) / 2);
        let preview = preview_content(&original_items[index], &source, middle);
        retained[index] = preview.clone();
        if candidate_fits(
            original_items,
            retained,
            fully_retained,
            is_error,
            had_structured_content,
            original_bytes,
            maximum_bytes,
        )? {
            best = preview;
            low = middle.saturating_add(1);
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
        }
    }
    retained[index] = best;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn candidate_fits(
    original_items: &[ToolOutputContent],
    retained: &[Option<ToolOutputContent>],
    fully_retained: &[bool],
    is_error: bool,
    had_structured_content: bool,
    original_bytes: usize,
    maximum_bytes: usize,
) -> Result<bool, ToolContractError> {
    if retained.iter().flatten().count() >= MAX_TOOL_OUTPUT_ITEMS {
        return Ok(false);
    }
    let (candidate, _) = build_output(
        original_items,
        retained,
        fully_retained,
        is_error,
        had_structured_content,
        original_bytes,
    )?;
    Ok(encoded_bytes(&candidate)? <= maximum_bytes)
}

fn build_output(
    original_items: &[ToolOutputContent],
    retained: &[Option<ToolOutputContent>],
    fully_retained: &[bool],
    is_error: bool,
    had_structured_content: bool,
    original_bytes: usize,
) -> Result<(ToolOutput, AgentToolOutputTruncation), ToolContractError> {
    let mut omitted_content_types = BTreeSet::new();
    let mut omitted_items = 0usize;
    for (index, item) in original_items.iter().enumerate() {
        if !fully_retained[index] {
            omitted_items = omitted_items.saturating_add(1);
            omitted_content_types.insert(content_kind(item).to_string());
        }
    }
    if had_structured_content {
        omitted_items = omitted_items.saturating_add(1);
        omitted_content_types.insert("structured_content".to_string());
    }
    let omitted_content_types = omitted_content_types.into_iter().collect::<Vec<_>>();
    let mut output = ToolOutput {
        content: retained.iter().flatten().cloned().collect(),
        structured_content: None,
        is_error,
    };
    output.content.push(ToolOutputContent::Text {
        text: String::new(),
    });
    let marker_index = output.content.len() - 1;
    let mut truncation = AgentToolOutputTruncation {
        original_bytes: usize_to_u64(original_bytes),
        retained_bytes: 0,
        omitted_bytes: usize_to_u64(original_bytes),
        omitted_items: u32::try_from(omitted_items).unwrap_or(u32::MAX),
        omitted_content_types,
    };
    for _ in 0..8 {
        let marker = render_marker(&truncation);
        output.content[marker_index] = ToolOutputContent::Text { text: marker };
        let retained_bytes = encoded_bytes(&output)?;
        let next = AgentToolOutputTruncation {
            retained_bytes: usize_to_u64(retained_bytes),
            omitted_bytes: usize_to_u64(original_bytes.saturating_sub(retained_bytes)),
            ..truncation.clone()
        };
        if next == truncation {
            break;
        }
        truncation = next;
    }
    output.content[marker_index] = ToolOutputContent::Text {
        text: render_marker(&truncation),
    };
    let final_bytes = encoded_bytes(&output)?;
    truncation.retained_bytes = usize_to_u64(final_bytes);
    truncation.omitted_bytes = usize_to_u64(original_bytes.saturating_sub(final_bytes));
    output.content[marker_index] = ToolOutputContent::Text {
        text: render_marker(&truncation),
    };
    Ok((output, truncation))
}

fn render_marker(truncation: &AgentToolOutputTruncation) -> String {
    format!(
        "<hanafubuki-tool-output-truncation>{{\"originalBytes\":{},\"retainedBytes\":{},\"omittedBytes\":{},\"omittedItems\":{},\"omittedContentTypes\":{}}}</hanafubuki-tool-output-truncation>",
        truncation.original_bytes,
        truncation.retained_bytes,
        truncation.omitted_bytes,
        truncation.omitted_items,
        serde_json::to_string(&truncation.omitted_content_types)
            .expect("bounded strings are serializable")
    )
}

fn preview_source(item: &ToolOutputContent) -> Option<String> {
    match item {
        ToolOutputContent::Text { text } => Some(text.clone()),
        ToolOutputContent::Structured { value } => serde_json::to_string(value).ok(),
        ToolOutputContent::EmbeddedResource {
            text: Some(text), ..
        } => Some(text.clone()),
        _ => None,
    }
}

fn preview_content(
    item: &ToolOutputContent,
    source: &str,
    retained_source_bytes: usize,
) -> Option<ToolOutputContent> {
    if retained_source_bytes == 0 {
        return None;
    }
    let preview = head_tail_preview(source, retained_source_bytes);
    match item {
        ToolOutputContent::Text { .. } => Some(ToolOutputContent::Text { text: preview }),
        ToolOutputContent::Structured { .. } => Some(ToolOutputContent::Text {
            text: format!(
                "[structured JSON preview; originalBytes={}]\n{preview}",
                source.len()
            ),
        }),
        ToolOutputContent::EmbeddedResource {
            uri,
            mime_type,
            text: Some(_),
            ..
        } => Some(ToolOutputContent::EmbeddedResource {
            uri: uri.clone(),
            mime_type: mime_type.clone(),
            text: Some(preview),
            data: None,
        }),
        _ => None,
    }
}

fn head_tail_preview(source: &str, retained_source_bytes: usize) -> String {
    if source.len() <= retained_source_bytes {
        return source.to_string();
    }
    let head_target = retained_source_bytes / 2;
    let tail_target = retained_source_bytes.saturating_sub(head_target);
    let head_end = previous_char_boundary(source, head_target);
    let tail_start = next_char_boundary(source, source.len().saturating_sub(tail_target));
    let omitted = tail_start.saturating_sub(head_end);
    format!(
        "{}\n… <{} bytes omitted> …\n{}",
        &source[..head_end],
        omitted,
        &source[tail_start..]
    )
}

fn previous_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}

fn next_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index = index.saturating_add(1);
    }
    index
}

fn is_binary_content(item: &ToolOutputContent) -> bool {
    matches!(
        item,
        ToolOutputContent::Image { .. }
            | ToolOutputContent::Audio { .. }
            | ToolOutputContent::EmbeddedResource { data: Some(_), .. }
    )
}

fn content_kind(item: &ToolOutputContent) -> &'static str {
    match item {
        ToolOutputContent::Text { .. } => "text",
        ToolOutputContent::Structured { .. } => "structured",
        ToolOutputContent::Image { .. } => "image",
        ToolOutputContent::Audio { .. } => "audio",
        ToolOutputContent::ResourceLink { .. } => "resource_link",
        ToolOutputContent::EmbeddedResource { .. } => "embedded_resource",
    }
}

fn encoded_bytes(output: &ToolOutput) -> Result<usize, ToolContractError> {
    serde_json::to_vec(output)
        .map(|encoded| encoded.len())
        .map_err(|_| ToolContractError::OutputTooLarge)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
