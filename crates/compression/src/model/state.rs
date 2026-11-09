// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/types.ts` and `kernel/src/state.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: every field decodes with a default (the
// kernel's `mergeCompressionState` dropped the fields it did not list), and
// the ledger, timestamp, absorb, rule and instance-id fields are not kept.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A session's whole compression state: plain data, persisted by the host
/// and passed into and out of every call. Refs, block ids and run ids are
/// never reissued, `message_refs` only grows, and blocks are deactivated,
/// never deleted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub blocks: Vec<Block>,
    pub message_refs: RefMap,
    /// The token count each ref showed the first time it was rendered, kept
    /// so the rendered tags never change between requests.
    pub token_snapshot: BTreeMap<String, u64>,
    pub nudge: NudgeState,
    pub stats: Stats,
    /// Consecutive requests over the truncation threshold with nothing left
    /// to reclaim.
    pub terminal_streak: u32,
    /// Refs of the orphan `compress` calls and results hidden from the view,
    /// so a range endpoint on one snaps to its visible neighbour.
    pub hidden_orphan_refs: Vec<String>,
    /// Orphan `compress` call ids swept at a fold; they stay hidden.
    pub swept_orphan_call_ids: Vec<String>,
    /// The lowest ref number of the last fold, until the next view sweeps
    /// the orphans at or after it.
    pub pending_orphan_sweep_min_ref: Option<u32>,
    /// Texts clipped by the emergency truncation, replayed until the next
    /// fold.
    pub emergency_truncate_latch: Vec<TruncateLatchEntry>,
    /// Refs that stopped being protected; their tag waits for the next fold.
    pub render_deferred_refs: Vec<String>,
    /// Transformed view texts by ref, replayed until the next fold.
    pub view_latches: BTreeMap<String, ViewLatch>,
    pub view_latch_config_digest: Option<String>,
    /// Refs that no longer back any visible or folded message.
    pub dead_refs: Vec<String>,
    pub next_block_id: u32,
    pub next_run_id: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            message_refs: RefMap::default(),
            token_snapshot: BTreeMap::new(),
            nudge: NudgeState::default(),
            stats: Stats::default(),
            terminal_streak: 0,
            hidden_orphan_refs: Vec::new(),
            swept_orphan_call_ids: Vec::new(),
            pending_orphan_sweep_min_ref: None,
            emergency_truncate_latch: Vec::new(),
            render_deferred_refs: Vec::new(),
            view_latches: BTreeMap::new(),
            view_latch_config_digest: None,
            dead_refs: Vec::new(),
            next_block_id: 1,
            next_run_id: 1,
        }
    }
}

impl State {
    pub fn block(&self, block_id: &str) -> Option<&Block> {
        self.blocks.iter().find(|block| block.block_id == block_id)
    }

    pub(crate) fn block_mut(&mut self, block_id: &str) -> Option<&mut Block> {
        self.blocks
            .iter_mut()
            .find(|block| block.block_id == block_id)
    }

    pub fn active_blocks(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter().filter(|block| block.active)
    }

    /// The message ids every active block covers.
    pub fn covered_message_ids(&self) -> std::collections::HashSet<&str> {
        self.active_blocks()
            .flat_map(|block| block.effective_message_ids.iter().map(String::as_str))
            .collect()
    }

    pub fn highest_active_tier(&self) -> u8 {
        self.active_blocks()
            .map(|block| block.tier)
            .max()
            .unwrap_or(0)
    }

    /// Ages every active block by one request; a block that survived
    /// `promotion_threshold` requests becomes old.
    pub fn advance_survival(&mut self, promotion_threshold: u32) {
        for block in self.blocks.iter_mut().filter(|block| block.active) {
            block.survived_count += 1;
            if block.survived_count >= promotion_threshold {
                block.generation = Generation::Old;
            }
        }
    }

    pub(crate) fn allocate_block_id(&mut self) -> String {
        let id = self.next_block_id;
        self.next_block_id = id.max(1) + 1;
        format!("b{id}")
    }

    pub(crate) fn allocate_run_id(&mut self) -> String {
        let id = self.next_run_id;
        self.next_run_id = id.max(1) + 1;
        format!("r{id}")
    }
}

/// A folded range: a tier-1 block summarises messages, a tier-2 block
/// distils tier-1 blocks, a tier-3 block condenses tier-2 blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Block {
    pub block_id: String,
    pub run_id: String,
    /// 1, 2 or 3.
    pub tier: u8,
    pub topic: Option<String>,
    pub summary: String,
    /// The message ids this block folded itself.
    pub direct_message_ids: Vec<String>,
    /// Every message id it covers, through the blocks it consumed too.
    pub effective_message_ids: Vec<String>,
    /// The blocks it consumed.
    pub direct_block_ids: Vec<String>,
    /// Tokens of what it replaced.
    pub compressed_tokens: u64,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    pub survived_count: u32,
    pub generation: Generation,
    pub active: bool,
    /// The user expanded it for good; it stays inactive.
    pub expanded: bool,
    /// Its content was restored into the view, so a compress over the same
    /// span updates it in place.
    pub restored_inline: bool,
    /// The tool-call id of the `compress` call that carries its summary.
    pub compress_call_id: Option<String>,
    /// The range as requested.
    pub start_ref: Option<String>,
    pub end_ref: Option<String>,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            block_id: String::new(),
            run_id: String::new(),
            tier: 1,
            topic: None,
            summary: String::new(),
            direct_message_ids: Vec::new(),
            effective_message_ids: Vec::new(),
            direct_block_ids: Vec::new(),
            compressed_tokens: 0,
            created_at: 0,
            survived_count: 0,
            generation: Generation::Young,
            active: true,
            expanded: false,
            restored_inline: false,
            compress_call_id: None,
            start_ref: None,
            end_ref: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Generation {
    #[default]
    Young,
    Old,
}

/// Message id to ref and back. A protected message maps to
/// [`BLOCKED_REF`](crate::BLOCKED_REF) in `by_raw` and has no `by_ref` entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefMap {
    pub by_raw: BTreeMap<String, String>,
    pub by_ref: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NudgeState {
    pub last_per_message_nudge_tokens: u64,
    pub last_nudge_shown_tokens: u64,
    /// The token count at which each tier was last nudged.
    pub last_shown_by_tier: BTreeMap<u8, u64>,
    pub absorbed_debt: u64,
    pub absorb_count: u32,
    /// The tier-1 pending tokens the last tier-1 nudge offered.
    pub last_t1_fire_pending: Option<u64>,
    /// The last tier-1 nudge was answered by a fold that took less than half
    /// of what it offered.
    pub t1_harvest_stall: bool,
    /// Consecutive requests whose token count was an estimate.
    pub estimate_hold_streak: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub tokens_compressed: u64,
    pub compression_count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TruncateLatchEntry {
    pub r#ref: String,
    pub text: String,
}

/// A latched view text: `None` keeps the source text.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewLatch {
    pub text: Option<String>,
    pub hidden: bool,
}
