// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/config.ts` and `kernel/src/types.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the host predicates are data (a message's
// own protection flag, the bare `compress` tool name), and the nudge keys no
// code read (`frequency`, `iterationThreshold`, `force`) are gone; the
// validation keeps only the rules the unsigned and optional types do not
// already enforce.

/// The compression settings, with the kernel's defaults from
/// [`Config::new`]. Tool patterns are a name or a `prefix*` glob, matched
/// without regard to case.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The model's context window in tokens.
    pub model_context_limit: u64,
    pub tiers: TierConfig,
    pub nudge: NudgeConfig,
    /// Requests a block survives before it counts as old.
    pub promotion_threshold: u32,
    pub truncate: TruncateConfig,
    pub compress: CompressLimits,
    /// Tools whose calls and results never fold.
    pub protected_tools: Vec<String>,
    /// Tools whose latest call (and its result) never folds.
    pub protected_latest_tools: Vec<String>,
    /// Tools whose results do not count toward the recent zone; `None` is
    /// the built-in list ([`NEVER_PRESERVE_RECENT_TOOLS`](crate::NEVER_PRESERVE_RECENT_TOOLS)).
    pub never_preserve_recent_tools: Option<Vec<String>>,
    /// Taken out of the never-preserve list.
    pub preserve_recent_tools: Vec<String>,
    /// The newest messages that stay visible.
    pub preserve_recent_messages: usize,
    /// The newest tokens that stay visible.
    pub preserve_recent_tokens: u64,
}

impl Config {
    pub fn new(model_context_limit: u64) -> Self {
        Self {
            model_context_limit,
            tiers: TierConfig {
                enabled: true,
                tier2_trigger: 1000,
                tier3_trigger: 2000,
            },
            nudge: NudgeConfig {
                max_context_limit_pct: 0.75,
                min_context_limit_pct: 0.45,
                growth_ratio: 0.05,
                growth_floor: 50_000,
                growth_cap: 50_000,
                min_growth_floor: 20_000,
                min_growth_ratio: 0.45,
                emergency_threshold_pct: 0.95,
                tier2_growth_multiplier: 1.5,
                tier_growth_tokens: TierGrowthTokens::default(),
                min_pressure_benefit_tokens: None,
                require_usage_sample_for_pressure: true,
                max_estimate_hold_rounds: 2,
            },
            promotion_threshold: 5,
            truncate: TruncateConfig {
                threshold: 0.95,
                terminal_escape_after: 3,
            },
            compress: CompressLimits {
                min_compress_range: 5000,
                max_summary_length: 20_000,
                min_summary_length: 50,
            },
            protected_tools: Vec::new(),
            protected_latest_tools: Vec::new(),
            never_preserve_recent_tools: None,
            preserve_recent_tools: Vec::new(),
            preserve_recent_messages: 3,
            preserve_recent_tokens: 5000,
        }
    }

    /// The settings that would keep thresholds from firing as intended,
    /// one message each; empty when the configuration is sound.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut check = |ok: bool, message: &str| {
            if !ok {
                errors.push(message.to_owned());
            }
        };
        let nudge = &self.nudge;
        check(
            self.model_context_limit > 0,
            "modelContextLimit must be a positive number",
        );
        check(
            nudge.min_context_limit_pct <= nudge.max_context_limit_pct,
            "nudge.minContextLimitPct must not exceed nudge.maxContextLimitPct",
        );
        check(
            nudge.max_context_limit_pct <= nudge.emergency_threshold_pct,
            "nudge.maxContextLimitPct must not exceed nudge.emergencyThresholdPct",
        );
        check(
            self.promotion_threshold >= 1,
            "promotionThreshold must be >= 1",
        );
        check(
            self.truncate.threshold > 0.0 && self.truncate.threshold <= 1.0,
            "truncate.threshold must be in (0, 1]",
        );
        for trigger in [self.tiers.tier2_trigger, self.tiers.tier3_trigger] {
            check(trigger >= 1, "tier triggers must be >= 1");
        }
        check(
            self.tiers.tier3_trigger > self.tiers.tier2_trigger,
            "tiers.tier3Trigger must be greater than tiers.tier2Trigger",
        );
        let growth = &nudge.tier_growth_tokens;
        for (tier, value) in [("t1", growth.t1), ("t2", growth.t2), ("t3", growth.t3)] {
            if value == Some(0) {
                errors.push(format!(
                    "nudge.tierGrowthTokens.{tier} must be a positive number"
                ));
            }
        }
        errors
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TierConfig {
    pub enabled: bool,
    /// Active tier-1 blocks that make tier-2 distillation due.
    pub tier2_trigger: usize,
    /// Active tier-2 blocks that make tier-3 condensation due.
    pub tier3_trigger: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NudgeConfig {
    /// Context usage from which the pressure band nudges.
    pub max_context_limit_pct: f64,
    pub min_context_limit_pct: f64,
    /// The growth step is the window times this, clamped to
    /// `growth_floor..=growth_cap`.
    pub growth_ratio: f64,
    pub growth_floor: u64,
    pub growth_cap: u64,
    /// Growth needed before nudging again:
    /// `max(min_growth_floor, min_growth_ratio × step)`.
    pub min_growth_floor: u64,
    pub min_growth_ratio: f64,
    pub emergency_threshold_pct: f64,
    /// Tier-2 and tier-3 thresholds are the step times this.
    pub tier2_growth_multiplier: f64,
    pub tier_growth_tokens: TierGrowthTokens,
    /// `None`: the larger of 5000 tokens and 1% of the window.
    pub min_pressure_benefit_tokens: Option<u64>,
    /// The pressure band waits for a measured token count.
    pub require_usage_sample_for_pressure: bool,
    /// For at most this many consecutive estimated counts.
    pub max_estimate_hold_rounds: u32,
}

/// Per-tier growth thresholds; an unset one is derived from the step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TierGrowthTokens {
    pub t1: Option<u64>,
    pub t2: Option<u64>,
    pub t3: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TruncateConfig {
    /// Context usage at which tool outputs are clipped as a last resort.
    pub threshold: f64,
    /// Stuck requests before the terminal escape.
    pub terminal_escape_after: u32,
}

/// What a `compress` call must meet; a zero turns a limit off.
#[derive(Clone, Debug, PartialEq)]
pub struct CompressLimits {
    /// Characters (UTF-16 code units) a batch of message ranges must span.
    pub min_compress_range: usize,
    pub max_summary_length: usize,
    pub min_summary_length: usize,
}
