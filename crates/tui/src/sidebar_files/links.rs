// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! File destinations carried by Markdown, source locations and conversation items.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use hanafubuki_agent::{ConversationItem, PathKey};
use pulldown_cmark::{Event, Parser, Tag};

use super::{FileEntry, FileTarget, Location};

impl FileTarget {
    pub(crate) fn link(destination: &str) -> Option<Self> {
        if destination.is_empty()
            || destination.starts_with('#')
            || destination.contains(char::is_control)
        {
            return None;
        }
        let windows_drive = matches!(destination.as_bytes(), [drive,b':',sep,..] if drive.is_ascii_alphabetic() && matches!(sep,b'/'|b'\\'));
        if destination.starts_with("//")
            || (destination.contains("://") && !destination.starts_with("file://"))
            || (!windows_drive
                && !destination.starts_with("file://")
                && crate::markdown_render::local_links::colon_location_suffix(destination)
                    .is_none()
                && url::Url::parse(destination).is_ok())
        {
            return None;
        }
        let (path, location) =
            crate::markdown_render::local_links::parse_local_link_target(destination)?;
        if path.contains(char::is_control) {
            return None;
        }
        let line = location.as_deref().and_then(|v| {
            let mut point = v
                .trim_start_matches(':')
                .split(['-', '–'])
                .next()?
                .split(':');
            Some(Location {
                line: point.next()?.parse().ok()?,
                column: point.next().and_then(|v| v.parse().ok()),
            })
        });
        let path = if let Some(suffix) = path.strip_prefix("~/") {
            std::env::home_dir()?.join(suffix)
        } else {
            PathBuf::from(path)
        };
        Some(Self::Document { path, line })
    }
}

pub(crate) fn references(
    item: ConversationItem,
    root: &Path,
    entries: &mut Vec<FileEntry>,
    seen: &mut HashSet<PathKey>,
    preview_patch: impl FnOnce(&str) -> hanafubuki_agent::PatchPreview,
) {
    let mut record = |target: FileTarget| {
        let FileTarget::Document { path, line } = target else {
            return;
        };
        let path = hanafubuki_agent::normalize_path(root.join(path));
        let Ok(path) = path else { return };
        if seen.insert(PathKey::of(&path)) {
            let label =
                hanafubuki_agent::relative_below(&path, root).unwrap_or_else(|| path.clone());
            entries.push(FileEntry {
                label: label.to_string_lossy().into_owned(),
                target: FileTarget::Document { path, line },
            });
        }
    };
    match item {
        ConversationItem::ToolCall {
            name,
            namespace,
            arguments,
            ..
        } => {
            if namespace.is_none() && name == "apply_patch" {
                if let Some(patch) = arguments
                    .as_str()
                    .or_else(|| arguments.get("patch").and_then(|v| v.as_str()))
                {
                    for file in preview_patch(patch).files {
                        if file.operation != hanafubuki_agent::PatchPreviewOperation::Delete {
                            record(FileTarget::document(
                                file.moved_to.unwrap_or(file.path).into(),
                            ));
                        }
                    }
                }
            }
            for key in ["path", "file_path", "file", "destination", "source"] {
                if let Some(path) = arguments.get(key).and_then(|v| v.as_str()) {
                    record(FileTarget::document(path.into()));
                }
            }
        }
        ConversationItem::AssistantMessage { text, .. }
        | ConversationItem::UserMessage { text, .. }
        | ConversationItem::SteeringMessage { text, .. }
        | ConversationItem::Reasoning { text, .. } => {
            for event in Parser::new(&text) {
                let target = match event {
                    Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                        FileTarget::link(&dest_url)
                    }
                    Event::Code(path)
                        if (path.contains('/')
                            || Path::new(path.as_ref()).extension().is_some())
                            && !path.contains(char::is_whitespace) =>
                    {
                        FileTarget::link(&path)
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    record(target)
                }
            }
        }
        _ => {}
    }
}
