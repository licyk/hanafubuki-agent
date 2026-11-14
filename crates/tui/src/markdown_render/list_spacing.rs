// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! List spacing is a layout policy: what the terminal's scrollback keeps
//! (Scrollback's rows, a streamed answer) separates the next sibling of an
//! item that took more than one row; a view that lays its entries out
//! again (Fullscreen, the transcript overlay) separates every sibling of a
//! list once any of its items takes more than one row, and streams its
//! lists without separators until the answer is consolidated.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ListSpacing {
    /// A blank line before the next sibling of an item of several rows.
    #[default]
    AfterMultiline,
    /// No blank line between siblings: a stream in a view that lays the
    /// finished answer out again.
    Compact,
    /// A blank line between every two siblings when any item of the list
    /// takes more than one row.
    Uniform,
}

/// An open list under [`ListSpacing::Uniform`]: each sibling gets a
/// provisional separator, taken out again at the list's end when no item
/// took more than one row.
#[derive(Default)]
pub(super) struct UniformList {
    pub(super) has_item: bool,
    pub(super) multiline: bool,
    pub(super) separators: Vec<usize>,
}

impl UniformList {
    pub(super) fn finish<T>(self, lines: &mut Vec<T>) {
        if self.multiline {
            return;
        }
        let mut separators = self.separators.into_iter().peekable();
        let Some(&start) = separators.peek() else {
            return;
        };
        let tail = lines.split_off(start);
        lines.extend(tail.into_iter().enumerate().filter_map(|(offset, line)| {
            if separators.peek() == Some(&(start + offset)) {
                separators.next();
                None
            } else {
                Some(line)
            }
        }));
    }
}

#[cfg(test)]
mod tests;
