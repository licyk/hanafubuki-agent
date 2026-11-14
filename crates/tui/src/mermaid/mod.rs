// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Bounded Mermaid diagrams drawn as text (Codex's `codex-mermaid` crate,
//! a module here): flowcharts, sequence diagrams, flat state diagrams,
//! class diagrams and entity-relationship diagrams, each a strict subset.
//!
//! Every graph edge owns its lane and its endpoints, and crossings never
//! join routes. Anything outside a family's subset, past a limit, or wider
//! than the width given is an error and never a partial drawing: the caller
//! keeps the source. Nothing here performs I/O or emits a control sequence.
//!
//! Limits: 16 KiB of source; 16 nodes, 24 edges (after `&` groups expand)
//! and 16 members per node; 24 references in one `&` group; 8 participants,
//! 64 events and 4 nested fragments in a sequence; 40 display columns per
//! label line and 40 bytes per identifier; 65,536 cells per drawing,
//! whatever the width.

mod draw;
mod output;
mod parse;
mod relations;
mod sequence;
mod state;
mod syntax;

pub(crate) use output::{Role, Span};

const MAX_SOURCE: usize = 16 * 1024;
const MAX_NODES: usize = 16;
const MAX_EDGES: usize = 24;
const MAX_LABEL: usize = 40;
const MAX_MEMBERS: usize = 16;
const MAX_CELLS: usize = 64 * 1024;

/// Why a diagram is not drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderError {
    /// Syntax or text outside the supported subset.
    Unsupported,
    /// A source, diagram, label or canvas limit exceeded.
    Limit,
    /// The whole diagram is wider than the width given.
    TooWide,
}

/// `source` drawn as plain Unicode text, its rows joined by `\n`.
#[cfg(test)]
pub(crate) fn render(source: &str, max_width: usize) -> Result<String, RenderError> {
    Ok(render_spans(source, max_width)?
        .into_iter()
        .map(|line| line.into_iter().map(|span| span.text).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// `source` drawn as rows of spans, each a node, an edge or text, for the
/// caller to style.
pub(crate) fn render_spans(source: &str, max_width: usize) -> Result<Vec<Vec<Span>>, RenderError> {
    if source.len() > MAX_SOURCE {
        return Err(RenderError::Limit);
    }
    let statements = syntax::statements(source)?;
    let (header, body) = statements.split_first().ok_or(RenderError::Unsupported)?;
    match *header {
        "sequenceDiagram" => sequence::render(body, max_width),
        "stateDiagram-v2" | "stateDiagram" | "classDiagram" | "erDiagram" => {
            draw::render(&relations::parse(header, body)?, max_width)
        }
        _ => draw::render(&parse::parse(header, body)?, max_width),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Direction {
    #[default]
    Down,
    Up,
    Right,
    Left,
}

impl Direction {
    fn parse(text: &str) -> Result<Self, RenderError> {
        match text {
            "TD" | "TB" => Ok(Self::Down),
            "BT" => Ok(Self::Up),
            "LR" => Ok(Self::Right),
            "RL" => Ok(Self::Left),
            _ => Err(RenderError::Unsupported),
        }
    }
}

/// The node shapes drawn.
#[derive(Debug, PartialEq, Eq)]
enum Shape {
    /// `A[Work]`.
    Rectangle,
    /// `A{Done?}`, a box with `◇` before its label.
    Decision,
    /// `A([Start])`, a box with rounded corners.
    Stadium,
}

#[derive(Debug, PartialEq, Eq)]
struct Node {
    id: String,
    label: String,
    shape: Shape,
    declared: bool,
    members: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Edge {
    from: usize,
    to: usize,
    label: String,
    target_label: String,
    source_tip: char,
    target_tip: char,
    dashed: bool,
}

impl Edge {
    fn directed(from: usize, to: usize, label: String) -> Self {
        Self {
            from,
            to,
            label,
            target_label: String::new(),
            source_tip: '─',
            target_tip: '◄',
            dashed: false,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Graph {
    direction: Direction,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

impl Graph {
    fn node(&mut self, id: &str) -> Result<usize, RenderError> {
        if let Some(index) = self.nodes.iter().position(|node| node.id == id) {
            return Ok(index);
        }
        if self.nodes.len() == MAX_NODES {
            return Err(RenderError::Limit);
        }
        self.nodes.push(Node {
            id: id.to_owned(),
            label: id.to_owned(),
            shape: Shape::Rectangle,
            declared: false,
            members: Vec::new(),
        });
        Ok(self.nodes.len() - 1)
    }
}

#[cfg(test)]
mod tests;
