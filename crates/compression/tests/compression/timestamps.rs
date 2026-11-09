// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/tests/issue4313-message-timestamps.test.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Ports its cases through `process_turn` with the host's clock (`clock`
// present turns the timestamps on, as the kernel's `Ports.timestamps`). Not
// ported: a time a message carries itself (no host message carries one) and
// the lean pack's TIMELINE rule (the lean pack is not ported). Adds the
// offset each stamp keeps, and the stamps a fork adopts.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    adopt_fork_state, build_compress_system_prompt, format_tag_time, process_turn, render_refs,
    Config, LocalTime, Message, Prompts, RenderStrategy, State, TurnInput, TurnOutput,
};

use crate::fixtures::{apply, assign, range, state_for, user};

/// UTC 2026-09-28 07:59:09.
const SEPTEMBER: u64 = 1_790_582_349_000;
/// UTC 2026-01-04 19:04:05.
const JANUARY: u64 = 1_767_553_445_000;
/// Asia/Shanghai, as the kernel's tests pin it.
const SHANGHAI: i32 = 8 * 3600;

fn at(ms: u64) -> LocalTime {
    LocalTime {
        at: ms,
        utc_offset_seconds: SHANGHAI,
    }
}

fn turn(messages: &[Message], state: State, clock: Option<LocalTime>) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config: &Config::new(100_000),
        token_count: 100,
        token_count_source: None,
        render: RenderStrategy::TextOnly,
        clock,
    })
}

fn texts(output: &TurnOutput) -> Vec<&str> {
    output
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect()
}

#[test]
fn a_stamp_formats_as_the_local_time_it_was_read_in() {
    assert_eq!(format_tag_time(at(SEPTEMBER)), "2026-09-28 15:59:09");
    // Zero-padded, and the day boundary is the local one, not UTC's.
    assert_eq!(format_tag_time(at(JANUARY)), "2026-01-05 03:04:05");
    let new_york = LocalTime {
        at: JANUARY,
        utc_offset_seconds: -5 * 3600,
    };
    assert_eq!(format_tag_time(new_york), "2026-01-04 14:04:05");
    let utc = LocalTime {
        at: 951_782_400_000,
        utc_offset_seconds: 0,
    };
    assert_eq!(format_tag_time(utc), "2000-02-29 00:00:00");
}

#[test]
fn first_sight_mints_a_stamp_once_and_every_render_shows_it() {
    let messages = [user("a", "alpha"), user("b", "beta")];
    let first = turn(&messages, State::default(), Some(at(SEPTEMBER)));
    assert_eq!(
        texts(&first),
        [
            "<acp tokens=\"2\" type=\"text\" time=\"2026-09-28 15:59:09\">m00001</acp>\nalpha",
            "<acp tokens=\"1\" type=\"text\" time=\"2026-09-28 15:59:09\">m00002</acp>\nbeta",
        ]
    );
    // A later request reads the clock again; the stamps do not move.
    let second = turn(&messages, first.state.clone(), Some(at(JANUARY)));
    assert_eq!(texts(&second), texts(&first));
    assert_eq!(
        first.state.ts_stamps.keys().collect::<Vec<_>>(),
        ["a", "b"],
        "stamped by raw id, no entry for an id never seen"
    );
    assert_eq!(second.state.ts_stamps, first.state.ts_stamps);
}

#[test]
fn a_message_arriving_later_is_stamped_later() {
    let first = turn(&[user("a", "alpha")], State::default(), Some(at(JANUARY)));
    let later = turn(
        &[user("a", "alpha"), user("b", "beta")],
        first.state,
        Some(at(SEPTEMBER)),
    );
    assert_eq!(
        texts(&later),
        [
            "<acp tokens=\"2\" type=\"text\" time=\"2026-01-05 03:04:05\">m00001</acp>\nalpha",
            "<acp tokens=\"1\" type=\"text\" time=\"2026-09-28 15:59:09\">m00002</acp>\nbeta",
        ]
    );
}

#[test]
fn a_stamp_keeps_its_offset_when_the_hosts_changes() {
    let first = turn(&[user("a", "alpha")], State::default(), Some(at(SEPTEMBER)));
    let moved = LocalTime {
        at: SEPTEMBER + 60_000,
        utc_offset_seconds: 0,
    };
    let second = turn(
        &[user("a", "alpha"), user("b", "beta")],
        first.state,
        Some(moved),
    );
    assert_eq!(
        texts(&second),
        [
            "<acp tokens=\"2\" type=\"text\" time=\"2026-09-28 15:59:09\">m00001</acp>\nalpha",
            "<acp tokens=\"1\" type=\"text\" time=\"2026-09-28 08:00:09\">m00002</acp>\nbeta",
        ]
    );
}

#[test]
fn an_id_seen_before_timestamps_were_on_keeps_its_untimed_tag() {
    let state = state_for(&[user("a", "alpha")]);
    let output = turn(&[user("a", "alpha")], state, Some(at(SEPTEMBER)));
    assert_eq!(
        texts(&output),
        ["<acp tokens=\"2\" type=\"text\">m00001</acp>\nalpha"]
    );
    assert!(output.state.ts_stamps.is_empty(), "never backfilled");
}

#[test]
fn rendering_reads_only_the_stored_stamps() {
    let messages = [user("a", "alpha"), user("b", "beta")];
    let mut state = state_for(&messages);
    state.ts_stamps.insert("a".to_owned(), at(SEPTEMBER));
    state.ts_stamps.insert("b".to_owned(), at(JANUARY));
    let first = render_refs(&messages, &mut state, RenderStrategy::TextOnly, true);
    let second = render_refs(&messages, &mut state, RenderStrategy::TextOnly, true);
    assert_eq!(first, second);
    assert!(first[0].text.contains(" time=\"2026-09-28 15:59:09\">"));
    assert!(first[1].text.contains(" time=\"2026-01-05 03:04:05\">"));
}

#[test]
fn the_render_path_reads_no_clock() {
    let source = include_str!("../../src/render/mod.rs");
    for clock in ["SystemTime", "Instant", "UNIX_EPOCH"] {
        assert!(!source.contains(clock), "render reads {clock}");
    }
}

#[test]
fn a_fold_keeps_the_stamps() {
    let body = "payload ".repeat(500);
    let messages: Vec<Message> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .into_iter()
        .map(|id| user(id, &body))
        .collect();
    let stamped = turn(&messages, State::default(), Some(at(SEPTEMBER))).state;
    assert_eq!(stamped.ts_stamps.len(), 8);
    let outcome = apply(
        &[range(
            "m00001",
            "m00005",
            "Messages a through e carry identical large payload bodies; nothing else was decided here.",
        )],
        &messages,
        &stamped,
        &Config::new(100_000),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.state.ts_stamps, stamped.ts_stamps);
}

#[test]
fn without_a_clock_nothing_is_stamped_or_shown() {
    let messages = [user("a", "alpha"), user("b", "beta")];
    let off = turn(&messages, State::default(), None);
    assert_eq!(
        texts(&off),
        [
            "<acp tokens=\"2\" type=\"text\">m00001</acp>\nalpha",
            "<acp tokens=\"1\" type=\"text\">m00002</acp>\nbeta",
        ]
    );
    assert!(off.state.ts_stamps.is_empty());
    // Turned on later, only the messages first seen from then are stamped.
    let on = turn(
        &[user("a", "alpha"), user("b", "beta"), user("c", "gamma")],
        off.state,
        Some(at(SEPTEMBER)),
    );
    assert_eq!(on.state.ts_stamps.keys().collect::<Vec<_>>(), ["c"]);
    assert!(!on.messages[0].text.contains(" time="));
    assert!(on.messages[2]
        .text
        .contains(" time=\"2026-09-28 15:59:09\">"));
}

#[test]
fn stored_stamps_stay_hidden_while_timestamps_are_off() {
    let messages = [user("a", "alpha")];
    let stamped = turn(&messages, State::default(), Some(at(SEPTEMBER))).state;
    let off = turn(&messages, stamped.clone(), None);
    assert_eq!(
        texts(&off),
        ["<acp tokens=\"2\" type=\"text\">m00001</acp>\nalpha"]
    );
    assert_eq!(off.state.ts_stamps, stamped.ts_stamps, "kept, not shown");
}

#[test]
fn the_prompt_shows_the_time_attribute_only_with_timestamps() {
    let off = build_compress_system_prompt(&Prompts::default(), false);
    assert!(off.contains("<acp tokens=\"2.1K\" type=\"tool:bash\">m00175</acp>"));
    assert!(!off.contains("time=\""));
    assert!(!off.contains("TIMELINE"));
    let on = build_compress_system_prompt(&Prompts::default(), true);
    assert!(on.contains(
        "<acp tokens=\"2.1K\" type=\"tool:bash\" time=\"2026-10-30 09:02:11\">m00175</acp>"
    ));
    assert!(on.contains(
        "Ordering: smaller ref number = earlier message; smaller time = earlier message."
    ));
    assert!(
        !on.contains("TIMELINE"),
        "the default rules carry no diary line"
    );
    let tags = |prompt: &str| {
        let start = prompt.find("ACP TAGS").unwrap();
        let end = prompt.find("\n\nTOOLS\n").unwrap();
        (prompt[..start].to_owned(), prompt[end..].to_owned())
    };
    assert_eq!(tags(&on), tags(&off), "only the ACP TAGS section differs");
}

#[test]
fn a_fork_keeps_the_stamps_of_the_messages_it_holds() {
    let messages = [user("a", "alpha"), user("b", "beta")];
    let mut parent = State::default();
    assign(&messages, &mut parent);
    parent.ts_stamps.insert("a".to_owned(), at(SEPTEMBER));
    parent.ts_stamps.insert("b".to_owned(), at(JANUARY));
    let fork = adopt_fork_state(&parent, &HashSet::from(["a"]));
    assert_eq!(fork.ts_stamps.keys().collect::<Vec<_>>(), ["a"]);
    assert_eq!(fork.ts_stamps["a"], at(SEPTEMBER));
}
