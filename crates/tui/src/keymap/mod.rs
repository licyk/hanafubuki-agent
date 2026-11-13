// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The keymap (Codex's `keymap.rs` and `keymap/`): which keys trigger each
//! of the interface's actions ([`actions`], [`catalogue`]), resolved from `tui.keymap`
//! over the defaults ([`resolve`]), with chords and the leader key
//! ([`chords`]).
//!
//! The interface reads its keys on one thread, the app loop's, so the
//! keymap in force is that thread's ([`current`], [`install`]): every text
//! field, view and overlay asks it, and `/keymap` replaces it when it saves.
//! A test thread starts with the defaults.
//!
//! A chord completed is handed on as a key of its own, a function key past
//! the keyboard's (its token), which the action's keys include: the handlers
//! read a chord's action as they read a key's.

mod actions;
mod catalogue;
mod chords;
mod resolve;
mod spec;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) use actions::{Action, Context};
pub(crate) use catalogue::ACTIONS;
pub(crate) use chords::{ChordState, ChordStep};
pub(crate) use resolve::{conflicts, parse_keys};
pub(crate) use spec::{Binding, KeyStroke};

/// The first function key that stands for a completed chord.
const FIRST_TOKEN: u8 = 100;

/// The keys of every action.
#[derive(Debug)]
pub(crate) struct Keymap {
    /// Each action's bindings, by `Action as usize`.
    bindings: Vec<Vec<Binding>>,
    /// Whether the configuration set the action's bindings.
    configured: Vec<bool>,
    /// The keys each action matches: its keys, and its chords' tokens.
    strokes: Vec<Vec<KeyStroke>>,
    chords: Vec<Chord>,
}

/// A chord, with the action it triggers and its token.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Chord {
    pub(crate) first: KeyStroke,
    pub(crate) second: KeyStroke,
    pub(crate) action: Action,
    token: u8,
}

thread_local! {
    static CURRENT: RefCell<Rc<Keymap>> = RefCell::new(Rc::new(Keymap::default()));
}

/// The keymap in force on this thread.
pub(crate) fn current() -> Rc<Keymap> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Makes `keymap` the one in force on this thread.
pub(crate) fn install(keymap: Keymap) {
    CURRENT.with(|current| *current.borrow_mut() = Rc::new(keymap));
}

/// The first key of `action` in the keymap in force, for a hint.
pub(crate) fn hint(action: Action) -> Option<crate::key_hint::KeyBinding> {
    CURRENT.with(|current| current.borrow().hint(action))
}

/// `action`'s keys when the configuration names none.
pub(crate) fn default_bindings(action: Action) -> Vec<Binding> {
    resolve::defaults().swap_remove(action as usize)
}

/// Whether `event` triggers `action` in the keymap in force.
pub(crate) fn matches(action: Action, event: &KeyEvent) -> bool {
    CURRENT.with(|current| current.borrow().matches(action, event))
}

impl Default for Keymap {
    fn default() -> Self {
        Self::resolve(&BTreeMap::new()).0
    }
}

impl Keymap {
    /// The keymap `config` (`tui.keymap`) asks for, and what of it was left
    /// out.
    pub(crate) fn resolve(
        config: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
    ) -> (Self, Vec<String>) {
        let (bindings, configured, warnings) = resolve::resolve(config);
        (Self::from_bindings(bindings, configured), warnings)
    }

    fn from_bindings(bindings: Vec<Vec<Binding>>, configured: Vec<bool>) -> Self {
        let leaders: Vec<KeyStroke> = bindings[Action::Leader as usize]
            .iter()
            .filter_map(|binding| match binding {
                Binding::Key(key) => Some(*key),
                _ => None,
            })
            .collect();
        let mut chords = Vec::new();
        let mut strokes = Vec::with_capacity(bindings.len());
        for spec in &ACTIONS {
            let mut keys = Vec::new();
            for binding in &bindings[spec.action as usize] {
                let pairs = match *binding {
                    Binding::Key(key) => {
                        keys.push(key);
                        continue;
                    }
                    Binding::Chord(first, second) => vec![(first, second)],
                    Binding::Leader(second) => {
                        leaders.iter().map(|leader| (*leader, second)).collect()
                    }
                };
                for (first, second) in pairs {
                    let Some(token) = u8::try_from(chords.len())
                        .ok()
                        .and_then(|index| FIRST_TOKEN.checked_add(index))
                    else {
                        continue;
                    };
                    chords.push(Chord {
                        first,
                        second,
                        action: spec.action,
                        token,
                    });
                    keys.extend(KeyStroke::new(KeyCode::F(token), KeyModifiers::NONE));
                }
            }
            strokes.push(keys);
        }
        Self {
            bindings,
            configured,
            strokes,
            chords,
        }
    }

    /// Whether `event` triggers `action`.
    pub(crate) fn matches(&self, action: Action, event: &KeyEvent) -> bool {
        self.strokes[action as usize]
            .iter()
            .any(|key| key.matches(event))
    }

    pub(crate) fn bindings(&self, action: Action) -> &[Binding] {
        &self.bindings[action as usize]
    }

    pub(crate) fn is_configured(&self, action: Action) -> bool {
        self.configured[action as usize]
    }

    /// The first key of `action` to show in a hint; `None` when it has
    /// only chords or nothing.
    pub(crate) fn hint(&self, action: Action) -> Option<crate::key_hint::KeyBinding> {
        self.bindings(action)
            .iter()
            .find_map(|binding| match binding {
                Binding::Key(key) => Some(key.hint()),
                _ => None,
            })
    }

    /// `action`'s keys as `tui.keymap` writes them (`ctrl-t, f3`), or
    /// `unbound`.
    pub(crate) fn label(&self, action: Action) -> String {
        let keys: Vec<String> = self.bindings(action).iter().map(Binding::spec).collect();
        if keys.is_empty() {
            "unbound".to_string()
        } else {
            keys.join(", ")
        }
    }

    /// The chords read in `contexts`.
    pub(crate) fn chords_in<'a>(
        &'a self,
        contexts: &'a [Context],
    ) -> impl Iterator<Item = &'a Chord> + 'a {
        self.chords
            .iter()
            .filter(|chord| contexts.contains(&chord.action.spec().context))
    }

    /// The bindings with `action`'s replaced by `bindings`: the keymap
    /// `/keymap` would save, and the conflicts it would have.
    pub(crate) fn with(&self, action: Action, bindings: Vec<Binding>) -> Vec<Vec<Binding>> {
        let mut all = self.bindings.clone();
        all[action as usize] = bindings;
        all
    }
}

impl Chord {
    /// The key that stands for this chord, handed on once it is completed.
    pub(crate) fn token(&self) -> KeyEvent {
        KeyEvent::new(KeyCode::F(self.token), KeyModifiers::NONE)
    }
}

#[cfg(test)]
mod tests;
