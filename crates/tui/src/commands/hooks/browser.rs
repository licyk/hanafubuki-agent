// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The hooks browser, one view through its pages (Codex's
//! `hooks_browser_view.rs`): the events, each with how many hooks it has,
//! how many run and how many wait to be trusted, above what the
//! configuration skipped; an event's hooks, each with its matcher,
//! command, trust, state and source; and the question before every hook
//! waiting is trusted. Every change is saved at once, and the hooks are
//! read again after it.

use hanafubuki_agent::{
    AgentHook, AgentHookHandler, AgentHooks, HookEventName, HookSource, HookTrust,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::agent_bridge::{needing_review, HooksCall, HooksReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, ListSelectionView, SelectionItem, SelectionViewParams,
};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

pub(super) struct HooksBrowser {
    tx: AppEventSender,
    /// `None` until the hooks are read.
    hooks: Option<AgentHooks>,
    page: Page,
    /// The open page's list, built from `hooks`.
    list: Option<ListSelectionView>,
    /// Under the list: a change being saved, or what failed.
    notice: Option<Line<'static>>,
    done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Events,
    Hooks(HookEventName),
    TrustAll,
}

impl HooksBrowser {
    pub(super) fn new(tx: AppEventSender) -> Self {
        Self {
            tx,
            hooks: None,
            page: Page::Events,
            list: None,
            notice: None,
            done: false,
        }
    }

    fn hooks(&self) -> &[AgentHook] {
        self.hooks
            .as_ref()
            .map_or(&[], |hooks| hooks.hooks.as_slice())
    }

    fn of_event(&self, event: HookEventName) -> impl Iterator<Item = &AgentHook> {
        self.hooks().iter().filter(move |hook| hook.event == event)
    }

    fn send(&self, call: HooksCall) {
        self.tx
            .send(AppEvent::Op(Op::Command(AgentCommand::Hooks(call))));
    }

    /// Opens `page` with its row `selected` highlighted.
    fn show(&mut self, page: Page, selected: Option<usize>) {
        self.page = page;
        self.list = self.hooks.as_ref().map(|_| {
            let params = match page {
                Page::Events => self.events_page(),
                Page::Hooks(event) => self.hooks_page(event),
                Page::TrustAll => self.trust_all_page(),
            };
            ListSelectionView::new(
                SelectionViewParams {
                    footer_note: self.notice.clone(),
                    initial_selected_idx: selected,
                    ..params
                },
                self.tx.clone(),
            )
        });
    }

    /// The same page again, over the hooks as they are now.
    fn refresh(&mut self) {
        let selected = self
            .list
            .as_ref()
            .and_then(ListSelectionView::selected_index);
        self.show(self.page, selected);
    }

    fn events_page(&self) -> SelectionViewParams {
        let items = HookEventName::ALL
            .into_iter()
            .map(|event| {
                let hooks: Vec<&AgentHook> = self.of_event(event).collect();
                let active = hooks.iter().filter(|hook| runs(hook)).count();
                let waiting = hooks.iter().filter(|hook| waits(hook)).count();
                let mut description = format!("{} installed · {active} active", hooks.len());
                if waiting > 0 {
                    description.push_str(&format!(" · {waiting} need review"));
                }
                SelectionItem {
                    name: event_label(event).to_string(),
                    description: Some(description),
                    ..Default::default()
                }
            })
            .collect();
        let mut header = vec![
            Line::from("Hooks".bold()),
            Line::from("A hook runs only once it is trusted, while it is on.".dim()),
        ];
        let waiting = self.hooks.as_ref().map_or(0, needing_review);
        if let Some(message) = review_message(waiting) {
            header.push(Line::from(message.yellow()));
        }
        let diagnostics = self
            .hooks
            .as_ref()
            .map_or(&[][..], |hooks| hooks.diagnostics.as_slice());
        if !diagnostics.is_empty() {
            header.push(Line::from("Skipped:".bold()));
            header.extend(
                diagnostics
                    .iter()
                    .map(|diagnostic| Line::from(format!("  · {diagnostic}").dim())),
            );
        }
        let hint = if waiting > 0 {
            "enter open · t trust all · esc close"
        } else {
            "enter open · esc close"
        };
        SelectionViewParams {
            header: Box::new(Paragraph::new(header).wrap(Wrap { trim: false })),
            footer_hint: Some(Line::from(hint)),
            items,
            ..Default::default()
        }
    }

    fn hooks_page(&self, event: HookEventName) -> SelectionViewParams {
        let items = self.of_event(event).map(hook_item).collect();
        let mut header = vec![
            Line::from(format!("{} hooks", event_label(event)).bold()),
            Line::from(event_description(event).dim()),
        ];
        let waiting = self.of_event(event).filter(|hook| waits(hook)).count();
        if let Some(message) = review_message(waiting) {
            header.push(Line::from(message.yellow()));
        }
        SelectionViewParams {
            header: Box::new(Paragraph::new(header).wrap(Wrap { trim: false })),
            footer_hint: Some(Line::from("space/enter on or off · t trust · esc back")),
            items,
            ..Default::default()
        }
    }

    fn trust_all_page(&self) -> SelectionViewParams {
        let keys: Vec<String> = self
            .hooks()
            .iter()
            .filter(|hook| waits(hook))
            .map(|hook| hook.key.clone())
            .collect();
        let count = keys.len();
        let tx = self.tx.clone();
        SelectionViewParams {
            title: Some(match count {
                1 => "Trust 1 hook?".to_string(),
                count => format!("Trust {count} hooks?"),
            }),
            subtitle: Some(
                "Each runs its command on its event from now on, until its definition changes."
                    .to_string(),
            ),
            items: vec![
                SelectionItem {
                    name: "Cancel".to_string(),
                    description: Some("Trust none of them".to_string()),
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Trust all".to_string(),
                    description: Some("Trust every hook that needs review".to_string()),
                    actions: vec![Box::new(move |_| {
                        tx.send(AppEvent::Op(Op::Command(AgentCommand::Hooks(
                            HooksCall::Trust(keys.clone()),
                        ))))
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// The hook highlighted on an event's page.
    fn selected_hook(&self, event: HookEventName) -> Option<AgentHook> {
        let index = self.list.as_ref()?.selected_index()?;
        self.of_event(event).nth(index).cloned()
    }

    fn saving(&mut self) {
        self.notice = Some(Line::from("Saving…".dim()));
        self.refresh();
    }

    fn handle_page_key(&mut self, key: KeyEvent) -> bool {
        let plain = key.modifiers == KeyModifiers::NONE;
        match (self.page, key.code) {
            (Page::Events, KeyCode::Esc) => self.done = true,
            (Page::Events, KeyCode::Enter) => {
                let selected = self
                    .list
                    .as_ref()
                    .and_then(ListSelectionView::selected_index);
                if let Some(event) = selected.and_then(|index| HookEventName::ALL.get(index)) {
                    self.show(Page::Hooks(*event), Some(0));
                }
            }
            (Page::Events, KeyCode::Char('t')) if plain => {
                if self.hooks().iter().any(waits) {
                    self.show(Page::TrustAll, Some(0));
                }
            }
            (Page::Hooks(event), KeyCode::Esc) => {
                let index = HookEventName::ALL.iter().position(|known| *known == event);
                self.show(Page::Events, index);
            }
            (Page::Hooks(event), KeyCode::Enter | KeyCode::Char(' ')) if plain => {
                if let Some(hook) = self.selected_hook(event) {
                    self.send(HooksCall::SetEnabled {
                        key: hook.key,
                        enabled: !hook.enabled,
                    });
                    self.saving();
                }
            }
            (Page::Hooks(event), KeyCode::Char('t')) if plain => {
                if let Some(hook) = self.selected_hook(event).filter(waits) {
                    self.send(HooksCall::Trust(vec![hook.key]));
                    self.saving();
                }
            }
            _ => return false,
        }
        true
    }
}

impl BottomPaneView for HooksBrowser {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let Some(list) = self.list.as_mut() else {
            self.done = key.code == KeyCode::Esc;
            return;
        };
        if self.page == Page::TrustAll {
            list.handle_key_event(key);
            if list.is_complete() {
                if list.take_last_selected_index() == Some(1) {
                    self.notice = Some(Line::from("Saving…".dim()));
                }
                self.show(Page::Events, None);
            }
            return;
        }
        if !self.handle_page_key(key) {
            if let Some(list) = self.list.as_mut() {
                list.handle_key_event(key);
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::Hooks(HooksReply::Loaded(hooks)) => {
                self.hooks = Some(hooks);
                self.notice = None;
            }
            CommandReply::Hooks(HooksReply::Failed(error)) => {
                self.notice = Some(Line::from(error.red()));
                if self.hooks.is_none() {
                    self.hooks = Some(AgentHooks {
                        hooks: Vec::new(),
                        diagnostics: Vec::new(),
                    });
                }
            }
            reply => return Some(reply),
        }
        self.refresh();
        None
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.done = true;
        CancellationEvent::Handled
    }
}

impl Renderable for HooksBrowser {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        match &self.list {
            Some(list) => list.render(area, buf),
            None => Paragraph::new(vec![
                Line::from("Hooks".bold()),
                Line::from("Reading the hooks…".dim()),
            ])
            .render(area, buf),
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.list
            .as_ref()
            .map_or(2, |list| list.desired_height(width))
    }
}

/// A hook: its matcher and command, then its trust, whether it is on, its
/// source and its timeout.
fn hook_item(hook: &AgentHook) -> SelectionItem {
    let AgentHookHandler::Command {
        command,
        timeout_sec,
        status_message,
    } = &hook.handler;
    let matcher = hook
        .matcher
        .as_deref()
        .filter(|matcher| !matcher.is_empty());
    let name = match matcher {
        Some(matcher) => format!("[{matcher}] {command}"),
        None => command.clone(),
    };
    let state = if hook.enabled { "on" } else { "off" };
    let source = match hook.source {
        HookSource::User => "user",
        HookSource::Project => "project",
    };
    let mut description = format!(
        "{} · {state} · {source} {} · {timeout_sec}s",
        trust_label(hook.trust),
        hook.source_path.display()
    );
    if let Some(message) = status_message {
        description.push_str(&format!(" · \"{message}\""));
    }
    SelectionItem {
        name,
        description: Some(description),
        ..Default::default()
    }
}

fn runs(hook: &AgentHook) -> bool {
    hook.enabled && hook.trust == HookTrust::Trusted
}

fn waits(hook: &AgentHook) -> bool {
    hook.trust != HookTrust::Trusted
}

fn review_message(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 hook needs review before it can run.".to_string()),
        count => Some(format!("{count} hooks need review before they can run.")),
    }
}

fn trust_label(trust: HookTrust) -> &'static str {
    match trust {
        HookTrust::Trusted => "trusted",
        HookTrust::Untrusted => "new, review required",
        HookTrust::Modified => "modified since trusted, review required",
    }
}

fn event_label(event: HookEventName) -> &'static str {
    match event {
        HookEventName::PreToolUse => "PreToolUse",
        HookEventName::PostToolUse => "PostToolUse",
        HookEventName::SessionStart => "SessionStart",
        HookEventName::UserPromptSubmit => "UserPromptSubmit",
        HookEventName::Stop => "Stop",
    }
}

fn event_description(event: HookEventName) -> &'static str {
    match event {
        HookEventName::PreToolUse => "Before a tool call's approval; a hook can deny the call.",
        HookEventName::PostToolUse => "After a tool call ran; a hook can add context.",
        HookEventName::SessionStart => "At a session's first turn; a hook can add context.",
        HookEventName::UserPromptSubmit => "When you send a message; a hook can add context.",
        HookEventName::Stop => {
            "When the model gives its final answer; a hook can keep the turn going."
        }
    }
}
