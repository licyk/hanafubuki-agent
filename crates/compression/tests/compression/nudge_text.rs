// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `nudge-text.test.ts`. The `golden/nudge-*.txt` files are the kernel's
// own `renderNudgeText` output for the decisions built in `crossed`, rendered
// with bun from `kernel/src` at the same commit.

use crate::fixtures::crossed;
use hanafubuki_agent_compression::{
    render_nudge_text, Block, BlockSpan, CompressibleRange, NudgeDecision, NudgeVoice, Prompts,
    RenderedNudge,
};

fn ranges(count: usize) -> Vec<CompressibleRange> {
    (0..count)
        .map(|index| CompressibleRange {
            start_ref: format!("m{:05}", index * 3 + 1),
            end_ref: format!("m{:05}", index * 3 + 3),
            count: 3,
            tokens: 1000 * (index as u64 + 1),
            ..CompressibleRange::default()
        })
        .collect()
}

fn decision() -> NudgeDecision {
    NudgeDecision {
        should_inject: true,
        reason: "test".to_owned(),
        compressible_ranges: ranges(3),
        context_usage: 0.5,
        ..NudgeDecision::default()
    }
}

fn emergency() -> NudgeDecision {
    let mut decision = decision();
    decision.context_usage = 0.99;
    decision.breakdown.emergency_override = true;
    decision
}

fn tier(tier: u8) -> NudgeDecision {
    NudgeDecision {
        tier: Some(tier),
        ..decision()
    }
}

fn render(decision: &NudgeDecision) -> RenderedNudge {
    render_nudge_text(decision, &Prompts::default(), false)
}

fn render_anthropic(decision: &NudgeDecision) -> RenderedNudge {
    render_nudge_text(decision, &Prompts::default(), true)
}

fn spans(count: usize) -> Vec<BlockSpan> {
    (0..count)
        .map(|index| BlockSpan {
            block_id: format!("b{}", index + 1),
            tier: 1,
            start_ref: format!("m{:05}", index * 10 + 1),
            end_ref: format!("m{:05}", index * 10 + 9),
        })
        .collect()
}

fn with_spans(decision: NudgeDecision, spans: Vec<BlockSpan>) -> NudgeDecision {
    NudgeDecision {
        active_block_spans: spans,
        ..decision
    }
}

#[test]
fn gentle_voice_and_header() {
    let result = render(&decision());
    assert_eq!(result.voice, NudgeVoice::Gentle);
    assert!(result.text.contains("efficiency nudge"));
    assert!(result.text.contains("not an overflow warning"));
}

#[test]
fn the_gentle_tip_is_conditional_and_licenses_deferral() {
    let gentle = render(&decision()).text;
    assert!(gentle.contains("If you compress"));
    assert!(!gentle.contains("Compress all ranges in one call"));
    assert!(gentle.contains("they reappear in later nudges"));
    let emergency = render(&emergency()).text;
    assert!(!emergency.contains("If you compress"));
    assert!(emergency.contains("compress now"));
}

#[test]
fn emergency_voice_and_header() {
    let result = render(&emergency());
    assert_eq!(result.voice, NudgeVoice::Emergency);
    assert!(result.text.contains("Context limit reached"));
    assert!(result.text.contains("compress now"));
}

#[test]
fn the_gentle_voice_has_no_emergency_language() {
    let text = render(&decision()).text;
    assert!(!text.contains("MUST compress"));
    assert!(!text.contains("⚠️"));
}

#[test]
fn the_emergency_voice_has_no_gentle_language() {
    let text = render(&emergency()).text;
    assert!(!text.contains("EFFICIENCY NUDGE"));
    assert!(!text.contains("not an overflow warning"));
}

#[test]
fn a_tier_2_nudge_names_the_tier_and_distillation() {
    let text = render(&tier(2)).text;
    assert!(text.contains("TIER 2"));
    assert!(text.contains("Distill"));
}

#[test]
fn a_tier_3_nudge_names_the_tier_and_condensation() {
    let text = render(&tier(3)).text;
    assert!(text.contains("TIER 3"));
    assert!(text.contains("Condense"));
}

#[test]
fn tier_guidance_says_raw_messages_in_the_span_are_absorbed() {
    for tier_number in [2, 3] {
        let text = render(&tier(tier_number)).text;
        assert!(text.contains("raw"));
        assert!(text.contains("absorbed"));
        assert!(text.contains("HOW TO COMPRESS"));
    }
}

#[test]
fn an_emergency_tier_nudge_keeps_its_tier_guidance() {
    for (tier_number, word) in [(2, "distill"), (3, "condense")] {
        let mut decision = tier(tier_number);
        decision.breakdown.emergency_override = true;
        let result = render(&decision);
        assert_eq!(result.voice, NudgeVoice::Emergency);
        assert!(result.text.to_lowercase().contains(word));
        assert!(result.text.contains(&format!("TIER {tier_number}")));
    }
}

#[test]
fn both_voices_list_the_compressible_ranges() {
    for decision in [decision(), emergency()] {
        let text = render(&decision).text;
        assert!(text.contains("Compressible ranges"));
        assert!(text.contains("m00001"));
        assert!(text.contains("1.0K") || text.contains("6.0K"));
    }
}

#[test]
fn both_voices_carry_the_keep_and_drop_guide() {
    for decision in [decision(), emergency()] {
        let text = render(&decision).text;
        for needle in ["KEEP VERBATIM", "DROP", "PRIORITY", "file paths"] {
            assert!(text.contains(needle), "{needle}");
        }
    }
}

#[test]
fn empty_ranges_are_handled() {
    let decision = NudgeDecision {
        compressible_ranges: Vec::new(),
        ..decision()
    };
    assert!(render(&decision).text.contains("No specific ranges"));
}

#[test]
fn the_usage_percentage_is_not_shown() {
    let decision = NudgeDecision {
        context_usage: 0.62,
        ..decision()
    };
    assert!(!render(&decision).text.contains("62%"));
}

#[test]
fn over_limit_renders_with_the_emergency_voice() {
    let mut decision = decision();
    decision.context_usage = 0.85;
    decision.breakdown.over_limit = true;
    let result = render(&decision);
    assert_eq!(result.voice, NudgeVoice::Emergency);
    assert!(!result.text.contains("not an overflow warning"));
}

#[test]
fn the_gentle_nudge_renders_the_active_block_map() {
    let text = render(&with_spans(decision(), spans(2))).text;
    assert!(text.contains("Active blocks (2): b1=m00001–m00009 · b2=m00011–m00019"));
}

#[test]
fn the_block_map_marks_higher_tiers() {
    let mut spans = spans(1);
    spans[0].tier = 2;
    assert!(render(&with_spans(decision(), spans))
        .text
        .contains("b1=m00001–m00009 t2"));
}

#[test]
fn the_block_map_keeps_the_newest_eight() {
    let text = render(&with_spans(decision(), spans(10))).text;
    assert!(text.contains("Active blocks (10): …+2 older · "));
    assert!(!text.contains("b1="));
    assert!(text.contains("b8="));
    assert!(text.contains("b10="));
}

#[test]
fn no_block_map_without_active_blocks() {
    assert!(!render(&decision()).text.contains("Active blocks"));
}

#[test]
fn the_emergency_nudge_renders_the_block_map_too() {
    let result = render(&with_spans(emergency(), spans(1)));
    assert_eq!(result.voice, NudgeVoice::Emergency);
    assert!(result.text.contains("Active blocks (1): b1=m00001–m00009"));
}

#[test]
fn range_lines_count_user_messages() {
    let decision = NudgeDecision {
        compressible_ranges: vec![
            CompressibleRange {
                start_ref: "m00001".to_owned(),
                end_ref: "m00005".to_owned(),
                count: 5,
                tokens: 2000,
                user_msgs: 3,
                ..CompressibleRange::default()
            },
            CompressibleRange {
                start_ref: "m00010".to_owned(),
                end_ref: "m00011".to_owned(),
                count: 2,
                tokens: 500,
                tool_pct: 100,
                user_msgs: 1,
                ..CompressibleRange::default()
            },
        ],
        ..decision()
    };
    let text = render(&decision).text;
    assert!(text.contains("· 3 user msgs"));
    assert!(text.contains("· 1 user msg"));
}

#[test]
fn no_user_message_note_without_user_messages() {
    assert!(!render(&decision()).text.contains("user msg"));
}

#[test]
fn the_gentle_nudge_carries_the_static_one_call_hint() {
    let text = render(&decision()).text;
    assert!(text.contains("fold the ranges you keep in ONE call"));
    assert!(!text.contains("<write your summary of this range>"));
    assert!(!text.contains("<topic>"));
}

#[test]
fn the_emergency_nudge_carries_the_static_hint_unconditionally() {
    let text = render(&emergency()).text;
    assert!(text.contains("ONE call, ONE string"));
    assert!(!text.contains("If you compress"));
    assert!(!text.contains("<write your summary of this range>"));
}

#[test]
fn the_hint_never_grows_with_the_range_count() {
    let few = render(&NudgeDecision {
        compressible_ranges: ranges(2),
        ..decision()
    })
    .text;
    let many = render(&NudgeDecision {
        compressible_ranges: ranges(24),
        ..decision()
    })
    .text;
    assert_eq!(few.matches("ONE call, ONE string").count(), 0);
    assert_eq!(many.matches("ONE call, ONE string").count(), 0);
    assert!(many.len() < few.len() + 200 * 22);
}

#[test]
fn tier_nudges_carry_no_message_ref_hint() {
    let text = render(&tier(2)).text;
    assert!(!text.contains("ONE call, ONE string"));
    assert!(!text.contains("<write your summary of this range>"));
}

const LANE_NOTE: &str = "Write the summary ONLY as the `compress` tool's `content` argument";

#[test]
fn the_anthropic_lane_names_the_envelope_failure_and_the_required_shape() {
    let text = render_anthropic(&decision()).text;
    assert!(text.contains(LANE_NOTE));
    assert!(text.contains("Never emit `<analysis>` or `<summary>` tags"));
    assert!(text.contains("a reply without a tool call ends the turn"));
}

#[test]
fn the_anthropic_lane_note_rides_emergency_and_tier_nudges() {
    let mut emergency = emergency();
    emergency.context_usage = 0.95;
    assert!(render_anthropic(&emergency).text.contains(LANE_NOTE));
    assert!(render_anthropic(&tier(2)).text.contains(LANE_NOTE));
}

#[test]
fn other_lanes_have_no_lane_note() {
    assert!(!render(&decision()).text.contains(LANE_NOTE));
}

#[test]
fn the_lane_note_adds_no_blank_line_runs_and_ends_the_text() {
    let base = render(&decision()).text;
    let anthropic = render_anthropic(&decision()).text;
    assert_eq!(
        anthropic.matches("\n\n\n").count(),
        base.matches("\n\n\n").count()
    );
    assert!(anthropic.ends_with("ends the turn."));
}

// Byte-exact against the kernel.

fn target(id: &str, summary: String, topic: Option<&str>, tokens: u64, covered: usize) -> Block {
    Block {
        block_id: id.to_owned(),
        summary,
        topic: topic.map(str::to_owned),
        compressed_tokens: tokens,
        effective_message_ids: (0..covered).map(|index| format!("{id}_{index}")).collect(),
        ..Block::default()
    }
}

#[test]
fn the_gentle_nudge_is_byte_identical_to_the_kernel() {
    assert_eq!(
        render(&crossed()).text,
        include_str!("golden/nudge-gentle.txt")
    );
}

#[test]
fn the_emergency_anthropic_nudge_is_byte_identical_to_the_kernel() {
    let mut decision = crossed();
    decision.breakdown.emergency_override = true;
    assert_eq!(
        render_anthropic(&decision).text,
        include_str!("golden/nudge-emergency-anthropic.txt")
    );
}

#[test]
fn the_tier_2_nudge_is_byte_identical_to_the_kernel() {
    let decision = NudgeDecision {
        tier: Some(2),
        tier_target_blocks: vec![
            target("b2", "x".repeat(1801), Some("auth flow"), 12_345, 7),
            target("b5", "short".to_owned(), None, 999, 2),
        ],
        ..crossed()
    };
    assert_eq!(
        render(&decision).text,
        include_str!("golden/nudge-tier2.txt")
    );
}
