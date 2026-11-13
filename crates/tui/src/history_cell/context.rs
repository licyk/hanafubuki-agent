//! `/context`'s card: what the session's last request held, group by group,
//! laid out as billion-context's panel (`kernel/src/panel/panel.ts`) lays
//! out its breakdown: a row per group with a bar of its share of the
//! window, its percent and its tokens, then the window's free space, then
//! the prompt cache. The bars, the token counts and the hit rates are the
//! panel's own helpers (`hanafubuki_agent::panel`).

use hanafubuki_agent::panel::{
    bar, cache_hit_stats, format_compact_tokens, format_hit_rate, CacheUsageSample,
};
use hanafubuki_agent::{
    AgentContextComposition, AgentContextRequest, AgentModelUsage, AgentReportedPrompt,
    AgentSessionContext, AgentUsageSource,
};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{plain_lines, HistoryCell};
use crate::wrapping::{word_wrap_lines, RtOptions};

const INDENT: &str = "  ";
/// The widest bar, the panel's.
const BAR_WIDTH: usize = 20;
/// A bar narrower than this is left out.
const MINIMUM_BAR_WIDTH: usize = 4;
const LONGEST_LABEL: usize = 28;
/// A label is cut to this, at the least, before the bar narrows.
const SHORT_LABEL: usize = 22;
const SHORTEST_LABEL: usize = 10;
/// The columns of a row besides its label and its bar: the indent, a space
/// after the label and one after the bar, ` 42%`, two spaces and the
/// tokens.
const ROW_FIXED: usize = INDENT.len() + 1 + 1 + 4 + 2 + TOKENS_WIDTH;
const TOKENS_WIDTH: usize = 5;

/// The prefix of the provider ids MCP servers' tools and installed
/// plugins' tools take (`mcp.SERVER`, `plugin.NAME.MARKETPLACE`).
const EXTERNAL_TOOL_PROVIDERS: [&str; 2] = ["mcp.", "plugin."];

#[derive(Debug)]
pub(crate) struct ContextHistoryCell {
    /// What the figures are of, wrapped at the width.
    heading: Vec<String>,
    rows: Vec<Row>,
    /// What a row's share is of: the window, else the request.
    scale: u64,
    cache: Vec<String>,
}

#[derive(Debug)]
struct Row {
    label: String,
    tokens: u64,
}

pub(crate) fn new_context_output(context: &AgentSessionContext) -> ContextHistoryCell {
    let window = context.context_window_tokens.filter(|window| *window > 0);
    let (mut heading, composition, reported) = match &context.request {
        AgentContextRequest::Sent(sent) => {
            let provider = match sent.reported {
                Some(reported) => format!(
                    "the Provider counted {}",
                    format_compact_tokens(reported.input_tokens)
                ),
                None => "no count from the Provider".to_string(),
            };
            (
                vec![format!(
                    "Last request: about {} tokens (estimated: about 4 bytes a token) · {provider}",
                    format_compact_tokens(sent.composition.total_tokens)
                )],
                Some(&sent.composition),
                Some(sent.reported),
            )
        }
        AgentContextRequest::Next(composition) => (
            vec![
                format!(
                    "No request sent yet. The next: about {} tokens (estimated: about 4 bytes a token)",
                    format_compact_tokens(composition.total_tokens)
                ),
                "Built outside a sampling step, it leaves out what plugins prepare for one, and outside a turn the context sections.".to_string(),
            ],
            Some(composition),
            None,
        ),
        AgentContextRequest::Unavailable(reason) => (
            vec![format!(
                "No request sent yet, and the next could not be estimated: {reason}"
            )],
            None,
            None,
        ),
    };
    heading.push(match window {
        Some(window) => format!("Context window: {}", format_compact_tokens(window)),
        None => "Context window: unknown; each share is of the request".to_string(),
    });
    let total = composition.map_or(0, |composition| composition.total_tokens);
    ContextHistoryCell {
        heading,
        rows: composition.map_or_else(Vec::new, |composition| rows(composition, window)),
        scale: window.unwrap_or(total),
        cache: cache_lines(reported, context.usage.as_ref()),
    }
}

/// A row per group that holds something, in the request's order, and the
/// free space when the window is known.
fn rows(composition: &AgentContextComposition, window: Option<u64>) -> Vec<Row> {
    let row = |label: &str, tokens| Row {
        label: label.to_string(),
        tokens,
    };
    let (mut tools, mut external) = ((0, 0), (0, 0));
    for group in &composition.tools {
        let provider = group.provider_id.as_deref().unwrap_or_default();
        let sum = if EXTERNAL_TOOL_PROVIDERS
            .iter()
            .any(|prefix| provider.starts_with(prefix))
        {
            &mut external
        } else {
            &mut tools
        };
        sum.0 += group.count;
        sum.1 += group.tokens;
    }
    let mut rows = vec![
        row("Instructions", composition.instructions_tokens),
        row(&format!("Tools ({})", tools.0), tools.1),
        row(&format!("MCP/plugin tools ({})", external.0), external.1),
        row(
            "Context sections",
            composition
                .sections
                .iter()
                .map(|section| section.tokens)
                .sum(),
        ),
    ];
    rows.extend(
        composition
            .sections
            .iter()
            .map(|section| row(&format!("  {}", section.kind), section.tokens)),
    );
    rows.extend([
        row("User messages", composition.user_messages_tokens),
        row("Assistant messages", composition.assistant_messages_tokens),
        row("Reasoning", composition.reasoning_tokens),
        row("Tool calls and outputs", composition.tool_calls_tokens),
        row("Continuation context", composition.continuation_tokens),
        row(
            "Compaction summaries",
            composition.compaction_summaries_tokens,
        ),
        row(
            &format!("Images ({})", composition.images),
            composition.image_tokens,
        ),
        row("Other request fields", composition.other_tokens),
    ]);
    rows.retain(|row| row.tokens > 0);
    if let Some(window) = window {
        rows.push(row("Free", window.saturating_sub(composition.total_tokens)));
    }
    rows
}

/// The cached and uncached tokens and the hit rate, of the last request
/// (`reported`: `None` with no request sent, `Some(None)` before or
/// without the Provider's count) and of the session (its usage). A figure
/// the Provider does not report is "not reported", never 0%.
fn cache_lines(
    reported: Option<Option<AgentReportedPrompt>>,
    usage: Option<&AgentModelUsage>,
) -> Vec<String> {
    let last = match reported {
        None => Err("(no request yet)"),
        Some(reported) => reported
            .filter(|prompt| {
                prompt.cache_read_tokens.is_some() || prompt.cache_write_tokens.is_some()
            })
            .map(|prompt| {
                (
                    prompt.input_tokens,
                    CacheUsageSample {
                        input: prompt.input_tokens.saturating_sub(
                            prompt.cache_read_tokens.unwrap_or(0)
                                + prompt.cache_write_tokens.unwrap_or(0),
                        ),
                        cache_read: prompt.cache_read_tokens.unwrap_or(0),
                        cache_write: prompt.cache_write_tokens.unwrap_or(0),
                    },
                )
            })
            .ok_or("not reported"),
    };
    let session = usage
        .filter(|usage| {
            usage.source == AgentUsageSource::ProviderReported
                && usage.cache_reporting_responses > 0
        })
        .map(|usage| {
            (
                usage.cumulative_input_tokens,
                CacheUsageSample {
                    input: usage.cumulative_input_tokens.saturating_sub(
                        usage.cumulative_cache_read_tokens + usage.cumulative_cache_write_tokens,
                    ),
                    cache_read: usage.cumulative_cache_read_tokens,
                    cache_write: usage.cumulative_cache_write_tokens,
                },
            )
        })
        .ok_or("not reported");
    let figure = |of: Result<(u64, CacheUsageSample), &'static str>,
                  value: fn(u64, &CacheUsageSample) -> String| {
        of.map_or_else(str::to_string, |(prompt, sample)| value(prompt, &sample))
    };
    let cached = |_: u64, sample: &CacheUsageSample| format_compact_tokens(sample.cache_read);
    let uncached = |prompt: u64, sample: &CacheUsageSample| {
        format_compact_tokens(prompt.saturating_sub(sample.cache_read))
    };
    // A sample without a read or a write is no cache activity to the
    // panel's statistics, which leave it out; reported, it is a 0% rate.
    let rate = |_: u64, sample: &CacheUsageSample| {
        format_hit_rate(cache_hit_stats(&[*sample]).last.unwrap_or(0.0))
    };
    let mut hit_rate = format!(
        "Cache hit rate: last {} · session {}",
        figure(last, rate),
        figure(session, rate)
    );
    if let (Ok((prompt, sample)), Some(usage)) = (session, usage) {
        hit_rate.push_str(&format!(
            " — {} of {} prompt tokens from cache ({} request{})",
            format_compact_tokens(sample.cache_read),
            format_compact_tokens(prompt),
            usage.cache_reporting_responses,
            if usage.cache_reporting_responses == 1 {
                ""
            } else {
                "s"
            },
        ));
    }
    vec![
        format!(
            "Cached tokens: last {} · session {}",
            figure(last, cached),
            figure(session, cached)
        ),
        format!(
            "Uncached tokens: last {} · session {}",
            figure(last, uncached),
            figure(session, uncached)
        ),
        hit_rate,
    ]
}

impl ContextHistoryCell {
    /// The rows at `width`: each label in a column as wide as the longest,
    /// the bar as wide as what remains up to the panel's 20. A narrow width
    /// cuts the labels (with `…`) to 22 columns first, then narrows the bar,
    /// then cuts the labels further; a bar under 4 cells is left out.
    fn row_lines(&self, width: usize) -> Vec<Line<'static>> {
        let room = |label: usize| width.saturating_sub(ROW_FIXED + label);
        let mut label_width = self
            .rows
            .iter()
            .map(|row| row.label.width())
            .max()
            .unwrap_or(0)
            .min(LONGEST_LABEL);
        if room(label_width) < BAR_WIDTH {
            label_width = label_width.min(room(0).saturating_sub(BAR_WIDTH).max(SHORT_LABEL));
        }
        if room(label_width) < MINIMUM_BAR_WIDTH {
            label_width = label_width.min(
                room(0)
                    .saturating_sub(MINIMUM_BAR_WIDTH)
                    .max(SHORTEST_LABEL),
            );
        }
        let bar_width = Some(room(label_width).min(BAR_WIDTH))
            .filter(|bar_width| *bar_width >= MINIMUM_BAR_WIDTH)
            .unwrap_or(0);
        self.rows
            .iter()
            .map(|row| {
                let label = fit(&row.label, label_width);
                let padding = " ".repeat(label_width.saturating_sub(label.width()));
                let percent = percent_of(row.tokens, self.scale);
                let mut spans = vec![Span::from(format!("{INDENT}{label}{padding} "))];
                if bar_width > 0 {
                    spans.push(Span::from(bar(row.tokens, self.scale, bar_width)).cyan());
                    spans.push(Span::from(" "));
                }
                spans.push(Span::from(format!("{percent:>3}%")).dim());
                spans.push(Span::from(format!(
                    "  {:>TOKENS_WIDTH$}",
                    format_compact_tokens(row.tokens)
                )));
                Line::from(spans)
            })
            .collect()
    }
}

impl HistoryCell for ContextHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let width = usize::from(width.max(1));
        let wrap = |text: &String| {
            let options = RtOptions::new(width)
                .initial_indent(Line::from(INDENT))
                .subsequent_indent(Line::from("    "));
            word_wrap_lines([Line::from(text.clone())], options)
        };
        let mut lines: Vec<Line<'static>> = vec!["/context".magenta().into(), Line::default()];
        lines.extend(self.heading.iter().flat_map(wrap));
        if !self.rows.is_empty() {
            lines.push(Line::default());
            lines.extend(self.row_lines(width));
        }
        lines.push(Line::default());
        lines.extend(self.cache.iter().flat_map(wrap));
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.display_lines(u16::MAX))
    }
}

/// `tokens` as a whole percent of `scale`, rounded half up; 0 without one.
fn percent_of(tokens: u64, scale: u64) -> u64 {
    if scale == 0 {
        return 0;
    }
    (u128::from(tokens) * 100 + u128::from(scale) / 2)
        .checked_div(u128::from(scale))
        .map_or(0, |percent| u64::try_from(percent).unwrap_or(u64::MAX))
}

/// `text` cut to `width` columns, ending in `…` when cut.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut fitted = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width + 1 > width {
            break;
        }
        fitted.push(character);
        used += character_width;
    }
    fitted.push('…');
    fitted
}

#[cfg(test)]
mod tests;
