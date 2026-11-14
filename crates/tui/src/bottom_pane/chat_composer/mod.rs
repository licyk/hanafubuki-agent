// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The chat composer is the bottom-pane text input state machine.
//!
//! It is responsible for:
//!
//! - Editing the input buffer (a [`TextArea`]), including the placeholder "elements" that stand
//!   for large pastes and for images (`attachments`: Ctrl+V, pasted image paths).
//! - Handling submit vs newline on Enter, and queueing on Tab while a turn runs.
//! - Shell-style history navigation (↑/↓) over this run's messages ([`ChatComposerHistory`]),
//!   and Ctrl+R's search through them and earlier sessions' (`history_search`).
//! - Ctrl+G's edit of the draft in an external editor (`external_editor`).
//! - Routing keys to the active popup (`popups`): the slash commands and the `@` file search.
//! - Turning raw key streams into explicit paste operations on platforms where terminals
//!   don't provide reliable bracketed paste (notably Windows).
//!
//! Codex's composer also offers Skill and app mentions, remote image rows and custom prompts;
//! none of those are ported.
//!
//! # Non-bracketed Paste Bursts
//!
//! On some terminals (especially on Windows), pastes arrive as a rapid sequence of
//! `KeyCode::Char` and `KeyCode::Enter` key events instead of a single paste event.
//!
//! To avoid misinterpreting these bursts as real typing (and to prevent transient UI effects like
//! shortcut overlays toggling on a pasted `?`), we feed "plain" character events into
//! [`PasteBurst`](super::paste_burst::PasteBurst), which buffers bursts and later flushes them
//! through [`ChatComposer::handle_paste`].
//!
//! The burst detector intentionally treats ASCII and non-ASCII differently:
//!
//! - ASCII: we briefly hold the first fast char (flicker suppression) until we know whether the
//!   stream is paste-like.
//! - non-ASCII: we do not hold the first char (IME input would feel dropped), but we still allow
//!   burst detection for actual paste streams.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::text::Line;

mod agents_navigation;
mod attachments;
mod external_editor;
mod history_search;
mod popups;
mod render;

use super::chat_composer_history::ChatComposerHistory;
use super::command_popup::CommandPopup;
use super::file_search_popup::FileSearchPopup;
use super::footer::reset_mode_after_activity;
use super::footer::toggle_shortcut_mode;
use super::footer::FooterMode;
use super::paste_burst::CharDecision;
use super::paste_burst::FlushResult;
use super::paste_burst::PasteBurst;
use super::textarea::TextArea;
use super::textarea::TextAreaState;
use super::InputResult;
use crate::app_event::AppEventSender;
use crate::key_hint;
use crate::key_hint::has_ctrl_or_alt;
use crate::key_hint::KeyBinding;
use crate::keymap::{self, Action};
use crate::update_notice::TuiUpdateNotice;
use crate::user_message::{MessageImage, UserMessage};
use history_search::HistorySearchSession;

/// If the pasted content exceeds this number of characters, replace it with a
/// placeholder in the UI.
const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;

pub(crate) struct ChatComposer {
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,
    active_popup: ActivePopup,
    app_event_tx: AppEventSender,
    history: ChatComposerHistory,
    quit_shortcut_expires_at: Option<Instant>,
    quit_shortcut_key: KeyBinding,
    use_shift_enter_hint: bool,
    pending_pastes: Vec<(String, String)>,
    /// The draft's images, `[Image #N]` each in the text (`attachments`).
    images: Vec<MessageImage>,
    large_paste_counters: HashMap<usize, usize>,
    has_focus: bool,
    placeholder_text: String,
    is_task_running: bool,
    /// The status line the footer shows (`/statusline`).
    status_line: Option<Line<'static>>,
    /// A newer release, on the right of the footer's row.
    update_notice: Option<TuiUpdateNotice>,
    /// Non-bracketed paste burst tracker (see `bottom_pane/paste_burst.rs`).
    paste_burst: PasteBurst,
    footer_mode: FooterMode,
    /// The `@` query the file search was last asked for.
    current_file_query: Option<String>,
    /// The `@` token whose popup Esc closed; it stays closed until the token changes.
    dismissed_file_popup_token: Option<String>,
    /// The commands turned off after they panicked.
    disabled_commands: Vec<&'static str>,
    /// In a second conversation (a side conversation, a subagent shown):
    /// the only commands it runs.
    command_scope: Option<&'static [&'static str]>,
    /// Ctrl+R's search, while it is open.
    history_search: Option<HistorySearchSession>,
    /// Ctrl+G's editor has the draft.
    external_editor_open: bool,
    /// Under WSL: a pasted Windows path is read as WSL's, and the shortcuts
    /// show Ctrl+Alt+V for pasting an image.
    wsl: bool,
}

/// Popup state – at most one can be visible at any time.
enum ActivePopup {
    None,
    Command(CommandPopup),
    File(FileSearchPopup),
}

impl ChatComposer {
    pub(crate) fn new(
        has_input_focus: bool,
        app_event_tx: AppEventSender,
        enhanced_keys_supported: bool,
        placeholder_text: String,
    ) -> Self {
        Self {
            textarea: TextArea::new(),
            textarea_state: RefCell::new(TextAreaState::default()),
            active_popup: ActivePopup::None,
            app_event_tx,
            history: ChatComposerHistory::new(),
            quit_shortcut_expires_at: None,
            quit_shortcut_key: key_hint::ctrl(KeyCode::Char('c')),
            use_shift_enter_hint: enhanced_keys_supported,
            pending_pastes: Vec::new(),
            images: Vec::new(),
            large_paste_counters: HashMap::new(),
            has_focus: has_input_focus,
            placeholder_text,
            is_task_running: false,
            status_line: None,
            update_notice: None,
            paste_burst: PasteBurst::default(),
            footer_mode: FooterMode::ComposerEmpty,
            current_file_query: None,
            dismissed_file_popup_token: None,
            disabled_commands: Vec::new(),
            command_scope: None,
            history_search: None,
            external_editor_open: false,
            wsl: crate::clipboard::paste::is_probably_wsl(),
        }
    }

    /// Returns true if the composer currently contains no user input.
    pub(crate) fn is_empty(&self) -> bool {
        self.textarea.is_empty()
    }

    /// Integrate pasted text into the composer.
    ///
    /// Acts as the only place where paste text is integrated, both for:
    ///
    /// - Real/explicit paste events surfaced by the terminal, and
    /// - Non-bracketed "paste bursts" that [`PasteBurst`](super::paste_burst::PasteBurst) buffers
    ///   and later flushes here.
    ///
    /// If the paste is larger than `LARGE_PASTE_CHAR_THRESHOLD` chars, inserts a placeholder
    /// element (expanded on submit) and stores the full text in `pending_pastes`; a paste that
    /// names one image file attaches it (`attachments`); otherwise inserts the pasted text
    /// directly into the textarea.
    ///
    /// In all cases, clears any paste-burst Enter suppression state so a real paste cannot affect
    /// the next user Enter key.
    pub fn handle_paste(&mut self, pasted: String) -> bool {
        let pasted = pasted.replace("\r\n", "\n").replace('\r', "\n");
        if self.paste_into_history_search(&pasted) {
            return true;
        }
        let char_count = pasted.chars().count();
        if char_count > LARGE_PASTE_CHAR_THRESHOLD {
            let placeholder = self.next_large_paste_placeholder(char_count);
            self.textarea.insert_element(&placeholder);
            self.pending_pastes.push((placeholder, pasted));
        } else if char_count <= 1 || !self.paste_image_path(&pasted) {
            self.textarea.insert_str(&pasted);
        }
        // Explicit paste events should not trigger Enter suppression.
        self.paste_burst.clear_after_explicit_paste();
        self.sync_popups();
        true
    }

    /// Replace the composer content, dropping pending pastes and images, with the cursor
    /// at the end.
    pub(crate) fn set_text_content(&mut self, text: String) {
        self.set_message(UserMessage::from_text(text));
    }

    /// Puts `message` on lines of its own before the draft, whose pastes
    /// and images stay, with the cursor at the end; behind a history
    /// search's match, into the draft the search gives back.
    pub(crate) fn restore_draft(&mut self, message: UserMessage) {
        self.edit_draft(|composer| {
            if composer.is_empty() {
                return composer.set_message(message);
            }
            composer.prepend_message(message);
            composer.textarea.set_cursor(composer.textarea.text().len());
            composer.sync_popups();
        });
    }

    /// Ctrl+C: the draft goes, into the history.
    pub(crate) fn clear_for_ctrl_c(&mut self) {
        if self.is_empty() {
            return;
        }
        let previous = self.draft_message();
        self.set_text_content(String::new());
        self.history.reset_navigation();
        self.history.record_local_submission(previous);
    }

    /// The composer's text with the large pastes it stands for put back.
    pub(crate) fn current_text_with_pending(&self) -> String {
        let mut text = self.textarea.text().to_string();
        for (placeholder, actual) in &self.pending_pastes {
            if let Some(start) = text.find(placeholder.as_str()) {
                text.replace_range(start..start + placeholder.len(), actual);
            }
        }
        text
    }

    pub(crate) fn flush_paste_burst_if_due(&mut self) -> bool {
        self.handle_paste_burst_flush(Instant::now())
    }

    /// Returns whether the composer is currently in any paste-burst related transient state.
    ///
    /// This includes actively buffering, having a non-empty burst buffer, or holding the first
    /// ASCII char for flicker suppression.
    pub(crate) fn is_in_paste_burst(&self) -> bool {
        self.paste_burst.is_active()
    }

    /// Show the transient "press again to quit" hint for `key`.
    ///
    /// The owner (`BottomPane`/`ChatWidget`) is responsible for scheduling a
    /// redraw after [`super::QUIT_SHORTCUT_TIMEOUT`] so the hint can disappear
    /// even when the UI is otherwise idle.
    pub(crate) fn set_has_focus(&mut self, has_focus: bool) {
        self.has_focus = has_focus;
    }

    pub fn show_quit_shortcut_hint(&mut self, key: KeyBinding, has_focus: bool) {
        self.quit_shortcut_expires_at = Instant::now()
            .checked_add(super::QUIT_SHORTCUT_TIMEOUT)
            .or_else(|| Some(Instant::now()));
        self.quit_shortcut_key = key;
        self.footer_mode = FooterMode::QuitShortcutReminder;
        self.has_focus = has_focus;
    }

    /// Clear the "press again to quit" hint immediately.
    pub fn clear_quit_shortcut_hint(&mut self, has_focus: bool) {
        self.quit_shortcut_expires_at = None;
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
        self.has_focus = has_focus;
    }

    /// Shows that a second Esc browses the prompts to edit one.
    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.footer_mode = FooterMode::EscHint;
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        if self.footer_mode == FooterMode::EscHint {
            self.footer_mode = FooterMode::ComposerEmpty;
        }
    }

    /// Whether the quit shortcut hint should currently be shown.
    ///
    /// This is time-based rather than event-based: it may become false without
    /// any additional user input, so the UI schedules a redraw when the hint
    /// expires.
    pub(crate) fn quit_shortcut_hint_visible(&self) -> bool {
        self.quit_shortcut_expires_at
            .is_some_and(|expires_at| Instant::now() < expires_at)
    }

    fn next_large_paste_placeholder(&mut self, char_count: usize) -> String {
        let base = format!("[Pasted Content {char_count} chars]");
        let next_suffix = self.large_paste_counters.entry(char_count).or_insert(0);
        *next_suffix += 1;
        if *next_suffix == 1 {
            base
        } else {
            format!("{base} #{next_suffix}")
        }
    }

    /// Handle a key event coming from the main UI.
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if self.handle_external_editor_key(&key_event) || self.handle_paste_image_key(&key_event) {
            return (InputResult::None, true);
        }
        let result = match &self.active_popup {
            _ if self.takes_history_search_key(&key_event) => {
                self.handle_history_search_key(key_event)
            }
            ActivePopup::Command(_) => self.handle_key_event_with_slash_popup(key_event),
            ActivePopup::File(_) => self.handle_key_event_with_file_popup(key_event),
            ActivePopup::None => self.handle_key_event_without_popup(key_event),
        };
        // Update (or hide/show) popup after processing the key.
        self.sync_popups();
        result
    }

    /// Handle key event when no popup is visible.
    fn handle_key_event_without_popup(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if self.handle_shortcut_overlay_key(&key_event) {
            return (InputResult::None, true);
        }
        if self.handle_agents_navigation(&key_event) {
            return (InputResult::None, false);
        }
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
        match key_event {
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            } if self.is_empty() => (InputResult::None, false),
            // History navigation – only when the composer is not empty or
            // the cursor is at the correct position, to avoid interfering
            // with normal cursor movement.
            key if keymap::matches(Action::HistoryPrevious, &key)
                || keymap::matches(Action::HistoryNext, &key) =>
            {
                if self
                    .history
                    .should_handle_navigation(self.textarea.text(), self.textarea.cursor())
                {
                    let replace_entry = if keymap::matches(Action::HistoryPrevious, &key) {
                        self.history.navigate_up()
                    } else {
                        self.history.navigate_down()
                    };
                    if let Some(entry) = replace_entry {
                        self.set_message(entry);
                        self.textarea.set_cursor(0);
                        return (InputResult::None, true);
                    }
                }
                self.handle_input_basic(key_event)
            }
            key if keymap::matches(Action::Queue, &key) => self.handle_queue_key(Instant::now()),
            key if keymap::matches(Action::Submit, &key) => self.handle_submission(Instant::now()),
            input => self.handle_input_basic(input),
        }
    }

    fn clamp_to_char_boundary(text: &str, pos: usize) -> usize {
        let mut p = pos.min(text.len());
        if p < text.len() && !text.is_char_boundary(p) {
            p = text
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= p)
                .last()
                .unwrap_or(0);
        }
        p
    }

    /// Handle non-ASCII character input (often IME) while still supporting paste-burst detection.
    ///
    /// This handler exists because non-ASCII input often comes from IMEs, where characters can
    /// legitimately arrive in short bursts that should **not** be treated as paste.
    ///
    /// The key differences from the ASCII path:
    ///
    /// - We never hold the first character (`PasteBurst::on_plain_char_no_hold`), because holding a
    ///   non-ASCII char can feel like dropped input.
    /// - If a burst is detected, we may need to retroactively remove already-inserted text before
    ///   the cursor and move it into the paste buffer (see `PasteBurst::decide_begin_buffer`).
    ///
    /// Because this path mixes "insert immediately" with "maybe retro-grab later", it must clamp
    /// the cursor to a UTF-8 char boundary before slicing `textarea.text()`.
    fn handle_non_ascii_char(&mut self, input: KeyEvent, now: Instant) -> (InputResult, bool) {
        if let KeyEvent {
            code: KeyCode::Char(ch),
            ..
        } = input
        {
            if self.paste_burst.try_append_char_if_active(ch, now) {
                return (InputResult::None, true);
            }
            // Before applying any non-ASCII input, flush any existing burst buffer (including a
            // pending first char from the ASCII path) so we don't carry that transient state
            // forward.
            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
            if let Some(decision) = self.paste_burst.on_plain_char_no_hold(now) {
                match decision {
                    CharDecision::BufferAppend => {
                        self.paste_burst.append_char_to_buffer(ch, now);
                        return (InputResult::None, true);
                    }
                    CharDecision::BeginBuffer { retro_chars } => {
                        if self.begin_burst_buffer(ch, retro_chars, now) {
                            return (InputResult::None, true);
                        }
                    }
                    // `on_plain_char_no_hold` never holds a char.
                    CharDecision::BeginBufferFromPending | CharDecision::RetainFirstChar => {}
                }
            }
        }
        if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
            self.handle_paste(pasted);
        }
        self.input_keeping_elements(input);
        (InputResult::None, true)
    }

    /// Starts buffering a burst with the text typed just before it (which
    /// was inserted as typing) and `ch`; `false` when it is not paste-like
    /// after all and `ch` is typed normally.
    fn begin_burst_buffer(&mut self, ch: char, retro_chars: u16, now: Instant) -> bool {
        let cur = self.textarea.cursor();
        let txt = self.textarea.text();
        let safe_cur = Self::clamp_to_char_boundary(txt, cur);
        let before = &txt[..safe_cur];
        let Some(grab) = self
            .paste_burst
            .decide_begin_buffer(now, before, retro_chars as usize)
        else {
            return false;
        };
        if !grab.grabbed.is_empty() {
            self.textarea.replace_range(grab.start_byte..safe_cur, "");
        }
        // Seed the paste burst buffer with everything (grabbed + new).
        self.paste_burst.append_char_to_buffer(ch, now);
        true
    }

    /// The trimmed message with its pastes put back and its images,
    /// recorded in the history; `None` when it is empty, or names no
    /// command although it looks like one.
    fn prepare_submission(&mut self) -> Option<UserMessage> {
        let text = self.current_text_with_pending().trim().to_string();
        if text.is_empty() || self.reject_unrecognized_command() {
            return None;
        }
        self.textarea.set_text_clearing_elements("");
        self.pending_pastes.clear();
        let message = UserMessage {
            text,
            images: std::mem::take(&mut self.images),
        };
        self.history.record_local_submission(message.clone());
        Some(message)
    }

    fn handle_submission(&mut self, now: Instant) -> (InputResult, bool) {
        // A command is run even once the caret left the '/name' token and
        // the popup with it.
        if let Some(result) = self.try_dispatch_slash_command() {
            return (result, true);
        }

        // If we're in a paste-like burst capture, treat Enter as part of the burst
        // and accumulate it rather than submitting or inserting immediately.
        // Do not treat as paste inside a slash-command context.
        let in_slash_context = matches!(self.active_popup, ActivePopup::Command(_))
            || self
                .textarea
                .text()
                .lines()
                .next()
                .unwrap_or("")
                .starts_with('/');
        if self.paste_burst.is_active()
            && !in_slash_context
            && self.paste_burst.append_newline_if_active(now)
        {
            return (InputResult::None, true);
        }

        // During a paste-like burst, treat Enter as a newline instead of submit.
        if !in_slash_context
            && self
                .paste_burst
                .newline_should_insert_instead_of_submit(now)
        {
            self.textarea.insert_str("\n");
            self.paste_burst.extend_window(now);
            return (InputResult::None, true);
        }

        match self.prepare_submission() {
            Some(message) => (InputResult::Submitted(message), true),
            None => (InputResult::None, true),
        }
    }

    /// Tab (Codex's `composer.queue`): while a turn runs the message is
    /// queued for after it, a command still runs at once; with nothing
    /// running it is sent as Enter sends it. Within a paste it is the
    /// paste's, and the paste goes on: flushing it here would let the
    /// paste's next Tab send what came before.
    fn handle_queue_key(&mut self, now: Instant) -> (InputResult, bool) {
        if self
            .paste_burst
            .newline_should_insert_instead_of_submit(now)
        {
            if !self.paste_burst.try_append_char_if_active('\t', now) {
                self.paste_burst.extend_window(now);
            }
            return (InputResult::None, true);
        }
        if !self.is_task_running {
            return self.handle_submission(now);
        }
        if let Some(result) = self.try_dispatch_slash_command() {
            return (result, true);
        }
        match self.prepare_submission() {
            Some(message) => (InputResult::Queued(message), true),
            None => (InputResult::None, true),
        }
    }

    /// Applies any due `PasteBurst` flush at time `now`.
    ///
    /// Converts [`PasteBurst::flush_if_due`] results into concrete textarea mutations.
    ///
    /// Callers:
    ///
    /// - UI ticks via [`ChatComposer::flush_paste_burst_if_due`], so held first-chars can render.
    /// - Input handling via [`ChatComposer::handle_input_basic`], so a due burst does not lag.
    fn handle_paste_burst_flush(&mut self, now: Instant) -> bool {
        match self.paste_burst.flush_if_due(now) {
            FlushResult::Paste(pasted) => {
                self.handle_paste(pasted);
                true
            }
            FlushResult::Typed(ch) => {
                // Mirror insert_str() behavior so popups stay in sync when a
                // pending fast char flushes as normal typed input.
                self.textarea.insert_str(ch.to_string().as_str());
                self.sync_popups();
                true
            }
            FlushResult::None => false,
        }
    }

    /// Handles keys that mutate the textarea, including paste-burst detection.
    ///
    /// Acts as the lowest-level keypath for keys that mutate the textarea. It is also where plain
    /// character streams are converted into explicit paste operations on terminals that do not
    /// reliably provide bracketed paste.
    ///
    /// Ordering is important:
    ///
    /// - Always flush any *due* paste burst first so buffered text does not lag behind unrelated
    ///   edits.
    /// - Then handle the incoming key, intercepting only "plain" (no Ctrl/Alt) char input.
    /// - For non-plain keys, flush via `flush_before_modified_input()` before applying the key;
    ///   otherwise `clear_window_after_non_char()` can leave buffered text waiting without a
    ///   timestamp to time out against.
    fn handle_input_basic(&mut self, input: KeyEvent) -> (InputResult, bool) {
        self.handle_input_basic_with_time(input, Instant::now())
    }

    fn handle_input_basic_with_time(
        &mut self,
        input: KeyEvent,
        now: Instant,
    ) -> (InputResult, bool) {
        // If we have a buffered non-bracketed paste burst and enough time has
        // elapsed since the last char, flush it before handling a new input.
        self.handle_paste_burst_flush(now);

        // If we're capturing a burst and receive Enter, accumulate it instead of inserting.
        if matches!(input.code, KeyCode::Enter)
            && self.paste_burst.is_active()
            && self.paste_burst.append_newline_if_active(now)
        {
            return (InputResult::None, true);
        }

        // Intercept plain Char inputs to optionally accumulate into a burst buffer.
        //
        // This is intentionally limited to "plain" (no Ctrl/Alt) chars so shortcuts keep their
        // normal semantics, and so we can aggressively flush/clear any burst state when non-char
        // keys are pressed.
        if let KeyEvent {
            code: KeyCode::Char(ch),
            modifiers,
            ..
        } = input
        {
            if !has_ctrl_or_alt(modifiers) {
                // Non-ASCII characters (e.g., from IMEs) can arrive in quick bursts, so avoid
                // holding the first char while still allowing burst detection for paste input.
                if !ch.is_ascii() {
                    return self.handle_non_ascii_char(input, now);
                }

                match self.paste_burst.on_plain_char(ch, now) {
                    CharDecision::BufferAppend | CharDecision::BeginBufferFromPending => {
                        self.paste_burst.append_char_to_buffer(ch, now);
                        return (InputResult::None, true);
                    }
                    CharDecision::BeginBuffer { retro_chars } => {
                        if self.begin_burst_buffer(ch, retro_chars, now) {
                            return (InputResult::None, true);
                        }
                    }
                    CharDecision::RetainFirstChar => {
                        // Keep the first fast char pending momentarily.
                        return (InputResult::None, true);
                    }
                }
            }
            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
        }

        // Flush any buffered burst before applying a non-char input (arrow keys, etc).
        //
        // `clear_window_after_non_char()` clears `last_plain_char_time`. If we cleared that while
        // `PasteBurst.buffer` is non-empty, `flush_if_due()` would no longer have a timestamp to
        // time out against, and the buffered paste could remain stuck until another plain char
        // arrives.
        if !matches!(input.code, KeyCode::Char(_) | KeyCode::Enter) {
            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
        }
        self.input_keeping_elements(input);

        // Update paste-burst heuristic for plain Char (no Ctrl/Alt) events.
        match input.code {
            KeyCode::Char(_) => {
                if has_ctrl_or_alt(input.modifiers) {
                    self.paste_burst.clear_window_after_non_char();
                }
            }
            KeyCode::Enter => {
                // Keep burst window alive (supports blank lines in paste).
            }
            _ => {
                // Other keys: clear burst window (buffer should have been flushed above if needed).
                self.paste_burst.clear_window_after_non_char();
            }
        }

        (InputResult::None, true)
    }

    /// Applies `input` to the text. Placeholders are atomic elements: when
    /// one is deleted its element disappears, and so does the paste or the
    /// image it stands for.
    fn input_keeping_elements(&mut self, input: KeyEvent) {
        let elements_before = (!self.pending_pastes.is_empty() || !self.images.is_empty())
            .then(|| self.textarea.element_payloads());
        self.textarea.input(input);
        if let Some(elements_before) = elements_before {
            self.reconcile_deleted_elements(elements_before);
        }
    }

    fn reconcile_deleted_elements(&mut self, elements_before: Vec<String>) {
        let elements_after: HashSet<String> =
            self.textarea.element_payloads().into_iter().collect();
        let removed: Vec<String> = elements_before
            .into_iter()
            .filter(|payload| !elements_after.contains(payload))
            .collect();
        self.pending_pastes
            .retain(|(placeholder, _)| !removed.contains(placeholder));
        if !removed.is_empty() && !self.images.is_empty() {
            self.drop_deleted_images(&removed);
        }
    }

    fn handle_shortcut_overlay_key(&mut self, key_event: &KeyEvent) -> bool {
        if key_event.kind != KeyEventKind::Press {
            return false;
        }

        let toggles = keymap::matches(Action::ShowShortcuts, key_event)
            && self.is_empty()
            && !self.is_in_paste_burst();

        if !toggles {
            return false;
        }

        let next = toggle_shortcut_mode(
            self.footer_mode,
            self.quit_shortcut_hint_visible(),
            self.is_empty(),
        );
        let changed = next != self.footer_mode;
        self.footer_mode = next;
        changed
    }

    pub fn set_task_running(&mut self, running: bool) {
        self.is_task_running = running;
    }

    pub(crate) fn set_status_line(&mut self, line: Option<Line<'static>>) {
        self.status_line = line;
    }

    pub(crate) fn set_update_notice(&mut self, notice: Option<TuiUpdateNotice>) {
        self.update_notice = notice;
    }

    /// Whether the terminal reports Shift+Enter apart from Enter.
    pub(crate) fn enhanced_keys_supported(&self) -> bool {
        self.use_shift_enter_hint
    }
}

#[cfg(test)]
mod tests;
