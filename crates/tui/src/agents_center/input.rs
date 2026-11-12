// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The center's keys (Codex's `agents_overview_view.rs` `handle_key_event`
//! and `agent_center/input.rs`): the session actions are the keymap's
//! `agents` context (`n`, `w`, `f`, `r`, `x`, `a`, Backspace, `g`, `/` and
//! F3 by default); Tab and Shift-Tab, or `1`–`6`, change the filter; the
//! arrows move, wrapping at the ends, and Enter or Right opens; `?` shows
//! the keys. A search or a rename takes the typed text, a confirmation its
//! choice, until Enter or Esc.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::grouping;
use super::status::Filter;
use super::{runs, AgentsCenter, CenterAction, Confirm, Mode};
use crate::keymap::{self, Action};

impl AgentsCenter {
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<CenterAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.notice = None;
        match self.mode {
            Mode::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?' | 'q')) || ctrl_c(&key) {
                    self.mode = Mode::Browse;
                }
                None
            }
            Mode::Confirm(_) => self.confirm_key(key),
            Mode::Rename { .. } => self.rename_key(key),
            Mode::Search(_) => self.search_key(key),
            Mode::Browse => self.browse_key(key),
        }
    }

    /// Text pasted into a search or a rename; `false` when neither is open.
    pub(crate) fn paste(&mut self, text: &str) -> bool {
        let line = text.lines().next().unwrap_or_default();
        match &mut self.mode {
            Mode::Search(query) => {
                query.push_str(line);
                self.searched();
            }
            Mode::Rename { title, .. } => title.push_str(line),
            _ => return false,
        }
        true
    }

    fn browse_key(&mut self, key: KeyEvent) -> Option<CenterAction> {
        if let Some(action) = self.session_key(&key) {
            return action;
        }
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Some(CenterAction::Close),
            _ if ctrl_c(&key) => return Some(CenterAction::Close),
            KeyCode::Char('?') if plain => self.mode = Mode::Help,
            KeyCode::BackTab => self.set_filter(self.filter.step(false)),
            KeyCode::Tab if plain => {
                let forward = !key.modifiers.contains(KeyModifiers::SHIFT);
                self.set_filter(self.filter.step(forward));
            }
            KeyCode::Char(digit @ '1'..='6') if plain => {
                let index = usize::from(digit as u8 - b'1');
                self.set_filter(Filter::ALL[index]);
            }
            KeyCode::Enter | KeyCode::Right => return self.open_selected(),
            _ => self.navigate(&key),
        }
        None
    }

    /// The keymap's `agents` actions: `Some` when `key` is one of them.
    fn session_key(&mut self, key: &KeyEvent) -> Option<Option<CenterAction>> {
        let action = SESSION_ACTIONS
            .into_iter()
            .find(|action| keymap::matches(*action, key))?;
        let selected = self.selected_row().map(|row| row.summary);
        let shown = |id| Some(id) == self.current;
        let done = match (action, selected) {
            (Action::AgentsSearch, _) => {
                self.mode = Mode::Search(String::new());
                None
            }
            (Action::AgentsNewTask, _) => Some(CenterAction::New),
            (Action::AgentsNewWorktree, _) => Some(CenterAction::NewWorktree),
            (Action::AgentsToggleGrouping, _) => {
                self.grouping = grouping::next(self.grouping);
                Some(CenterAction::Grouping(self.grouping))
            }
            (_, None) => None,
            (Action::AgentsFork, Some(session)) => Some(CenterAction::Fork(session.session_id)),
            (Action::AgentsRename, Some(session)) => {
                self.mode = Mode::Rename {
                    session_id: session.session_id,
                    title: session.title.map(|title| title.value).unwrap_or_default(),
                };
                None
            }
            (Action::AgentsStop, Some(session)) if runs(&session) => {
                Some(CenterAction::Stop(session.session_id))
            }
            (Action::AgentsStop, Some(_)) => {
                self.notice = Some("Nothing runs there.".to_string());
                None
            }
            (Action::AgentsArchive, Some(session)) if self.filter == Filter::Archived => {
                Some(CenterAction::SetArchived {
                    session_id: session.session_id,
                    archive: false,
                })
            }
            (Action::AgentsArchive, Some(session)) if shown(session.session_id) => {
                self.notice = Some("Use /archive for the session shown.".to_string());
                None
            }
            (Action::AgentsDelete, Some(session)) if shown(session.session_id) => {
                self.notice = Some("Use /delete for the session shown.".to_string());
                None
            }
            (Action::AgentsArchive | Action::AgentsDelete, Some(session)) => {
                self.mode = Mode::Confirm(Confirm {
                    session_id: session.session_id,
                    title: super::render::title_of(&session),
                    delete: action == Action::AgentsDelete,
                    accept: false,
                });
                None
            }
            _ => None,
        };
        Some(done)
    }

    fn search_key(&mut self, key: KeyEvent) -> Option<CenterAction> {
        let Mode::Search(query) = &mut self.mode else {
            return None;
        };
        match key.code {
            KeyCode::Esc => {}
            _ if ctrl_c(&key) => {}
            KeyCode::Enter => {
                let action = self.open_selected();
                self.leave_search();
                return action;
            }
            KeyCode::Backspace => {
                query.pop();
                self.searched();
                return None;
            }
            KeyCode::Char(character) if is_text(&key) => {
                query.push(character);
                self.searched();
                return None;
            }
            // The search key again ends the search, as Esc does.
            _ if keymap::matches(Action::AgentsSearch, &key) => {}
            _ => {
                self.navigate(&key);
                return None;
            }
        }
        self.leave_search();
        None
    }

    fn rename_key(&mut self, key: KeyEvent) -> Option<CenterAction> {
        let Mode::Rename { session_id, title } = &mut self.mode else {
            return None;
        };
        match key.code {
            KeyCode::Esc => {}
            _ if ctrl_c(&key) => {}
            KeyCode::Enter => {
                let title = title.trim().to_string();
                if title.is_empty() {
                    return None;
                }
                let session_id = *session_id;
                self.mode = Mode::Browse;
                return Some(CenterAction::Rename { session_id, title });
            }
            KeyCode::Backspace => {
                title.pop();
                return None;
            }
            KeyCode::Char(character) if is_text(&key) => {
                title.push(character);
                return None;
            }
            _ => return None,
        }
        self.mode = Mode::Browse;
        None
    }

    fn confirm_key(&mut self, key: KeyEvent) -> Option<CenterAction> {
        let Mode::Confirm(confirm) = &mut self.mode else {
            return None;
        };
        match key.code {
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Char('j' | 'k') => {
                confirm.accept = !confirm.accept;
                return None;
            }
            KeyCode::Enter if confirm.accept => {
                let action = if confirm.delete {
                    CenterAction::Delete(confirm.session_id)
                } else {
                    CenterAction::SetArchived {
                        session_id: confirm.session_id,
                        archive: true,
                    }
                };
                self.mode = Mode::Browse;
                return Some(action);
            }
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('n' | 'q') => {}
            _ if ctrl_c(&key) => {}
            _ => return None,
        }
        self.mode = Mode::Browse;
        None
    }

    /// The selection moved by an arrow, a page, or to an end; the arrows
    /// wrap around.
    fn navigate(&mut self, key: &KeyEvent) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let last = rows.len() - 1;
        let index = self.selected_index(&rows);
        let page = self.view_rows.unwrap_or(10).max(1);
        let next = match key.code {
            KeyCode::Up | KeyCode::Char('k') if index == 0 => last,
            KeyCode::Up | KeyCode::Char('k') => index - 1,
            KeyCode::Down | KeyCode::Char('j') if index == last => 0,
            KeyCode::Down | KeyCode::Char('j') => index + 1,
            KeyCode::PageUp => index.saturating_sub(page),
            KeyCode::PageDown => (index + page).min(last),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return,
        };
        self.selected = Some(rows[next].summary.session_id);
    }

    /// Enter or Right: the session opened, or restored under Archived.
    fn open_selected(&self) -> Option<CenterAction> {
        let session = self.selected_row()?.summary;
        Some(if self.filter == Filter::Archived {
            CenterAction::SetArchived {
                session_id: session.session_id,
                archive: false,
            }
        } else {
            CenterAction::Open(Box::new(session))
        })
    }

    fn set_filter(&mut self, filter: Filter) {
        if filter == self.filter {
            return;
        }
        let refetch = filter.lifecycle() != self.filter.lifecycle();
        self.filter = filter;
        self.scroll_top = 0;
        if refetch {
            self.reload(self.current);
        }
    }

    /// The search changed: the list is read again for it.
    fn searched(&mut self) {
        self.scroll_top = 0;
        self.selected = None;
        self.reload(self.current);
    }

    fn leave_search(&mut self) {
        let searched = self.query().is_some();
        self.mode = Mode::Browse;
        if searched {
            self.reload(self.current);
        }
    }
}

/// The session actions of the keymap's `agents` context, as `?` lists them.
pub(super) const SESSION_ACTIONS: [Action; 9] = [
    Action::AgentsNewTask,
    Action::AgentsNewWorktree,
    Action::AgentsFork,
    Action::AgentsRename,
    Action::AgentsStop,
    Action::AgentsArchive,
    Action::AgentsDelete,
    Action::AgentsSearch,
    Action::AgentsToggleGrouping,
];

fn ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_text(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}
