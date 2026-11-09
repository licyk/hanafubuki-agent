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
    let visible_range = start_idx..(start_idx + visible_items);
    let max_name_width = rows_all
        .iter()
        .enumerate()
        .filter(|(i, _)| visible_range.contains(i))
        .map(|(_, r)| {
            let mut spans: Vec<Span> = vec![r.name.clone().into()];
            if r.disabled_reason.is_some() {
                spans.push(" (disabled)".dim());
            }
            Line::from(spans).width()
        })
        .max()
        .unwrap_or(0);
    let desc_col = max_name_width.saturating_add(2);
    (desc_col.saturating_add(MIN_DESCRIPTION_WIDTH) <= content_width as usize).then_some(desc_col)
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

    let desc_col = compute_desc_col(rows_all, start_idx, visible_items, area.width);

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
        let wrapped = row_lines(row, desc_col, area.width, |line| {
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

    let desc_col = compute_desc_col(rows_all, start_idx, visible_items, content_width);

    let mut total: u16 = 0;
    for row in rows_all
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(visible_items)
        .map(|(_, r)| r)
    {
        let wrapped_lines = row_lines(row, desc_col, content_width, |_| {}).len();
        total = total.saturating_add(wrapped_lines as u16);
    }
    total.max(1)
}
