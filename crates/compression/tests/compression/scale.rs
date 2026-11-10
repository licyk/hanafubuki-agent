//! `process_turn`'s cost per call over long histories, unfolded, half
//! folded and folded, quiet and under pressure. Run in release:
//! `cargo test --release -p hanafubuki-agent-compression --test compression
//! scale -- --ignored --nocapture`. Each line ends with a digest of the two
//! outputs, so a change that must not alter the view compares them before
//! and after.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use hanafubuki_agent_compression::{
    apply_compression, process_turn, ApplyInput, Config, Message, RangeSpec, RenderStrategy, State,
    TurnInput, TurnOutput,
};

use crate::fixtures::{assistant, call, result, user};

const WINDOW: u64 = 200_000;

/// A round's first and last message ids, and the id of the `compress` call
/// of the round after it.
struct Round {
    first: String,
    last: String,
    folded_by: Option<String>,
}

/// Rounds of a short user message, three `read` calls with ~1.5 KB results
/// and a ~3 KB answer; from the second round on, a `compress` call after the
/// user message folds the round before.
fn session(messages: usize) -> (Vec<Message>, Vec<Round>) {
    let body = "lorem ipsum dolor sit amet, consectetur adipiscing elit ".repeat(27);
    let mut history = Vec::with_capacity(messages + 10);
    let mut rounds: Vec<Round> = Vec::new();
    while history.len() < messages {
        let round = rounds.len();
        let first = format!("r{round}u");
        history.push(user(&first, &format!("round {round}: look at the files")));
        if let Some(previous) = rounds.last_mut() {
            let call_id = format!("r{round}fold");
            let arguments = format!("{{\"content\":\"round {} summary\"}}", round - 1);
            history.push(call(
                &format!("r{round}c"),
                &call_id,
                "compress",
                &arguments,
            ));
            history.push(result(
                &format!("r{round}cr"),
                &call_id,
                "compress",
                "Compressed 1 range.",
            ));
            previous.folded_by = Some(call_id);
        }
        for read in 0..3 {
            let call_id = format!("r{round}read{read}");
            let path = format!("{{\"path\":\"src/file_{round}_{read}.rs\"}}");
            history.push(call(&format!("r{round}c{read}"), &call_id, "read", &path));
            let output = format!("{read} {body}");
            history.push(result(
                &format!("r{round}r{read}"),
                &call_id,
                "read",
                &output,
            ));
        }
        let last = format!("r{round}a");
        history.push(assistant(&last, &format!("{round} {body}{body}")));
        rounds.push(Round {
            first,
            last,
            folded_by: None,
        });
    }
    (history, rounds)
}

fn turn(messages: &[Message], state: State, token_count: u64) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config: &Config::new(WINDOW),
        token_count,
        token_count_source: None,
        render: RenderStrategy::TextAndToolResults,
        clock: None,
    })
}

/// The state after each of `rounds` was folded by the `compress` call of the
/// round after it.
fn folded(messages: &[Message], rounds: &[Round]) -> State {
    let state = turn(messages, State::default(), 0).state;
    let refs = &state.message_refs.by_raw;
    let ranges: Vec<RangeSpec> = rounds
        .iter()
        .map(|round| RangeSpec {
            start_ref: refs[&round.first].clone(),
            end_ref: refs[&round.last].clone(),
            summary: format!(
                "{}..{}: the user and the assistant read three files and answered.",
                round.first, round.last
            ),
            compress_call_id: round.folded_by.clone(),
            ..RangeSpec::default()
        })
        .collect();
    let outcome = apply_compression(&ApplyInput {
        ranges: &ranges,
        messages,
        state: &state,
        config: &Config::new(WINDOW),
        protected_refs: None,
        now_ms: 0,
    });
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result);
    outcome.state
}

/// The fastest of `runs` calls, each from the state one call left, and the
/// digest of its output.
fn time(messages: &[Message], state: &State, token_count: u64, runs: usize) -> (Duration, u64) {
    let state = turn(messages, state.clone(), token_count).state;
    let mut best = Duration::MAX;
    let mut digest = DefaultHasher::new();
    for _ in 0..runs {
        let input = state.clone();
        let start = Instant::now();
        let output = turn(messages, input, token_count);
        best = best.min(start.elapsed());
        digest = DefaultHasher::new();
        format!("{output:?}").hash(&mut digest);
    }
    (best, digest.finish())
}

#[test]
#[ignore = "release timing of process_turn over 1k to 20k messages"]
fn process_turn_scales_with_the_window() {
    for size in [1_000, 2_000, 5_000, 10_000, 20_000] {
        let (messages, rounds) = session(size);
        let runs = if size >= 10_000 { 3 } else { 5 };
        let unfolded = State::default();
        let half = folded(&messages, &rounds[..rounds.len() / 2]);
        let folded = folded(&messages, &rounds[..rounds.len() - 2]);
        for (name, state) in [("unfolded", unfolded), ("half", half), ("folded", folded)] {
            let (quiet, quiet_digest) = time(&messages, &state, 1_000, runs);
            let (pressed, pressed_digest) = time(&messages, &state, WINDOW * 4 / 5, runs);
            println!(
                "{:>6} messages {name:>8} ({:>4} blocks): {:>7.1} ms quiet, {:>7.1} ms pressed  {quiet_digest:016x} {pressed_digest:016x}",
                messages.len(),
                state.blocks.len(),
                quiet.as_secs_f64() * 1000.0,
                pressed.as_secs_f64() * 1000.0,
            );
        }
    }
}
