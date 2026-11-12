//! A directory listing (`list_directory`). Collapsed: `Listed`, the path
//! and how many entries. Expanded: the entries, directories first.

use hanafubuki_agent::{AgentListingEntry, AgentListingEntryKind};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{counted, wrapped, Extent, ToolCall, View};
use crate::history_cell::ActivityDisclosure;
use crate::text_formatting::strip_terminal_controls;

pub(super) fn view(
    call: &ToolCall,
    path: &str,
    entries: &[AgentListingEntry],
    truncated: bool,
    extent: Extent,
) -> View {
    let mut amount = counted(entries.len(), "entry", "entries");
    if truncated {
        amount.push_str(", more not listed");
    }
    let title = vec![
        "Listed".bold(),
        " ".into(),
        call.subject_or(path).into(),
        format!(" ({amount})").dim(),
    ];
    let directories = entries
        .iter()
        .filter(|entry| entry.kind == AgentListingEntryKind::Directory);
    let files = entries
        .iter()
        .filter(|entry| entry.kind == AgentListingEntryKind::File);
    let mut details = Vec::new();
    for entry in directories.chain(files).filter(|_| extent.expanded) {
        let path = strip_terminal_controls(&entry.path);
        let line = match entry.kind {
            AgentListingEntryKind::Directory => Line::from(format!("{path}/").cyan()),
            AgentListingEntryKind::File => Line::from(vec![
                path.into(),
                entry
                    .size
                    .map(|size| format!("  {}", size_label(size)))
                    .unwrap_or_default()
                    .dim(),
            ]),
        };
        details.extend(wrapped(line, extent.columns, "  "));
    }
    View {
        failed: false,
        title,
        preview: Vec::new(),
        details,
        hidden: (!entries.is_empty()).then_some(ActivityDisclosure::OutputLines(entries.len())),
    }
}

/// `42 B`, `1.5 KiB`, `3.0 MiB`.
fn size_label(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let size = bytes as f64;
    if size < KIB {
        format!("{bytes} B")
    } else if size < KIB * KIB {
        format!("{:.1} KiB", size / KIB)
    } else {
        format!("{:.1} MiB", size / (KIB * KIB))
    }
}
