// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The drawing as spans of one role each, for the caller's theme; no cell
//! ever holds a control character.

/// The part of a diagram a caller styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Node,
    Edge,
    Text,
}

/// Adjacent characters of one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) text: String,
    pub(crate) role: Role,
}

#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub(super) symbol: char,
    pub(super) role: Role,
}

impl Cell {
    pub(super) fn edge(symbol: char) -> Self {
        Self {
            symbol,
            role: Role::Edge,
        }
    }

    pub(super) fn node(symbol: char) -> Self {
        Self {
            symbol,
            role: Role::Node,
        }
    }
}

/// The canvas's rows as spans, a wide character's second cell (`\0`) left
/// out and each row's trailing blanks trimmed.
pub(super) fn finish(rows: Vec<Vec<Cell>>) -> Vec<Vec<Span>> {
    rows.into_iter()
        .map(|row| {
            let mut spans: Vec<Span> = Vec::new();
            for Cell { symbol, role } in row {
                if symbol == '\0' {
                    continue;
                }
                match spans.last_mut() {
                    Some(last) if last.role == role => last.text.push(symbol),
                    _ => spans.push(Span {
                        text: symbol.to_string(),
                        role,
                    }),
                }
            }
            while let Some(last) = spans.last_mut() {
                last.text.truncate(last.text.trim_end().len());
                if !last.text.is_empty() {
                    break;
                }
                spans.pop();
            }
            spans
        })
        .collect()
}
