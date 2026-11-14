// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
// Note: Table-based layout previously used Constraint; the manual renderer
// below no longer requires it.
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::key_hint::KeyBinding;
use crate::render::Insets;
use crate::render::RectExt as _;
use crate::style::user_message_style;

use super::scroll_state::ScrollState;

/// A generic representation of a display row for selection popups.
#[derive(Default)]
pub(crate) struct GenericDisplayRow {
    pub name: String,
    pub display_shortcut: Option<KeyBinding>,
    pub match_indices: Option<Vec<usize>>, // indices to bold (char positions)
    pub description: Option<String>,       // optional grey text after the name
    pub disabled_reason: Option<String>,   // optional disabled message
    pub is_disabled: bool,
    pub wrap_indent: Option<usize>, // optional indent for wrapped lines
    pub facts: Option<RowFacts>,
}

/// What a row shows of its item beside the name, as a list of models does:
/// `columns`, fixed-width text drawn at the row's right edge, and the same
/// labelled on one line (`stacked`), drawn under the name where the screen
/// is too narrow for columns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RowFacts {
    pub columns: String,
    pub stacked: String,
}

const MENU_SURFACE_INSET_V: u16 = 1;
const MENU_SURFACE_INSET_H: u16 = 2;

/// Apply the shared "menu surface" padding used by bottom-pane overlays.
///
/// Rendering code should generally call [`render_menu_surface`] and then lay
/// out content inside the returned inset rect.
pub(crate) fn menu_surface_inset(area: Rect) -> Rect {
    area.inset(Insets::vh(MENU_SURFACE_INSET_V, MENU_SURFACE_INSET_H))
}

/// Total vertical padding introduced by the menu surface treatment.
pub(crate) const fn menu_surface_padding_height() -> u16 {
    MENU_SURFACE_INSET_V * 2
}

/// Paint the shared menu background and return the inset content area.
///
/// This keeps the surface treatment consistent across selection-style overlays
/// (for example `/model`, approvals, and request-user-input).
pub(crate) fn render_menu_surface(area: Rect, buf: &mut Buffer) -> Rect {
    if area.is_empty() {
        return area;
    }
    Block::default()
        .style(user_message_style())
        .render(area, buf);
    menu_surface_inset(area)
}

pub(crate) fn wrap_styled_line<'a>(line: &'a Line<'a>, width: u16) -> Vec<Line<'a>> {
    use crate::wrapping::word_wrap_line;
    use crate::wrapping::RtOptions;

    let width = width.max(1) as usize;
    let opts = RtOptions::new(width)
        .initial_indent(Line::from(""))
        .subsequent_indent(Line::from(""));
    word_wrap_line(line, opts)
}

/// The narrowest description column beside the names. Where the widest
/// visible name leaves less, every description goes under its name: a
/// column a character or two wide would stack its text one character a line.
const MIN_DESCRIPTION_WIDTH: usize = 10;

/// How far a description under its name, and the name's own wrapped lines,
/// are indented.
const STACKED_INDENT: usize = 4;

/// Compute a shared description-column start based on the widest visible name
/// plus two spaces of padding; `None` when that leaves the description fewer
/// than [`MIN_DESCRIPTION_WIDTH`] columns, so each description goes under its
/// name.
fn compute_desc_col(
    rows_all: &[GenericDisplayRow],
    start_idx: usize,
    visible_items: usize,
    content_width: u16,
) -> Option<usize> {
    let max_name_width = rows_all
        .iter()
        .skip(start_idx)
        .take(visible_items)
        .map(name_width)
        .max()
        .unwrap_or(0);
    let desc_col = max_name_width.saturating_add(2);
    (desc_col.saturating_add(MIN_DESCRIPTION_WIDTH) <= content_width as usize).then_some(desc_col)
}

fn name_width(row: &GenericDisplayRow) -> usize {
    let disabled = if row.disabled_reason.is_some() {
        " (disabled)".width()
    } else {
        0
    };
    row.name.width() + disabled
}

/// The narrowest a name (with its prefix) is cut to beside fact columns;
/// narrower, each row's facts go under its name.
const MIN_TABULAR_NAME_WIDTH: usize = 22;

/// How the visible rows are laid out.
enum Columns {
    /// No visible row has facts: the shared description column, or none.
    Plain(Option<usize>),
    /// The facts in columns at the right edge; names cut to `name` columns,
    /// and the descriptions of rows with facts to `description`. Rows
    /// without facts share the description column.
    Tabular { name: usize, description: usize },
    /// Each row's facts labelled under its name; a description under its
    /// name too, but a row with facts' beside it when both fit.
    Stacked,
}

/// The layout of the visible rows at `width`. Whether the facts are in
/// columns depends on the width alone, so scrolling never switches it. The
/// description beside them shrinks to [`MIN_DESCRIPTION_WIDTH`] before a
/// name is cut, a name is cut down to [`MIN_TABULAR_NAME_WIDTH`], and only
/// then the description goes.
fn columns(
    rows_all: &[GenericDisplayRow],
    start_idx: usize,
    visible_items: usize,
    width: u16,
) -> Columns {
    let visible = || rows_all.iter().skip(start_idx).take(visible_items);
    let Some(facts) = visible()
        .filter_map(|row| row.facts.as_ref())
        .map(|facts| facts.columns.width())
        .max()
    else {
        return Columns::Plain(compute_desc_col(rows_all, start_idx, visible_items, width));
    };
    let left = (width as usize).saturating_sub(facts + 2);
    if left < MIN_TABULAR_NAME_WIDTH {
        return Columns::Stacked;
    }
    let max_name = visible().map(name_width).max().unwrap_or(0);
    let max_description = visible()
        .filter(|row| row.facts.is_some())
        .filter_map(|row| row.description.as_deref())
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0);
    let name = max_name
        .min(left.saturating_sub(max_description.min(MIN_DESCRIPTION_WIDTH) + 2))
        .max(max_name.min(MIN_TABULAR_NAME_WIDTH));
    Columns::Tabular {
        name,
        description: left.saturating_sub(name + 2).min(max_description),
    }
}

/// Whether the visible rows have their facts in columns, which a header
/// can name.
pub(crate) fn facts_in_columns(
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    width: u16,
) -> bool {
    let visible_items = max_results.min(rows_all.len());
    let start_idx = start_index(rows_all, state, visible_items);
    matches!(
        columns(rows_all, start_idx, visible_items, width),
        Columns::Tabular { .. }
    )
}

/// The first visible row: the scroll position, moved so the selected row
/// is among the `visible_items`.
fn start_index(rows_all: &[GenericDisplayRow], state: &ScrollState, visible_items: usize) -> usize {
    let mut start_idx = state.scroll_top.min(rows_all.len().saturating_sub(1));
    if let Some(sel) = state.selected_idx {
        if sel < start_idx {
            start_idx = sel;
        } else if visible_items > 0 {
            let bottom = start_idx + visible_items - 1;
            if sel > bottom {
                start_idx = sel + 1 - visible_items;
            }
        }
    }
    start_idx
}

/// A row's lines in `columns`' layout, each styled by `style`.
fn laid_out_lines(
    row: &GenericDisplayRow,
    columns: &Columns,
    width: u16,
    style: impl Fn(&mut Line<'static>),
) -> Vec<Line<'static>> {
    let mut lines = match (&row.facts, columns) {
        (_, Columns::Plain(desc_col)) => return row_lines(row, *desc_col, width, style),
        (None, Columns::Tabular { name, .. }) => {
            return row_lines(row, Some(name + 2), width, style)
        }
        (None, Columns::Stacked) => return row_lines(row, None, width, style),
        (Some(facts), Columns::Tabular { name, description }) => {
            vec![tabular_line(row, facts, *name, *description, width)]
        }
        (Some(facts), Columns::Stacked) => stacked_lines(row, facts, width),
    };
    lines.iter_mut().for_each(style);
    lines
}

/// A row with facts beside the fact columns: its name and description cut
/// to theirs, the facts at the right edge.
fn tabular_line(
    row: &GenericDisplayRow,
    facts: &RowFacts,
    name: usize,
    description: usize,
    width: u16,
) -> Line<'static> {
    let name_text = cut(&row.name, name);
    let mut used = name_text.width();
    let mut spans: Vec<Span<'static>> = vec![name_text.into()];
    if let Some(text) = row.description.as_deref().filter(|_| description > 0) {
        let text = cut(text, description);
        spans.push(" ".repeat(name + 2 - used).into());
        used = name + 2 + text.width();
        spans.push(text.dim());
    }
    let facts_at = (width as usize).saturating_sub(facts.columns.width());
    spans.push(" ".repeat(facts_at.saturating_sub(used)).into());
    spans.push(facts.columns.clone().into());
    Line::from(spans)
}

/// A row with facts on a narrow screen: the name, with the description
/// when both fit, then the facts labelled under it, each cut to the width.
fn stacked_lines(row: &GenericDisplayRow, facts: &RowFacts, width: u16) -> Vec<Line<'static>> {
    let width = width as usize;
    let mut first: Vec<Span<'static>> = vec![cut(&row.name, width).into()];
    if let Some(text) = row
        .description
        .as_deref()
        .filter(|text| row.name.width() + 2 + text.width() <= width)
    {
        first.push("  ".into());
        first.push(text.to_string().dim());
    }
    let indent = " ".repeat(STACKED_INDENT);
    let stacked = cut(&facts.stacked, width.saturating_sub(STACKED_INDENT));
    vec![Line::from(first), Line::from(format!("{indent}{stacked}"))]
}

/// `text` cut to `width` columns, its end replaced by `…` when it is cut.
fn cut(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut kept = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width + 1 > width {
            break;
        }
        used += ch_width;
        kept.push(ch);
    }
    if width > 0 {
        kept.push('…');
    }
    kept
}

/// Determine how many spaces to indent wrapped lines for a row.
fn wrap_indent(row: &GenericDisplayRow, desc_col: Option<usize>, max_width: u16) -> usize {
    let max_indent = max_width.saturating_sub(1) as usize;
    let indent = row.wrap_indent.unwrap_or_else(|| {
        if row.description.is_some() || row.disabled_reason.is_some() {
            desc_col.unwrap_or(STACKED_INDENT)
        } else {
            0
        }
    });
    indent.min(max_indent)
}

fn combined_description(row: &GenericDisplayRow) -> Option<String> {
    match (&row.description, &row.disabled_reason) {
        (Some(desc), Some(reason)) => Some(format!("{desc} (disabled: {reason})")),
        (Some(desc), None) => Some(desc.clone()),
        (None, Some(reason)) => Some(format!("disabled: {reason}")),
        (None, None) => None,
    }
}

/// A row's lines at `width`, each styled by `style` before it is wrapped:
/// the name with the description in the shared column, or, with no column
/// (`desc_col` is `None`), the name and then the description under it.
fn row_lines(
    row: &GenericDisplayRow,
    desc_col: Option<usize>,
    width: u16,
    style: impl Fn(&mut Line<'static>),
) -> Vec<Line<'static>> {
    use crate::wrapping::word_wrap_lines;
    use crate::wrapping::RtOptions;

    let indent = " ".repeat(wrap_indent(row, desc_col, width));
    let mut name = build_full_line(row, desc_col);
    style(&mut name);
    let options = RtOptions::new(width as usize)
        .initial_indent(Line::from(""))
        .subsequent_indent(Line::from(indent.clone()));
    let mut lines = word_wrap_lines([name], options);
    if let (None, Some(description)) = (desc_col, combined_description(row)) {
        let mut description = Line::from(description.dim());
        style(&mut description);
        let options = RtOptions::new(width as usize)
            .initial_indent(Line::from(indent.clone()))
            .subsequent_indent(Line::from(indent));
        lines.extend(word_wrap_lines([description], options));
    }
    lines
}

/// Build the display line for a row: with a description column, the
/// description padded to start at `desc_col`; without one, the name alone.
/// Applies fuzzy-match bolding when indices are present and dims the
/// description.
fn build_full_line(row: &GenericDisplayRow, desc_col: Option<usize>) -> Line<'static> {
    let combined_description = desc_col.and_then(|_| combined_description(row));

    // Enforce single-line name beside a description: allow at most
    // desc_col - 2 cells for name, reserving two spaces before the column.
    let name_limit = match (desc_col, &combined_description) {
        (Some(desc_col), Some(_)) => desc_col.saturating_sub(2),
        _ => usize::MAX,
    };

    let mut name_spans: Vec<Span> = Vec::with_capacity(row.name.len());
    let mut used_width = 0usize;
    let mut truncated = false;

    if let Some(idxs) = row.match_indices.as_ref() {
        let mut idx_iter = idxs.iter().peekable();
        for (char_idx, ch) in row.name.chars().enumerate() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
            let next_width = used_width.saturating_add(ch_w);
            if next_width > name_limit {
                truncated = true;
                break;
            }
            used_width = next_width;

            if idx_iter.peek().is_some_and(|next| **next == char_idx) {
                idx_iter.next();
                name_spans.push(ch.to_string().bold());
            } else {
                name_spans.push(ch.to_string().into());
            }
        }
    } else {
        for ch in row.name.chars() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
            let next_width = used_width.saturating_add(ch_w);
            if next_width > name_limit {
                truncated = true;
                break;
            }
            used_width = next_width;
            name_spans.push(ch.to_string().into());
        }
    }

    if truncated {
        // If there is at least one cell available, add an ellipsis.
        // When name_limit is 0, we still show an ellipsis to indicate truncation.
        name_spans.push("…".into());
    }

    if row.disabled_reason.is_some() {
        name_spans.push(" (disabled)".dim());
    }

    let this_name_width = Line::from(name_spans.clone()).width();
    let mut full_spans: Vec<Span> = name_spans;
    if let Some(display_shortcut) = row.display_shortcut {
        full_spans.push(" (".into());
        full_spans.push(display_shortcut.into());
        full_spans.push(")".into());
    }
    if let (Some(desc_col), Some(desc)) = (desc_col, combined_description.as_ref()) {
        let gap = desc_col.saturating_sub(this_name_width);
        if gap > 0 {
            full_spans.push(" ".repeat(gap).into());
        }
        full_spans.push(desc.clone().dim());
    }
    Line::from(full_spans)
}

/// Render a list of rows using the provided ScrollState, with shared styling
/// and behavior for selection popups.
pub(crate) fn render_rows(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
) {
    if rows_all.is_empty() {
        if area.height > 0 {
            Line::from(empty_message.dim().italic()).render(area, buf);
        }
        return;
    }

    // Determine which logical rows (items) are visible given the selection and
    // the max_results clamp. Scrolling is still item-based for simplicity.
    let visible_items = max_results
        .min(rows_all.len())
        .min(area.height.max(1) as usize);

    let start_idx = start_index(rows_all, state, visible_items);
    let columns = columns(rows_all, start_idx, visible_items, area.width);

    // Render items, wrapping descriptions and aligning wrapped lines under the
    // shared description column. Stop when we run out of vertical space.
    let mut cur_y = area.y;
    for (i, row) in rows_all
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(visible_items)
    {
        if cur_y >= area.y + area.height {
            break;
        }

        let selected = Some(i) == state.selected_idx;
        // Wrapped with subsequent indent aligned to the description column.
        let wrapped = laid_out_lines(row, &columns, area.width, |line| {
            if selected && !row.is_disabled {
                // Match previous behavior: cyan + bold for the selected row.
                // Reset the style first to avoid inheriting dim from keyboard shortcuts.
                line.spans.iter_mut().for_each(|span| {
                    span.style = Style::default().fg(Color::Cyan).bold();
                });
            }
            if row.is_disabled {
                line.spans.iter_mut().for_each(|span| {
                    span.style = span.style.dim();
                });
            }
        });

        // Render the wrapped lines.
        for line in wrapped {
            if cur_y >= area.y + area.height {
                break;
            }
            line.render(
                Rect {
                    x: area.x,
                    y: cur_y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
            cur_y = cur_y.saturating_add(1);
        }
    }
}

/// Compute the number of terminal rows required to render up to `max_results`
/// items from `rows_all` given the current scroll/selection state and the
/// available `width`. Accounts for description wrapping and alignment so the
/// caller can allocate sufficient vertical space.
pub(crate) fn measure_rows_height(
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    width: u16,
) -> u16 {
    if rows_all.is_empty() {
        return 1; // placeholder "no matches" line
    }

    let content_width = width.saturating_sub(1).max(1);

    let visible_items = max_results.min(rows_all.len());
    let start_idx = start_index(rows_all, state, visible_items);
    let columns = columns(rows_all, start_idx, visible_items, content_width);

    let mut total: u16 = 0;
    for row in rows_all
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(visible_items)
        .map(|(_, r)| r)
    {
        let wrapped_lines = laid_out_lines(row, &columns, content_width, |_| {}).len();
        total = total.saturating_add(wrapped_lines as u16);
    }
    total.max(1)
}
