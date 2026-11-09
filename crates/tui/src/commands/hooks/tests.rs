//! `/hooks`: the browser's pages over a list of hooks, what its keys ask
//! of the Agent, a failure, and the line at start.

use std::path::PathBuf;

use hanafubuki_agent::{
    AgentHook, AgentHookHandler, AgentHooks, HookEventName, HookSource, HookTrust,
};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::agent_bridge::TurnUpdate;
use crate::app_event::{AppEvent, Op};
use crate::chatwidget::ChatWidget;
use crate::commands::tests::session::{chat, key, run, screen, sent, Sent};

fn hooks_call(call: HooksCall) -> Sent {
    Sent::Op(Op::Command(AgentCommand::Hooks(call)))
}

fn hook(
    key: &str,
    event: HookEventName,
    matcher: Option<&str>,
    trust: HookTrust,
    enabled: bool,
) -> AgentHook {
    AgentHook {
        key: key.to_string(),
        event,
        matcher: matcher.map(str::to_string),
        handler: AgentHookHandler::Command {
            command: format!("~/bin/{key}.sh"),
            timeout_sec: 30,
            status_message: None,
        },
        source: HookSource::User,
        source_path: PathBuf::from("/home/me/.config/hanafubuki-agent/hooks.json"),
        trust,
        enabled,
        current_hash: "sha256:00".to_string(),
    }
}

fn listed() -> AgentHooks {
    AgentHooks {
        hooks: vec![
            hook(
                "check",
                HookEventName::PreToolUse,
                Some("Bash|exec_command"),
                HookTrust::Untrusted,
                true,
            ),
            hook(
                "audit",
                HookEventName::PreToolUse,
                None,
                HookTrust::Trusted,
                false,
            ),
            hook(
                "tests",
                HookEventName::Stop,
                None,
                HookTrust::Modified,
                true,
            ),
        ],
        diagnostics: vec!["hooks.json: PreCompact hooks are not supported".to_string()],
    }
}

fn open(chat: &mut ChatWidget, events: &std::sync::mpsc::Receiver<AppEvent>) {
    assert_eq!(run(chat, events, "hooks"), [hooks_call(HooksCall::Load)]);
    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::Loaded(listed())));
}

#[test]
fn the_browser_lists_the_events_and_an_events_hooks() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "hooks"),
        [hooks_call(HooksCall::Load)]
    );
    assert!(screen(&chat, 80).contains("Reading the hooks…"));
    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::Loaded(listed())));
    insta::assert_snapshot!("events", screen(&chat, 80));

    key(&mut chat, KeyCode::Enter);
    insta::assert_snapshot!("pre_tool_use", screen(&chat, 80));

    key(&mut chat, KeyCode::Esc);
    let page = screen(&chat, 80);
    assert!(page.contains("PreToolUse"), "{page}");
    assert!(page.contains("t trust all"), "{page}");
    key(&mut chat, KeyCode::Esc);
    assert!(!screen(&chat, 80).contains("Hooks"));
}

#[test]
fn keys_turn_a_hook_on_or_off_and_trust_it_and_the_list_is_read_again() {
    let (mut chat, events) = chat(false);
    open(&mut chat, &events);
    key(&mut chat, KeyCode::Enter);

    key(&mut chat, KeyCode::Char(' '));
    assert_eq!(
        sent(&events),
        [hooks_call(HooksCall::SetEnabled {
            key: "check".to_string(),
            enabled: false
        })]
    );
    assert!(screen(&chat, 80).contains("Saving…"));
    key(&mut chat, KeyCode::Char('t'));
    assert_eq!(
        sent(&events),
        [hooks_call(HooksCall::Trust(vec!["check".to_string()]))]
    );

    // The trusted hook has nothing to trust.
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Char('t'));
    assert!(sent(&events).is_empty());
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [hooks_call(HooksCall::SetEnabled {
            key: "audit".to_string(),
            enabled: true
        })]
    );

    let mut trusted = listed();
    trusted.hooks[0].trust = HookTrust::Trusted;
    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::Loaded(trusted)));
    let page = screen(&chat, 80);
    assert!(!page.contains("Saving…"), "{page}");
    assert!(!page.contains("new, review required"), "{page}");
    assert!(page.contains("PreToolUse hooks"), "{page}");
}

#[test]
fn trusting_every_waiting_hook_asks_first() {
    let (mut chat, events) = chat(false);
    open(&mut chat, &events);
    key(&mut chat, KeyCode::Char('t'));
    let question = screen(&chat, 80);
    assert!(question.contains("Trust 2 hooks?"), "{question}");
    key(&mut chat, KeyCode::Enter);
    assert!(sent(&events).is_empty(), "cancel is first");
    assert!(screen(&chat, 80).contains("PreToolUse"));

    key(&mut chat, KeyCode::Char('t'));
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [hooks_call(HooksCall::Trust(vec![
            "check".to_string(),
            "tests".to_string()
        ]))]
    );
    assert!(screen(&chat, 80).contains("Saving…"));
}

#[test]
fn a_failure_shows_in_the_open_browser_else_in_the_transcript() {
    let (mut chat, events) = chat(false);
    run(&mut chat, &events, "hooks");
    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::Failed(
        "hooks are unavailable: the hooks plugin is off".to_string(),
    )));
    assert!(screen(&chat, 80).contains("hooks are unavailable"));
    assert!(sent(&events).is_empty());

    key(&mut chat, KeyCode::Esc);
    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::Failed(
        "hooks are unavailable: the hooks plugin is off".to_string(),
    )));
    assert_eq!(
        sent(&events),
        [Sent::Cell(
            "■ /hooks: hooks are unavailable: the hooks plugin is off".to_string()
        )]
    );
}

#[test]
fn hooks_waiting_at_start_are_one_line() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::NeedReview(2)));
    assert_eq!(
        sent(&events),
        [Sent::Cell("• 2 hooks need review: /hooks".to_string())]
    );
    assert_eq!(
        startup_warning(&HooksReply::NeedReview(1)).as_deref(),
        Some("1 hook needs review: /hooks")
    );
    assert_eq!(startup_warning(&HooksReply::Loaded(listed())), None);
}
