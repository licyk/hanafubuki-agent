// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The plugin browser's tab line, drawn as the settings center's tabs are:
//! the tab shown in reverse, the others dim, each over the cells it was
//! drawn on so a click can choose it.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::pages::Tab;
use crate::agent_bridge::PluginsState;
use crate::render::renderable::Renderable;

/// Where the tab line drew each tab, for the pointer.
pub(super) type TabHits = Rc<RefCell<Vec<(Rect, Tab)>>>;

pub(super) struct Tabs {
    labels: Vec<(Tab, String)>,
    active: Tab,
    hits: TabHits,
}

impl Tabs {
    pub(super) fn new(state: &PluginsState, active: Tab, hits: &TabHits) -> Self {
        let labels = Tab::ALL
            .into_iter()
            .map(|tab| {
                let count = match tab {
                    Tab::Installed => state.packages.len(),
                    Tab::Marketplaces => state.marketplaces.len(),
                    Tab::BuiltIn => state.builtins.len(),
                };
                (tab, format!(" {} ({count}) ", tab.label()))
            })
            .collect();
        Self {
            labels,
            active,
            hits: hits.clone(),
        }
    }
}

impl Renderable for Tabs {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut hits = self.hits.borrow_mut();
        hits.clear();
        let width = |from: usize, to: usize| {
            self.labels[from..=to]
                .iter()
                .map(|(_, label)| label.width() + 1)
                .sum::<usize>()
        };
        let active = self
            .labels
            .iter()
            .position(|(tab, _)| *tab == self.active)
            .unwrap_or(0);
        // From the first tab that leaves the one shown its room.
        let first = (0..=active)
            .find(|first| width(*first, active) <= usize::from(area.width))
            .unwrap_or(active);
        let mut x = area.x;
        for (tab, label) in self.labels.iter().skip(first) {
            let width = label.width() as u16;
            if x.saturating_add(width) > area.right() {
                break;
            }
            let cell = Rect {
                x,
                width,
                height: 1,
                ..area
            };
            let style = if *tab == self.active {
                Style::new().bold().reversed()
            } else {
                Style::new().dim()
            };
            Line::from(Span::styled(label.as_str(), style)).render(cell, buf);
            hits.push((cell, *tab));
            x += width + 1;
        }
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}
