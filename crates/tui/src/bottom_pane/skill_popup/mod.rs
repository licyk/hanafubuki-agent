// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use std::cmp::Reverse;
use std::collections::HashMap;

use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use crate::fuzzy_match::fuzzy_match;
use crate::key_hint;

#[derive(Clone, Debug)]
pub(crate) struct MentionItem {
    pub(crate) display_name: String,
    pub(crate) description: Option<String>,
    pub(crate) insert_text: String,
    pub(crate) search_terms: Vec<String>,
    pub(crate) path: Option<String>,
    pub(crate) category_tag: Option<String>,
    pub(crate) sort_rank: u8,
}

/// The best fuzzy match for a mention and the display-name positions that can be highlighted.
///
/// `display_name_indices` is absent when only an alternate search term matched.
pub(crate) struct MentionMatch {
    display_name_indices: Option<Vec<usize>>,
    score: i32,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum MatchTier {
    Exact,
    Prefix,
    Fuzzy,
}

impl MentionItem {
    /// Matches the display name first, then falls back to the best distinct search term.
    pub(crate) fn fuzzy_match_query(&self, query: &str) -> Option<MentionMatch> {
        if let Some((indices, score)) = fuzzy_match(&self.display_name, query) {
            Some(MentionMatch {
                display_name_indices: Some(indices),
                score,
            })
        } else {
            self.search_terms
                .iter()
                .filter(|term| *term != &self.display_name)
                .filter_map(|term| fuzzy_match(term, query).map(|(_indices, score)| score))
                .min()
                .map(|score| MentionMatch {
                    display_name_indices: None,
                    score,
                })
        }
    }

    fn match_tier(&self, lowercase_query: &str) -> MatchTier {
        let display_name = self.display_name.to_lowercase();
        let insert_name = self
            .insert_text
            .strip_prefix('$')
            .unwrap_or(&self.insert_text)
            .to_lowercase();
        if display_name == lowercase_query || insert_name == lowercase_query {
            MatchTier::Exact
        } else if display_name.starts_with(lowercase_query)
            || insert_name.starts_with(lowercase_query)
        {
            MatchTier::Prefix
        } else {
            MatchTier::Fuzzy
        }
    }
}

mod render;

pub(crate) struct SkillPopup {
    query: String,
    mentions: Vec<MentionItem>,
    frequencies: HashMap<String, usize>,
    state: ScrollState,
}

impl SkillPopup {
    pub(crate) fn new(mentions: Vec<MentionItem>) -> Self {
        Self {
            query: String::new(),
            mentions,
            frequencies: HashMap::new(),
            state: ScrollState::new(),
        }
    }

    pub(crate) fn with_frequencies(mut self, frequencies: HashMap<String, usize>) -> Self {
        self.frequencies = frequencies;
        self
    }

    pub(crate) fn set_frequencies(&mut self, frequencies: HashMap<String, usize>) {
        self.frequencies = frequencies;
        self.clamp_selection();
    }

    pub(crate) fn set_mentions(&mut self, mentions: Vec<MentionItem>) {
        self.mentions = mentions;
        self.clamp_selection();
    }

    pub(crate) fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.clamp_selection();
    }

    pub(crate) fn calculate_required_height(&self, _width: u16) -> u16 {
        let visible = self.filtered().len().clamp(1, MAX_POPUP_ROWS);
        (visible as u16).saturating_add(/*rhs*/ 3)
    }

    pub(crate) fn move_up(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    pub(crate) fn move_down(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_down_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    pub(crate) fn selected_mention(&self) -> Option<&MentionItem> {
        let matches = self.filtered_items();
        let idx = self.state.selected_idx?;
        let mention_idx = matches.get(idx)?;
        self.mentions.get(*mention_idx)
    }

    fn clamp_selection(&mut self) {
        let len = self.filtered_items().len();
        self.state.clamp_selection(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn filtered_items(&self) -> Vec<usize> {
        self.filtered().into_iter().map(|(idx, _, _)| idx).collect()
    }

    fn filtered(&self) -> Vec<(usize, Option<Vec<usize>>, i32)> {
        let filter = self.query.trim();
        let lowercase_filter = filter.to_lowercase();
        let mut out: Vec<(usize, Option<Vec<usize>>, i32)> = Vec::new();

        for (idx, mention) in self.mentions.iter().enumerate() {
            if filter.is_empty() {
                out.push((idx, None, 0));
                continue;
            }

            if let Some(mention_match) = mention.fuzzy_match_query(filter) {
                out.push((idx, mention_match.display_name_indices, mention_match.score));
            }
        }

        out.sort_by_cached_key(|(idx, indices, score)| {
            let mention = &self.mentions[*idx];
            let tier = if filter.is_empty() {
                MatchTier::Fuzzy
            } else {
                mention.match_tier(&lowercase_filter)
            };
            let frequency = mention
                .path
                .as_ref()
                .and_then(|path| self.frequencies.get(path))
                .copied()
                .unwrap_or_default();
            (
                tier,
                Reverse(frequency),
                indices.is_none(),
                *score,
                mention.sort_rank,
                &mention.display_name,
            )
        });

        out
    }
}

impl Widget for &SkillPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (list_area, hint_area) = if area.height > 1 {
            let [list_area, hint_area] =
                Layout::vertical([Constraint::Length(area.height - 1), Constraint::Length(1)])
                    .areas(area);
            (list_area, Some(hint_area))
        } else {
            (area, None)
        };
        self.render_rows(list_area, buf);
        if let Some(hint_area) = hint_area {
            let hint_area = Rect {
                x: hint_area.x + 2,
                y: hint_area.y,
                width: hint_area.width.saturating_sub(2),
                height: hint_area.height,
            };
            skill_popup_hint_line().render(hint_area, buf);
        }
    }
}

fn skill_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        ratatui::text::Span::raw(
            ratatui::text::Span::from(key_hint::plain(KeyCode::Enter)).content,
        )
        .bold(),
        " insert · ".dim(),
        ratatui::text::Span::raw(ratatui::text::Span::from(key_hint::plain(KeyCode::Esc)).content)
            .bold(),
        " close".dim(),
    ])
}

#[cfg(test)]
mod tests;
