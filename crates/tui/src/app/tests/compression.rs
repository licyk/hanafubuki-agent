//! The hierarchical compression in the conversation, end to end: a fold
//! shows as one folded `compress` cell, live and replayed; `/compression`
//! shows the panel and the cache report, or says the session does not use
//! it; the status line's `compression` item counts the blocks.

use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, tool_events};
use hanafubuki_agent::{AgentCompactionSettings, AgentCompactionStrategy, AgentSettingsPatch};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

const SUMMARY: &str =
    "The user said hello and the assistant answered at length; nothing in it is still open.";
const RECEIPT: &str = "[Compressed m00002–m00002 → 1 block(s)";

impl Harness {
    /// The `hierarchical` strategy over a window of 100,000 tokens, with
    /// only the newest message kept out of a fold. The first answer is long
    /// enough to fold; the second response folds it.
    fn hierarchical(responses: Vec<Vec<u8>>) -> Self {
        let mut all = vec![
            assistant_events(&"The first answer is long. ".repeat(100)),
            String::from_utf8(tool_events(
                "compress",
                &serde_json::json!({ "content": format!("m00002–m00002 Greeting\n{SUMMARY}") })
                    .to_string(),
            ))
            .unwrap()
            .replace("call-tool", "fold-1")
            .into_bytes(),
            assistant_events("Folded."),
        ];
        all.extend(responses);
        let all = all.into_iter().map(|body| (body, Duration::ZERO)).collect();
        Self::start_configured(all, true, TranscriptMode::Terminal, |config| {
            config
                .mutate_providers(1, |registry| {
                    registry.model_policy.context_window_tokens = Some(100_000);
                    Ok(())
                })
                .unwrap();
            let mut compaction = AgentCompactionSettings {
                strategy: AgentCompactionStrategy::Hierarchical,
                ..AgentCompactionSettings::default()
            };
            compaction.hierarchical.preserve_recent_messages = 1;
            compaction.hierarchical.preserve_recent_tokens = 0;
            compaction.hierarchical.min_compress_range_chars = 1_000;
            config
                .write_settings(AgentSettingsPatch {
                    compaction: Some(compaction),
                    ..AgentSettingsPatch::default()
                })
                .unwrap();
        })
    }

    fn fold(&mut self) {
        self.exchange("Hello.", "The first answer is long.");
        self.exchange("Fold it.", "Folded.");
    }
}

#[test]
fn a_fold_shows_one_folded_cell_live_and_replayed() {
    let mut harness = Harness::hierarchical(Vec::new());
    harness.fold();

    let transcript = harness.transcript();
    assert_eq!(transcript.matches(RECEIPT).count(), 1, "{transcript}");
    assert!(
        transcript.contains("Called compress: m00002–m00002 Greeting"),
        "{transcript}"
    );
    // The summary and the receipt's fingerprints are folded away.
    assert!(!transcript.contains(SUMMARY), "{transcript}");
    assert!(!transcript.contains("Fingerprints"), "{transcript}");

    let session_id = harness.app.bridge.session_id().unwrap();
    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    harness.type_and_submit(&format!("/resume {}", session_id.value()));
    harness.pump_until("the resumed transcript", |harness| {
        harness.transcript().contains("Folded.")
    });
    let transcript = harness.transcript();
    assert_eq!(transcript.matches(RECEIPT).count(), 1, "{transcript}");
    assert!(
        transcript.contains("Called compress: m00002–m00002 Greeting"),
        "{transcript}"
    );
}

#[test]
fn compression_shows_the_panel_the_cache_report_and_the_status_item() {
    let mut harness = Harness::hierarchical(Vec::new());
    harness.fold();

    harness.type_and_submit("/compression");
    let screen = harness.pump_until_screen_shows("Blocks: 1 active / 1 total");
    assert!(screen.contains("ACP Context Analysis"), "{screen}");
    assert!(screen.contains("Greeting"), "{screen}");
    harness.pump_until_screen_shows("Stats: 1 compress call(s) applied");

    harness.type_and_submit("/compression cache");
    harness.pump_until_screen_shows("Compression cache");

    harness.type_and_submit("/compression what");
    harness.pump_until_screen_shows("Usage: /compression [cache]");

    harness.type_and_submit("/statusline compression");
    harness.pump_until("the status item", |harness| {
        harness.bottom_rows(3).contains("1 block, ")
    });
    assert!(harness.bottom_rows(3).contains(" saved"));
}

#[test]
fn a_session_without_hierarchical_compression_says_so() {
    let mut harness = Harness::start(vec![assistant_events("Hi.")]);
    harness.type_and_submit("/compression");
    harness.pump_until_screen_shows("not using hierarchical compression");
    harness.exchange("Hello.", "Hi.");
    harness.type_and_submit("/compression");
    harness.pump_until("a second answer", |harness| {
        harness
            .transcript()
            .matches("not using hierarchical compression")
            .count()
            == 2
    });
}
