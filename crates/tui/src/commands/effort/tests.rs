//! `/effort`: what it asks of the Agent, its refusal during a turn, the
//! picker over the model's known efforts (only Default and Custom when
//! none is known), and a custom effort typed and checked.

use std::sync::mpsc::Receiver;

use hanafubuki_agent::{ModelMetadata, ModelMetadataSource, ModelReasoningSupport, Sourced};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::agent_bridge::{EffortChoices, TurnUpdate};
use crate::app_event::AppEvent;
use crate::chatwidget::ChatWidget;
use crate::commands::tests::session::{chat, key, run, screen, to_sent, Sent};

fn set(effort: Option<&str>) -> Sent {
    Sent::Op(Op::Command(AgentCommand::Effort(EffortCall::Set(
        effort.map(|effort| ModelReasoningEffort::parse(effort).unwrap()),
    ))))
}

/// What the chat sent, the picker's later steps run as they come.
fn sent_after_steps(chat: &mut ChatWidget, events: &Receiver<AppEvent>) -> Vec<Sent> {
    let mut sent = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::CommandStep(step) => chat.run_command_step(step),
            event => sent.extend(to_sent(event)),
        }
    }
    sent
}

fn reported(efforts: &[&str], default: Option<&str>) -> ModelMetadata {
    let parse = |effort: &str| ModelReasoningEffort::parse(effort).unwrap();
    ModelMetadata {
        reasoning: Some(Sourced {
            value: ModelReasoningSupport::Supported {
                efforts: efforts.iter().copied().map(parse).collect(),
                default: default.map(parse),
            },
            source: ModelMetadataSource::Reported,
            observed_at_ms: 0,
        }),
        ..ModelMetadata::default()
    }
}

fn show(chat: &mut ChatWidget, metadata: ModelMetadata, current: Option<&str>) {
    chat.handle_turn_update(TurnUpdate::EffortChoices(EffortChoices {
        model: "gpt-5".to_string(),
        current: current.map(|effort| ModelReasoningEffort::parse(effort).unwrap()),
        metadata,
    }));
}

fn type_text(chat: &mut ChatWidget, text: &str) {
    for character in text.chars() {
        key(chat, KeyCode::Char(character));
    }
}

#[test]
fn effort_lists_or_sets_a_typed_effort_or_the_default() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "effort"),
        [
            Sent::Op(Op::EnsureSession),
            Sent::Op(Op::Command(AgentCommand::Effort(EffortCall::List)))
        ]
    );
    assert_eq!(
        run(&mut chat, &events, "effort  max "),
        [Sent::Op(Op::EnsureSession), set(Some("max"))]
    );
    assert_eq!(
        run(&mut chat, &events, "effort default"),
        [Sent::Op(Op::EnsureSession), set(None)]
    );
    assert_eq!(
        run(&mut chat, &events, "effort none"),
        [
            Sent::Op(Op::EnsureSession),
            Sent::Cell(
                "■ /effort: reasoning effort 'none' is not supported; omit the setting for Provider default"
                    .to_string()
            )
        ]
    );
}

#[test]
fn no_effort_is_changed_during_a_turn() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Started);
    for line in ["effort", "effort high"] {
        let sent = run(&mut chat, &events, line);
        assert!(
            sent.contains(&Sent::Cell(format!("■ {DURING_TASK}"))),
            "{sent:?}"
        );
        assert!(
            !sent
                .iter()
                .any(|sent| matches!(sent, Sent::Op(Op::Command(AgentCommand::Effort(_))))),
            "{sent:?}"
        );
    }
}

#[test]
fn the_picker_offers_the_reported_levels_then_default_and_custom() {
    let (mut chat, events) = chat(false);
    show(
        &mut chat,
        reported(&["low", "medium", "high"], Some("medium")),
        Some("high"),
    );
    insta::assert_snapshot!("effort_picker_with_levels", screen(&chat, 80));
    sent_after_steps(&mut chat, &events);
    key(&mut chat, KeyCode::Char('1'));
    assert_eq!(sent_after_steps(&mut chat, &events), [set(Some("low"))]);
}

#[test]
fn with_no_known_levels_only_default_and_custom_are_offered() {
    let (mut chat, events) = chat(false);
    show(&mut chat, ModelMetadata::default(), None);
    insta::assert_snapshot!("effort_picker_without_levels", screen(&chat, 80));
    sent_after_steps(&mut chat, &events);
    key(&mut chat, KeyCode::Char('1'));
    assert_eq!(sent_after_steps(&mut chat, &events), [set(None)]);
}

#[test]
fn a_models_dev_guess_or_a_learned_refusal_offers_no_level() {
    let (mut guessing, _events) = chat(false);
    let mut guessed = reported(&["low", "high"], None);
    guessed.reasoning.as_mut().unwrap().source = ModelMetadataSource::Catalog;
    show(&mut guessing, guessed, None);
    let shown = screen(&guessing, 80);
    assert!(shown.contains("1. Default (current)"), "{shown}");
    assert!(!shown.contains("low"), "{shown}");

    let (mut chat, _events) = chat(false);
    let refused = ModelMetadata {
        reasoning: Some(Sourced {
            value: ModelReasoningSupport::Unsupported,
            source: ModelMetadataSource::Learned,
            observed_at_ms: 0,
        }),
        ..ModelMetadata::default()
    };
    show(&mut chat, refused, Some("high"));
    let shown = screen(&chat, 80);
    assert!(
        shown.contains("The Provider refused the reasoning parameter"),
        "{shown}"
    );
    assert!(shown.contains("2. Custom: high (current)"), "{shown}");
}

#[test]
fn a_custom_effort_is_typed_and_one_refused_stays_with_its_reason() {
    let (mut chat, events) = chat(false);
    show(&mut chat, reported(&["low", "high"], None), Some("max"));
    let shown = screen(&chat, 80);
    assert!(shown.contains("› 4. Custom: max (current)"), "{shown}");
    sent_after_steps(&mut chat, &events);
    key(&mut chat, KeyCode::Enter);
    assert!(sent_after_steps(&mut chat, &events).is_empty());
    let prompt = screen(&chat, 80);
    assert!(prompt.contains("Custom reasoning effort"), "{prompt}");
    assert!(prompt.contains("max"), "{prompt}");

    for _ in 0.."max".len() {
        key(&mut chat, KeyCode::Backspace);
    }
    type_text(&mut chat, "none");
    key(&mut chat, KeyCode::Enter);
    assert!(sent_after_steps(&mut chat, &events).is_empty());
    let refused = screen(&chat, 80);
    assert!(refused.contains("none"), "{refused}");
    assert!(
        refused.contains("reasoning effort 'none' is not supported"),
        "{refused}"
    );

    for _ in 0.."none".len() {
        key(&mut chat, KeyCode::Backspace);
    }
    type_text(&mut chat, "ultra");
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent_after_steps(&mut chat, &events), [set(Some("ultra"))]);
}

#[test]
fn the_effort_changed_is_said() {
    let (mut chat, events) = chat(false);
    for (effort, said) in [
        (Some("high"), "Reasoning effort set to high"),
        (None, "Reasoning effort set to the Provider's default"),
    ] {
        chat.handle_turn_update(TurnUpdate::EffortChanged(
            effort.map(|effort| ModelReasoningEffort::parse(effort).unwrap()),
        ));
        let sent = sent_after_steps(&mut chat, &events);
        assert!(
            sent.iter()
                .any(|sent| matches!(sent, Sent::Cell(cell) if cell.contains(said))),
            "{sent:?}"
        );
    }
}
