// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the `processTurn` pipeline of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Pins that the opt-in transforms change nothing while they are off: the
// golden was recorded before they existed (`COMPRESSION_BLESS=1` rewrites
// it, which must only be done on purpose).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use hanafubuki_agent_compression::{
    process_turn, Config, Message, RenderStrategy, State, TurnInput, TurnOutput,
};

use crate::fixtures::{apply, assistant, call, range, result, user, with_call};

const GOLDEN: &str = include_str!("defaults.json");

fn turn(messages: &[Message], state: State, config: &Config, token_count: u64) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config,
        token_count,
        token_count_source: None,
        render: RenderStrategy::TextAndToolResults,
        clock: None,
    })
}

fn config() -> Config {
    let mut config = Config::new(100_000);
    config.compress.min_compress_range = 0;
    config.compress.min_summary_length = 0;
    config.preserve_recent_messages = 2;
    config.preserve_recent_tokens = 0;
    config
}

fn rows() -> String {
    let rows: Vec<Value> = (0..300)
        .map(|index| json!({"id": index, "level": "INFO", "service": "api", "region": "eu-west-1"}))
        .collect();
    Value::Array(rows).to_string()
}

fn history() -> Vec<Message> {
    vec![
        user("u1", "run the build and read the config"),
        call("c1", "call1", "bash", r#"{"command":"npm run build"}"#),
        result("r1", "call1", "bash", &"line of build output\n".repeat(800)),
        call("c2", "call2", "read", r#"{"path":"rows.json"}"#),
        result("r2", "call2", "read", &rows()),
        assistant(
            "a1",
            &"the build passed and the rows are uniform ".repeat(100),
        ),
        call(
            "c0",
            "call0",
            "compress",
            r#"{"content":"m00090–m00099 nothing"}"#,
        ),
        result(
            "r0",
            "call0",
            "compress",
            "[Compression FAILED: no such range]",
        ),
        user("u2", "now fold that"),
    ]
}

fn folded(mut messages: Vec<Message>) -> Vec<Message> {
    messages.extend([
        call(
            "c3",
            "call3",
            "compress",
            r#"{"content":"m00001–m00006 build\nThe build passed; rows.json holds 300 uniform INFO rows."}"#,
        ),
        result("r3", "call3", "compress", "Compressed 1 range."),
        call("c4", "call4", "bash", r#"{"command":"cargo test"}"#),
        result("r4", "call4", "bash", &"test result: ok\n".repeat(1500)),
        assistant("a2", "the tests pass"),
        user("u3", "and again"),
    ]);
    messages
}

fn record(output: &TurnOutput) -> Value {
    let messages: Vec<Value> = output
        .messages
        .iter()
        .map(|message| {
            json!({
                "id": message.id,
                "role": format!("{:?}", message.role),
                "kind": format!("{:?}", message.kind),
                "text": fingerprint(&message.text),
                "tool_name": message.tool_name,
                "tool_call_id": message.tool_call_id,
            })
        })
        .collect();
    json!({
        "messages": messages,
        "state": serde_json::to_value(&output.state).unwrap_or_default(),
        "nudge": format!("{:?}", output.nudge),
        "status_line": output.status_line,
        "terminal_escape": format!("{:?}", output.terminal_escape),
        "truncation_skipped": output.truncation_skipped,
    })
}

/// Long texts as their length and digest, short ones as they are.
fn fingerprint(text: &str) -> String {
    if text.len() <= 400 {
        return text.to_owned();
    }
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{} bytes, sha256 {hex}", text.len())
}

fn turns() -> Vec<TurnOutput> {
    let config = config();
    let first = turn(&history(), State::default(), &config, 60_000);
    let fold = apply(
        &[with_call(
            range(
                "m00001",
                "m00006",
                "The build passed; rows.json holds 300 uniform INFO rows.",
            ),
            "call3",
        )],
        &history(),
        &first.state,
        &config,
    );
    assert_eq!(fold.result.blocks_created, 1, "{:?}", fold.result.errors);
    let messages = folded(history());
    let second = turn(&messages, fold.state, &config, 80_000);
    let third = turn(&messages, second.state.clone(), &config, 97_000);
    let fourth = turn(&messages, third.state.clone(), &config, 97_500);
    vec![first, second, third, fourth]
}

#[test]
fn every_transform_off_leaves_the_turns_as_they_were() {
    let recorded: Vec<Value> = turns().iter().map(record).collect();
    if std::env::var_os("COMPRESSION_BLESS").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/compression/opt_in/defaults.json"
        );
        let text = serde_json::to_string_pretty(&recorded).unwrap_or_default();
        std::fs::write(path, text + "\n").unwrap_or_default();
        return;
    }
    let golden: Vec<Value> = serde_json::from_str(GOLDEN).unwrap_or_default();
    assert_eq!(golden.len(), recorded.len());
    for (index, (golden, recorded)) in golden.iter().zip(&recorded).enumerate() {
        for key in [
            "messages",
            "nudge",
            "status_line",
            "terminal_escape",
            "truncation_skipped",
        ] {
            assert_eq!(golden[key], recorded[key], "turn {index}: {key}");
        }
        // The state may have gained fields since; the golden's decodes with
        // their defaults and must equal what the turn persists now.
        let then: State = serde_json::from_value(golden["state"].clone()).unwrap_or_default();
        let now: State = serde_json::from_value(recorded["state"].clone()).unwrap_or_default();
        assert_eq!(then, now, "turn {index}: state");
    }
}
