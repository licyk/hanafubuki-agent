use super::*;

fn started_step() -> AgentReasoningCheckpoint {
    let mut checkpoint = AgentReasoningCheckpoint::default();
    checkpoint.begin_step(1).unwrap();
    checkpoint
}

#[test]
fn first_item_starts_and_last_item_completes_timing() {
    let mut checkpoint = started_step();
    assert!(checkpoint.item_started(100).unwrap());
    assert_eq!(
        checkpoint.snapshot(),
        AgentReasoningSnapshot {
            sampling_step: 1,
            phase: AgentReasoningPhase::Thinking,
            active_since: Some(100),
            step_completed_millis: 0,
            turn_completed_millis: 0,
            last_completed_at: None,
        }
    );

    assert!(checkpoint.item_completed(145).unwrap());
    assert_eq!(checkpoint.snapshot().phase, AgentReasoningPhase::Completed);
    assert_eq!(checkpoint.snapshot().step_completed_millis, 45);
    assert_eq!(checkpoint.snapshot().turn_completed_millis, 45);
    assert_eq!(checkpoint.snapshot().last_completed_at, Some(145));
}

#[test]
fn overlapping_items_count_the_active_union_once() {
    let mut checkpoint = started_step();
    assert!(checkpoint.item_started(100).unwrap());
    assert!(!checkpoint.item_started(110).unwrap());
    assert!(!checkpoint.item_completed(130).unwrap());
    assert_eq!(checkpoint.snapshot().active_since, Some(100));
    assert!(checkpoint.item_completed(150).unwrap());
    assert_eq!(checkpoint.snapshot().step_completed_millis, 50);
    assert_eq!(checkpoint.snapshot().turn_completed_millis, 50);
}

#[test]
fn sequential_intervals_and_steps_accumulate_without_carrying_step_state() {
    let mut checkpoint = started_step();
    checkpoint.item_started(10).unwrap();
    checkpoint.item_completed(20).unwrap();
    checkpoint.item_started(30).unwrap();
    checkpoint.item_completed(50).unwrap();
    assert_eq!(checkpoint.snapshot().step_completed_millis, 30);
    assert_eq!(checkpoint.snapshot().turn_completed_millis, 30);

    checkpoint.begin_step(2).unwrap();
    assert_eq!(
        checkpoint.snapshot().phase,
        AgentReasoningPhase::NotObserved
    );
    assert_eq!(checkpoint.snapshot().step_completed_millis, 0);
    assert_eq!(checkpoint.snapshot().turn_completed_millis, 30);
    checkpoint.item_started(80).unwrap();
    checkpoint.item_completed(85).unwrap();
    assert_eq!(checkpoint.snapshot().step_completed_millis, 5);
    assert_eq!(checkpoint.snapshot().turn_completed_millis, 35);
}

#[test]
fn terminal_close_settles_one_active_interval() {
    let mut checkpoint = started_step();
    checkpoint.item_started(100).unwrap();
    checkpoint.item_started(120).unwrap();
    assert!(checkpoint.terminal_close(160).unwrap());
    assert_eq!(checkpoint.snapshot().phase, AgentReasoningPhase::Completed);
    assert_eq!(checkpoint.snapshot().step_completed_millis, 60);
    assert!(!checkpoint.terminal_close(200).unwrap());
    assert_eq!(checkpoint.snapshot().step_completed_millis, 60);
}

#[test]
fn recovery_abort_drops_unconfirmed_interval_and_preserves_completed_time() {
    let mut checkpoint = started_step();
    checkpoint.item_started(10).unwrap();
    checkpoint.item_completed(20).unwrap();
    checkpoint.item_started(30).unwrap();
    assert!(checkpoint.recovery_abort().unwrap());
    assert_eq!(checkpoint.snapshot().phase, AgentReasoningPhase::Completed);
    assert_eq!(checkpoint.snapshot().active_since, None);
    assert_eq!(checkpoint.snapshot().step_completed_millis, 10);
    assert_eq!(checkpoint.snapshot().turn_completed_millis, 10);
    assert_eq!(checkpoint.snapshot().last_completed_at, Some(20));
}

#[test]
fn illegal_transitions_and_backwards_clock_are_typed_errors() {
    let mut checkpoint = AgentReasoningCheckpoint::default();
    assert!(matches!(
        checkpoint.item_started(1),
        Err(AgentReasoningTransitionError::InvalidSamplingStep { .. })
    ));
    checkpoint.begin_step(1).unwrap();
    assert_eq!(
        checkpoint.item_completed(1),
        Err(AgentReasoningTransitionError::ItemNotActive)
    );
    checkpoint.item_started(20).unwrap();
    let before_error = checkpoint;
    assert_eq!(
        checkpoint.item_completed(19),
        Err(AgentReasoningTransitionError::ClockMovedBackwards)
    );
    assert_eq!(checkpoint, before_error);
}

#[test]
fn overflow_and_invalid_checkpoint_are_rejected() {
    let mut checkpoint = AgentReasoningCheckpoint {
        snapshot: AgentReasoningSnapshot {
            sampling_step: 1,
            phase: AgentReasoningPhase::Completed,
            active_since: None,
            step_completed_millis: u128::MAX,
            turn_completed_millis: u128::MAX,
            last_completed_at: Some(1),
        },
        active_reasoning_item_count: 0,
    };
    checkpoint.item_started(2).unwrap();
    let before_error = checkpoint;
    assert_eq!(
        checkpoint.item_completed(3),
        Err(AgentReasoningTransitionError::ArithmeticOverflow)
    );
    assert_eq!(checkpoint, before_error);

    let invalid = AgentReasoningCheckpoint {
        snapshot: AgentReasoningSnapshot {
            sampling_step: 1,
            phase: AgentReasoningPhase::Thinking,
            active_since: None,
            step_completed_millis: 0,
            turn_completed_millis: 0,
            last_completed_at: None,
        },
        active_reasoning_item_count: 1,
    };
    assert_eq!(
        invalid.validate(),
        Err(AgentReasoningTransitionError::InvalidCheckpoint)
    );
}

#[test]
fn checkpoint_json_contains_only_lifecycle_and_timing_metadata() {
    let mut checkpoint = started_step();
    checkpoint.item_started(100).unwrap();
    let json = serde_json::to_value(checkpoint).unwrap();
    assert_eq!(json["activeReasoningItemCount"], 1);
    assert!(json.get("itemId").is_none());
    assert!(json.get("summary").is_none());
    assert!(json.get("content").is_none());
}
