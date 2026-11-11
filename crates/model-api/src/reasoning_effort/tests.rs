use super::*;

fn effort(value: &str) -> ModelReasoningEffort {
    ModelReasoningEffort::parse(value).unwrap()
}

fn supported(levels: &[&str]) -> ModelReasoningSupport {
    ModelReasoningSupport::Supported {
        efforts: levels.iter().map(|level| effort(level)).collect(),
        default: None,
    }
}

fn sent(configured: Option<&str>, support: Option<&ModelReasoningSupport>) -> Option<String> {
    let configured = configured.map(effort);
    request_reasoning_effort(configured.as_ref(), support).map(|effort| effort.to_string())
}

#[test]
fn no_configured_effort_sends_none_whatever_is_known() {
    for support in [
        None,
        Some(ModelReasoningSupport::Unsupported),
        Some(supported(&[])),
        Some(supported(&["low", "high"])),
    ] {
        assert_eq!(sent(None, support.as_ref()), None, "{support:?}");
    }
}

#[test]
fn an_unknown_route_or_one_without_levels_sends_the_configured_effort() {
    assert_eq!(sent(Some("minimal"), None).as_deref(), Some("minimal"));
    assert_eq!(
        sent(Some("vendor.tier-2"), Some(&supported(&[]))).as_deref(),
        Some("vendor.tier-2")
    );
}

#[test]
fn an_unsupported_route_sends_none() {
    assert_eq!(
        sent(Some("high"), Some(&ModelReasoningSupport::Unsupported)),
        None
    );
}

#[test]
fn a_listed_effort_is_sent_as_configured() {
    let levels = supported(&["low", "medium", "high", "vendor.tier-2"]);
    for configured in ["low", "high", "vendor.tier-2"] {
        assert_eq!(
            sent(Some(configured), Some(&levels)).as_deref(),
            Some(configured)
        );
    }
}

#[test]
fn an_alias_wins_over_the_nearest_rank() {
    // `xhigh` ties between `high` and `max`; its alias is `max`.
    let levels = supported(&["low", "high", "max"]);
    assert_eq!(sent(Some("xhigh"), Some(&levels)).as_deref(), Some("max"));
    let levels = supported(&["low", "medium", "high"]);
    assert_eq!(sent(Some("minimal"), Some(&levels)).as_deref(), Some("low"));
    assert_eq!(sent(Some("light"), Some(&levels)).as_deref(), Some("low"));
    assert_eq!(
        sent(Some("balanced"), Some(&levels)).as_deref(),
        Some("medium")
    );
    assert_eq!(sent(Some("deep"), Some(&levels)).as_deref(), Some("high"));
}

#[test]
fn otherwise_the_nearest_listed_level_is_sent_and_a_tie_goes_lower() {
    // No alias listed: `xhigh` (4) is nearest `high` (3).
    let levels = supported(&["low", "medium", "high"]);
    assert_eq!(sent(Some("xhigh"), Some(&levels)).as_deref(), Some("high"));
    // `medium` (2) ties between `low` (1) and `high` (3).
    let levels = supported(&["low", "high"]);
    assert_eq!(sent(Some("medium"), Some(&levels)).as_deref(), Some("low"));
    // `minimal` (0), with `low` not listed, goes to `medium`.
    let levels = supported(&["medium", "max"]);
    assert_eq!(
        sent(Some("minimal"), Some(&levels)).as_deref(),
        Some("medium")
    );
    // `max` (5) with only lower levels takes the highest.
    let levels = supported(&["minimal", "low", "high"]);
    assert_eq!(sent(Some("max"), Some(&levels)).as_deref(), Some("high"));
}

#[test]
fn an_effort_without_a_rank_sends_none_unless_listed() {
    let levels = supported(&["low", "high"]);
    assert_eq!(sent(Some("vendor.tier-2"), Some(&levels)), None);
    // Listed levels without a rank are no candidates for a ranked effort.
    let levels = supported(&["vendor.tier-1", "vendor.tier-2"]);
    assert_eq!(sent(Some("high"), Some(&levels)), None);
}

#[test]
fn the_ladder_orders_the_known_efforts() {
    let ranks = ["minimal", "low", "medium", "high", "xhigh", "max"]
        .map(|value| reasoning_effort_rank(&effort(value)));
    assert_eq!(ranks, [0, 1, 2, 3, 4, 5].map(Some));
    assert_eq!(reasoning_effort_rank(&effort("light")), Some(1));
    assert_eq!(reasoning_effort_rank(&effort("future")), None);
}
