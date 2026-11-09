//! `/compression [cache]`: the session's hierarchical compression, read
//! through the facade's `compression_status`. Without an argument,
//! billion-context's status panel (the context, the nudge and the ranges
//! it offers, the active blocks by tier) and then the counters and the
//! content store's size; with `cache`, the cache report. A session that
//! never ran the `hierarchical` strategy says so.

use hanafubuki_agent::AgentCompressionStatus;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell::{self, format_tokens_compact, plain_lines, HistoryCell};
use crate::wrapping::{word_wrap_lines, RtOptions};

const USAGE: &str = "Usage: /compression [cache]";

pub(super) static COMPRESSION: CommandSpec = CommandSpec {
    name: "compression",
    aliases: &[],
    description: "show the hierarchical compression's blocks and savings (cache: its cache report)",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let cache = match args.trim() {
        "" => false,
        "cache" => true,
        _ => {
            context.add_to_history(history_cell::new_error_event(USAGE.to_string()));
            return;
        }
    };
    context.session_call(SessionCall::Compression { cache });
}

pub(super) fn on_reply(
    context: &mut CommandContext<'_>,
    status: Option<AgentCompressionStatus>,
    cache: bool,
) {
    match status {
        None => context.add_to_history(history_cell::new_info_event(
            "This session is not using hierarchical compression.".to_string(),
            Some("([compaction] strategy = \"hierarchical\" turns it on)".to_string()),
        )),
        Some(status) if cache => context.add_to_history(CompressionCell::cache(&status)),
        Some(status) => context.add_to_history(CompressionCell::panel(&status)),
    }
}

/// A report as text, its lines wrapped at the width with a hanging indent.
#[derive(Debug)]
pub(super) struct CompressionCell {
    title: &'static str,
    lines: Vec<String>,
}

impl CompressionCell {
    /// The panel, then the counters and the content store's size.
    pub(super) fn panel(status: &AgentCompressionStatus) -> Self {
        let stats = &status.stats;
        let mut lines: Vec<String> = status.panel.lines().map(str::to_string).collect();
        lines.push(String::new());
        lines.push(format!(
            "Stats: {} compress call(s) applied · {} tokens compressed · {} tokens absorbed · {} result(s) stored · {} retrieval(s)",
            stats.compression_count,
            format_tokens_compact(stats.tokens_compressed),
            format_tokens_compact(stats.absorbed_tokens),
            stats.stored_count,
            stats.retrieval_count,
        ));
        if let Some(store) = &status.content_store {
            lines.push(format!(
                "Content store: {} entr{} ({} unique, {} characters)",
                store.entries,
                if store.entries == 1 { "y" } else { "ies" },
                store.unique_contents,
                store.total_chars,
            ));
        }
        Self {
            title: "Compression",
            lines,
        }
    }

    pub(super) fn cache(status: &AgentCompressionStatus) -> Self {
        Self {
            title: "Compression cache",
            lines: status.cache_report.lines().map(str::to_string).collect(),
        }
    }

    fn source_lines(&self) -> Vec<Line<'static>> {
        std::iter::once(Line::from(self.title).bold())
            .chain(self.lines.iter().map(|line| Line::from(line.clone())))
            .collect()
    }
}

impl HistoryCell for CompressionCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let options = RtOptions::new(usize::from(width.max(1)))
            .initial_indent(Line::from("  "))
            .subsequent_indent(Line::from("    "));
        self.source_lines()
            .into_iter()
            .flat_map(|line| word_wrap_lines([line], options.clone()))
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.source_lines())
    }
}

#[cfg(test)]
mod tests;
