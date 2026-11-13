// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Rows that are prose the parser took into a table: pulldown-cmark takes a
//! line without pipes after a table as one more row, so a paragraph after a
//! table becomes a row of one cell.

use super::{TableBodyRow, TableCell};

/// Whether `row` is prose after the table: only its first cell has text,
/// and its line has no pipe, or it looks like HTML, or it is a label (`…:`)
/// that HTML follows or that ends the table naming HTML.
pub(super) fn is_spillover_row(row: &TableBodyRow, next_row: Option<&TableBodyRow>) -> bool {
    let Some(first_text) = first_non_empty_only_text(&row.cells) else {
        return false;
    };
    if row.cells.len() == 1 && !row.has_table_pipe_syntax {
        return true;
    }
    if looks_like_html_content(&first_text) {
        return true;
    }
    if first_text.trim_end().ends_with(':') {
        if next_row
            .and_then(|row| first_non_empty_only_text(&row.cells))
            .is_some_and(|text| looks_like_html_content(&text))
        {
            return true;
        }
        // The parser can end the table before the HTML block the label
        // introduces.
        if next_row.is_none() && looks_like_html_label_line(&first_text) {
            return true;
        }
    }
    false
}

/// The first cell's text when it is the only cell with any.
fn first_non_empty_only_text(row: &[TableCell]) -> Option<String> {
    let first = row.first()?.plain_text();
    if first.trim().is_empty() {
        return None;
    }
    row[1..]
        .iter()
        .all(|cell| cell.plain_text().trim().is_empty())
        .then_some(first)
}

/// Whether `text` holds a tag: `<`, an optional `/` or `!`, a letter, and
/// a `>` later.
fn looks_like_html_content(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(index, &byte)| {
        if byte != b'<' {
            return false;
        }
        let mut tag_start = index + 1;
        if matches!(bytes.get(tag_start), Some(b'/' | b'!')) {
            tag_start += 1;
        }
        bytes.get(tag_start).is_some_and(u8::is_ascii_alphabetic)
            && bytes
                .get(tag_start + 1..)
                .is_some_and(|suffix| suffix.contains(&b'>'))
    })
}

/// A label ending in `:` that names HTML (`HTML block:`).
fn looks_like_html_label_line(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.ends_with(':')
        && trimmed
            .trim_end_matches(':')
            .split_whitespace()
            .any(|word| word.eq_ignore_ascii_case("html"))
}
