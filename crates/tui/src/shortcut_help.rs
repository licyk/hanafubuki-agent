// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Shortcut groups (Codex's `shortcut_help.rs`): each a title over its keys,
//! aligned, the groups flowing into three, two or one column by width. The
//! surface that shows them gives the keys in force and adds its title,
//! footer and height limit.

use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use crate::key_hint::KeyBinding;
use crate::style::accent_color;

const COLUMN_GAP: usize = 4;

pub(crate) struct Shortcut {
    pub(crate) key: String,
    pub(crate) action: &'static str,
}

impl Shortcut {
    pub(crate) fn new(key: KeyBinding, action: &'static str) -> Self {
        Self {
            key: label(key),
            action,
        }
    }
}

pub(crate) struct Group {
    pub(crate) title: &'static str,
    pub(crate) entries: Vec<Shortcut>,
}

impl Group {
    /// Adds `action` under `key`; nothing for an action with no key.
    pub(crate) fn push(&mut self, key: Option<KeyBinding>, action: &'static str) {
        if let Some(key) = key {
            self.entries.push(Shortcut::new(key, action));
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let key_width = self
            .entries
            .iter()
            .map(|entry| Span::raw(&entry.key).width())
            .max()
            .unwrap_or(0);
        let mut lines = vec![self.title.bold().into()];
        for entry in &self.entries {
            let key = entry.key.clone().fg(accent_color());
            let padding = " ".repeat(key_width.saturating_sub(key.width()) + 2);
            lines.push(vec![key, padding.into(), entry.action.into()].into());
        }
        lines
    }
}

/// A key as a hint writes it (`ctrl + j`).
pub(crate) fn label(key: KeyBinding) -> String {
    Span::from(key).content.into_owned()
}

/// The groups side by side when the three fit in `width`, else the first
/// beside the other two stacked, else all stacked.
pub(crate) fn group_lines(groups: [Group; 3], width: u16) -> Vec<Line<'static>> {
    let groups = groups.map(|group| group.lines());
    let widths: Vec<usize> = groups
        .iter()
        .map(|group| group.iter().map(Line::width).max().unwrap_or(0))
        .collect();
    let width = usize::from(width.max(1));
    let mut result = Vec::new();
    if widths.iter().sum::<usize>() + COLUMN_GAP * 2 <= width {
        result.extend(columns(&groups, &widths));
    } else if widths[0] + COLUMN_GAP + widths[1].max(widths[2]) <= width {
        let mut right = groups[1].clone();
        right.push(Line::default());
        right.extend(groups[2].clone());
        result.extend(columns(
            &[groups[0].clone(), right],
            &[widths[0], widths[1].max(widths[2])],
        ));
    } else {
        for (index, group) in groups.into_iter().enumerate() {
            if index > 0 {
                result.push(Line::default());
            }
            result.extend(group);
        }
    }
    result
}

fn columns(groups: &[Vec<Line<'static>>], widths: &[usize]) -> Vec<Line<'static>> {
    let height = groups.iter().map(Vec::len).max().unwrap_or(0);
    (0..height)
        .map(|row| {
            let mut line = Line::default();
            for (column, group) in groups.iter().enumerate() {
                let entry = group.get(row).cloned().unwrap_or_default();
                let padding = widths[column].saturating_sub(entry.width()) + COLUMN_GAP;
                line.extend(entry.spans);
                if column + 1 < groups.len() {
                    line.push_span(" ".repeat(padding));
                }
            }
            line
        })
        .collect()
}
