//! The configuration center's layers page: the files the settings are read
//! from, lowest first, the warnings they raised, and where writes go.

use hanafubuki_agent::ConfigLayerSource;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Widget, Wrap};

use super::ConfigCenter;
use crate::key_hint;

impl ConfigCenter {
    pub(super) fn open_layers(&mut self) {
        self.layers_open = true;
    }

    pub(super) fn render_layers(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let [title, body, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        Line::from("Layers & files".bold().cyan()).render(title, buf);
        let lines = match &self.state {
            Ok(state) => layer_lines(state),
            Err(error) => vec![Line::from(error.clone().red())],
        };
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(body, buf);
        Line::from(vec![
            key_hint::plain(ratatui::crossterm::event::KeyCode::Esc).into(),
            " back  ".dim(),
            key_hint::plain(ratatui::crossterm::event::KeyCode::Char('q')).into(),
            " close".dim(),
        ])
        .render(hint, buf);
    }
}

fn layer_lines(state: &crate::agent_bridge::ConfigState) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(
            "Writes go to the user's config.toml. providers.toml and mcp.toml are not \
             layered; their editors write the owner's file."
                .dim(),
        ),
        Line::default(),
        Line::from("Layers, lowest first".bold()),
    ];
    lines.extend(state.layers.iter().map(layer_line));
    lines.push(Line::default());
    lines.push(Line::from(
        format!("Warnings ({})", state.warnings.len()).bold(),
    ));
    if state.warnings.is_empty() {
        lines.push(Line::from("None.".dim()));
    } else {
        lines.extend(
            state
                .warnings
                .iter()
                .map(|warning| Line::from(vec!["  • ".dim(), warning.clone().into()])),
        );
    }
    lines
}

fn layer_line(source: &ConfigLayerSource) -> Line<'static> {
    match source {
        ConfigLayerSource::User { path, present } => {
            let mut spans = vec!["  User       ".bold(), path.display().to_string().into()];
            if !present {
                spans.push("  (not created yet)".dim());
            }
            Line::from(spans)
        }
        ConfigLayerSource::Profile { name } => {
            Line::from(vec!["  Profile    ".bold(), name.clone().into()])
        }
        ConfigLayerSource::Project { path, trusted } => Line::from(vec![
            "  Project    ".bold(),
            path.display().to_string().into(),
            if *trusted {
                "  (trusted)".dim()
            } else {
                "  (not trusted; not read)".dim()
            },
        ]),
        ConfigLayerSource::Overrides => {
            Line::from(vec!["  Overrides  ".bold(), "-c values".into()])
        }
    }
}
