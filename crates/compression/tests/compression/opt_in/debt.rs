// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `tests/issue5415-absorb-debt.test.ts` at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the host side of #5415 (the nudge side is `nudge/debt.rs`): an
// aborted turn's growth is banked and counted. The anchor is the token count
// the aborted turn was sent with; the stale-flag case is the host's (it
// keeps the flag and its age).

use std::collections::BTreeMap;

use hanafubuki_agent_compression::NudgeState;

fn nudge(reference: u64) -> NudgeState {
    NudgeState {
        last_per_message_nudge_tokens: reference,
        last_nudge_shown_tokens: 12_000,
        last_shown_by_tier: BTreeMap::from([(1, 12_000)]),
        ..NudgeState::default()
    }
}

#[test]
fn an_abort_banks_the_skipped_growth() {
    let mut state = nudge(50_000);
    assert!(state.absorb_aborted_turn_growth(60_000, 70_000));
    assert_eq!(state.last_per_message_nudge_tokens, 70_000);
    assert_eq!(state.absorbed_debt, 10_000);
    assert_eq!(state.absorb_count, 1);
    assert_eq!(state.last_nudge_shown_tokens, 0);
    assert!(state.last_shown_by_tier.is_empty());
}

#[test]
fn repeated_aborts_accumulate() {
    let mut state = nudge(50_000);
    state.absorb_aborted_turn_growth(60_000, 70_000);
    state.absorb_aborted_turn_growth(73_000, 75_000);
    state.absorb_aborted_turn_growth(80_000, 82_000);
    assert_eq!(state.absorbed_debt, 18_000);
    assert_eq!(state.absorb_count, 3);
}

#[test]
fn a_reference_past_the_anchor_banks_nothing() {
    let mut state = nudge(90_000);
    assert!(state.absorb_aborted_turn_growth(85_000, 95_000));
    assert_eq!(state.absorbed_debt, 0);
    assert_eq!(state.absorb_count, 1);
    assert_eq!(state.last_per_message_nudge_tokens, 95_000);
}

#[test]
fn nothing_moves_until_the_count_passes_the_anchor() {
    let mut state = nudge(50_000);
    assert!(!state.absorb_aborted_turn_growth(60_000, 58_000));
    assert_eq!(state, nudge(50_000));
}
