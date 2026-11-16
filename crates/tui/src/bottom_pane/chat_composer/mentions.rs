// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Shared mention routing, cursor affinity and atomic completion in the composer.
use super::{
    completion_target::{self, DollarQueryKind},
    dismissed::DismissedToken,
    ActivePopup, ChatComposer, InputResult, MentionPopup, SkillPopup,
};
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::bottom_pane::{
    footer::reset_mode_after_activity,
    mentions::{Candidate, MentionType, Selection},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::ops::Range;

impl ChatComposer {
    pub(crate) fn set_mention_usage(&mut self, recent: &[String]) {
        self.mention_usage.clear();
        for identity in recent {
            *self.mention_usage.entry(identity.clone()).or_default() += 1;
        }
        if let ActivePopup::Skill(popup) = &mut self.active_popup {
            popup.set_frequencies(self.mention_usage.clone());
        }
    }

    pub(crate) fn set_mention_catalog(&mut self, catalog: crate::mentions::Catalog) {
        let tasks = self
            .mention_catalog
            .at
            .iter()
            .filter(|item| item.mention_type == MentionType::Task)
            .cloned()
            .collect::<Vec<_>>();
        self.mention_catalog = catalog;
        self.mention_catalog.at.extend(tasks);
        if let ActivePopup::Mention(popup) = &mut self.active_popup {
            popup.set_candidates(self.mention_catalog.at.clone());
        }
        if let ActivePopup::Skill(popup) = &mut self.active_popup {
            popup.set_mentions(self.mention_catalog.dollar.clone());
        }
        self.sync_popups();
    }

    pub(crate) fn set_mention_tasks(&mut self, query: String, candidates: Vec<Candidate>) {
        if self.editable_token('@').as_ref().map(|(_, q)| q) != Some(&query) {
            return;
        }
        self.mention_catalog
            .at
            .retain(|c| c.mention_type != MentionType::Task);
        self.mention_catalog.at.extend(candidates);
        if let ActivePopup::Mention(popup) = &mut self.active_popup {
            popup.set_candidates(self.mention_catalog.at.clone());
        }
    }

    fn editable_token(&self, prefix: char) -> Option<(Range<usize>, String)> {
        let predicate = |query: &str| match completion_target::dollar_query_kind(query) {
            DollarQueryKind::Completable => true,
            DollarQueryKind::AmbiguousShellParameter => self
                .mention_catalog
                .dollar
                .iter()
                .any(|item| item.path.is_some() && item.fuzzy_match_query(query).is_some()),
            _ => false,
        };
        let (range, query) = completion_target::current_prefixed_token_range_with_dollar_predicate(
            &self.textarea,
            prefix,
            true,
            predicate,
        )?;
        (completion_target::prefixed_token_range_is_editable(
            &self.textarea,
            prefix,
            &range,
            &query,
        ) && (prefix != '$' || predicate(&query)))
        .then_some((range, query))
    }

    pub(super) fn sync_popups(&mut self) {
        if self.history_search_active()
            || self
                .history
                .should_handle_navigation(self.textarea.text(), self.textarea.cursor())
        {
            self.end_file_search();
            self.active_popup = ActivePopup::None;
            return;
        }
        let mut at = self.editable_token('@');
        let mut dollar = self.editable_token('$');
        if let (Some((left, _)), Some((right, _))) = (&at, &dollar) {
            if left.start > right.start {
                dollar = None;
            } else if right.start > left.start {
                at = None;
            }
        }
        self.sync_command_popup(at.is_none() && dollar.is_none());
        if matches!(self.active_popup, ActivePopup::Command(_)) {
            self.end_file_search();
            return;
        }
        let token = at.as_ref().or(dollar.as_ref());
        let Some((range, query)) = token else {
            self.end_file_search();
            self.active_popup = ActivePopup::None;
            self.dismissed_mention_token = None;
            self.mention_catalog_requested = false;
            return;
        };
        if self
            .dismissed_mention_token
            .as_ref()
            .is_some_and(|d| d.matches(&self.textarea, range, query))
        {
            return;
        }
        self.dismissed_mention_token = None;
        if !self.mention_catalog_requested {
            self.mention_catalog_requested = true;
            self.app_event_tx
                .send(AppEvent::Op(Op::Command(AgentCommand::LoadMentions)));
        }
        if at.is_some() {
            if self.current_file_query.as_ref() != Some(query) {
                self.current_file_query = Some(query.clone());
                self.mention_catalog
                    .at
                    .retain(|c| c.mention_type != MentionType::Task);
                self.app_event_tx
                    .send(AppEvent::StartFileSearch(query.clone()));
                self.app_event_tx.send(AppEvent::Op(Op::Command(
                    AgentCommand::SearchMentionTasks(query.clone()),
                )));
            }
            match &mut self.active_popup {
                ActivePopup::Mention(popup) => {
                    popup.set_query(query);
                    popup.set_candidates(self.mention_catalog.at.clone());
                }
                _ => {
                    self.active_popup = ActivePopup::Mention(MentionPopup::new(
                        self.mention_catalog.at.clone(),
                        query,
                    ))
                }
            }
        } else {
            self.end_file_search();
            match &mut self.active_popup {
                ActivePopup::Skill(popup) => popup.set_query(query),
                _ => {
                    let mut popup = SkillPopup::new(self.mention_catalog.dollar.clone())
                        .with_frequencies(self.mention_usage.clone());
                    popup.set_query(query);
                    self.active_popup = ActivePopup::Skill(popup);
                }
            }
        }
    }

    fn end_file_search(&mut self) {
        if self.current_file_query.take().is_some() {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(String::new()));
            self.app_event_tx
                .send(AppEvent::Op(Op::Command(AgentCommand::SearchMentionTasks(
                    String::new(),
                ))));
        }
    }

    pub(super) fn handle_key_event_with_mention_popup(
        &mut self,
        key: KeyEvent,
    ) -> (InputResult, bool) {
        if key.kind == ratatui::crossterm::event::KeyEventKind::Release {
            return (InputResult::None, false);
        }
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
        let unified = matches!(self.active_popup, ActivePopup::Mention(_));
        let prefix = if unified { '@' } else { '$' };
        match key {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => match &mut self.active_popup {
                ActivePopup::Mention(p) => p.move_up(),
                ActivePopup::Skill(p) => p.move_up(),
                _ => {}
            },
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => match &mut self.active_popup {
                ActivePopup::Mention(p) => p.move_down(),
                ActivePopup::Skill(p) => p.move_down(),
                _ => {}
            },
            KeyEvent {
                code: KeyCode::Left | KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            } if unified => {
                if let ActivePopup::Mention(p) = &mut self.active_popup {
                    if key.code == KeyCode::Left {
                        p.previous_search_mode();
                    } else {
                        p.next_search_mode();
                    }
                }
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                if let Some((range, query)) = self.editable_token(prefix) {
                    self.dismissed_mention_token =
                        Some(DismissedToken::new(&self.textarea, range, query));
                }
                self.active_popup = ActivePopup::None;
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                let selected = match &self.active_popup {
                    ActivePopup::Mention(p) => p.selected(),
                    ActivePopup::Skill(p) => p.selected_mention().map(|m| Selection::Tool {
                        insert_text: m.insert_text.clone(),
                        path: m.path.clone(),
                    }),
                    _ => None,
                };
                let submit = unified && selected.is_none() && key.code == KeyCode::Enter;
                if let (Some(selected), Some((range, _))) = (selected, self.editable_token(prefix))
                {
                    match selected {
                        Selection::File {
                            path,
                            absolute_path,
                        } => self.complete_file(range, &path.to_string_lossy(), &absolute_path),
                        Selection::Tool { insert_text, path } => {
                            self.complete_mention(range, &insert_text, path.as_deref());
                            if !unified {
                                if let Some(path) = path {
                                    *self.mention_usage.entry(path.clone()).or_default() += 1;
                                    self.app_event_tx.send(AppEvent::RecordMention(path));
                                }
                            }
                        }
                    }
                }
                self.active_popup = ActivePopup::None;
                if submit {
                    return self.handle_key_event_without_popup(key);
                }
            }
            input => return self.handle_input_basic(input),
        }
        (InputResult::None, true)
    }

    fn complete_file(&mut self, range: Range<usize>, path: &str, file: &std::path::Path) {
        let range = self.separate_completion(range);
        if self.image_attachable().is_ok()
            && matches!(
                crate::clipboard::image_file::inspect(file),
                crate::clipboard::image_file::Inspection::Image
            )
        {
            let start = range.start;
            self.textarea.replace_range(range, "");
            self.textarea.set_cursor(start);
            self.attach_image(crate::clipboard::image_file::file_image(file));
            self.completion_separator();
            return;
        }
        let inserted = if path.chars().any(char::is_whitespace) && !path.contains('"') {
            format!("\"{path}\"")
        } else {
            path.to_owned()
        };
        let start = range.start;
        self.textarea.replace_range(range, &inserted);
        self.textarea.set_cursor(start + inserted.len());
        self.completion_separator();
    }

    fn complete_mention(&mut self, range: Range<usize>, label: &str, path: Option<&str>) {
        let range = self.separate_completion(range);
        let start = range.start;
        self.textarea.replace_range(range, "");
        self.textarea.set_cursor(start);
        self.textarea.insert_str(label);
        let range = start..start + label.len();
        if let Some(path) = path {
            self.textarea.mark_link(range, path.to_owned());
        } else {
            self.textarea.mark_element(range);
        }
        self.completion_separator();
    }

    fn separate_completion(&mut self, mut range: Range<usize>) -> Range<usize> {
        if self
            .textarea
            .text_element_ranges()
            .any(|e| e.end == range.start)
        {
            self.textarea.insert_str_at(range.start, " ");
            range.start += 1;
            range.end += 1;
        }
        range
    }

    fn completion_separator(&mut self) {
        let cursor = self.textarea.cursor();
        let next = self.textarea.text()[cursor..].chars().next();
        let separator = next.filter(|c| {
            c.is_whitespace()
                && !matches!(
                    *c,
                    '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
                )
        });
        if let Some(separator) = separator {
            let after = cursor + separator.len_utf8();
            if self.textarea.text()[after..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
            {
                self.textarea.insert_str_at(cursor, " ");
            } else {
                self.textarea.set_cursor(after);
            }
        } else {
            self.textarea.insert_str_at(cursor, " ");
        }
    }
}

#[cfg(test)]
mod tests;
