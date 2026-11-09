// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Two keys in a row (Codex's `keymap/chords.rs`): the first key of a chord
//! read where the input is waits for the second, which completes the chord
//! (handed on as its token) or cancels it; either way the keys are not
//! handed on themselves. An ordinary chord's first key expires after a
//! second; the leader waits until the next key.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::{Action, Context, KeyStroke, Keymap};

/// How long an ordinary chord's first key waits for the second.
pub(crate) const CHORD_TIMEOUT: Duration = Duration::from_secs(1);

/// The first key of a chord, while it waits.
#[derive(Debug, Default)]
pub(crate) struct ChordState {
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    first: KeyStroke,
    at: Instant,
    leader: bool,
}

/// What became of a key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChordStep {
    /// No chord: the key goes where it goes.
    Pass,
    /// The first key of a chord: kept, waiting for the second.
    Wait,
    /// The chord's token, to hand on in place of the keys.
    Complete(KeyEvent),
    /// The second key completed no chord; both are dropped.
    Cancelled,
}

impl ChordState {
    /// Takes `event`, read where `contexts` are.
    pub(crate) fn feed(
        &mut self,
        keymap: &Keymap,
        event: &KeyEvent,
        contexts: &[Context],
        now: Instant,
    ) -> ChordStep {
        if let Some(pending) = self.pending.take() {
            let expired = !pending.leader && now.duration_since(pending.at) > CHORD_TIMEOUT;
            if !expired {
                if event.kind == KeyEventKind::Release {
                    self.pending = Some(pending);
                    return ChordStep::Wait;
                }
                if event.code == KeyCode::Esc {
                    return ChordStep::Cancelled;
                }
                return keymap
                    .chords_in(contexts)
                    .find(|chord| chord.first == pending.first && chord.second.matches(event))
                    .map_or(ChordStep::Cancelled, |chord| {
                        ChordStep::Complete(chord.token())
                    });
            }
        }
        if event.kind != KeyEventKind::Press {
            return ChordStep::Pass;
        }
        let Some(first) = KeyStroke::of(event) else {
            return ChordStep::Pass;
        };
        if !keymap.chords_in(contexts).any(|chord| chord.first == first) {
            return ChordStep::Pass;
        }
        let leader = keymap
            .bindings(Action::Leader)
            .contains(&super::Binding::Key(first));
        self.pending = Some(Pending {
            first,
            at: now,
            leader,
        });
        ChordStep::Wait
    }

    /// The leader was pressed and waits for its second key.
    pub(crate) fn leader_pending(&self) -> bool {
        self.pending.as_ref().is_some_and(|pending| pending.leader)
    }

    #[cfg(test)]
    pub(crate) fn pending(&self) -> Option<KeyStroke> {
        self.pending.as_ref().map(|pending| pending.first)
    }

    pub(crate) fn clear(&mut self) {
        self.pending = None;
    }
}
