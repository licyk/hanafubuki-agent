//! A command's view in the bottom pane, run inside `catch_panic`: its key
//! handling and its drawing. A view that panics is done (the pane drops
//! it), draws nothing more, and is reported to the pane's failures, from
//! which the chat turns its command off.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use hanafubuki_agent::panic::catch_panic;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;

use super::bottom_pane_view::BottomPaneView;
use super::CancellationEvent;
use crate::render::renderable::Renderable;

/// A command whose view panicked, and the panic's message.
#[derive(Debug)]
pub(crate) struct ViewFailure {
    pub(crate) command: &'static str,
    pub(crate) message: String,
}

/// Where the pane's command views report their panics; the pane reads it.
pub(super) type ViewFailures = Rc<RefCell<Vec<ViewFailure>>>;

pub(super) struct ModuleView {
    guard: Guard,
    view: Box<dyn BottomPaneView>,
}

impl ModuleView {
    pub(super) fn new(
        command: &'static str,
        view: Box<dyn BottomPaneView>,
        failures: ViewFailures,
    ) -> Self {
        Self {
            guard: Guard {
                command,
                failed: Cell::new(false),
                failures,
            },
            view,
        }
    }
}

impl BottomPaneView for ModuleView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        self.guard.run((), || self.view.handle_key_event(key_event));
    }

    fn is_complete(&self) -> bool {
        self.guard.failed.get() || self.guard.run(true, || self.view.is_complete())
    }

    /// A view that panics on Ctrl-C took the key: it is dropped.
    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.guard
            .run(CancellationEvent::Handled, || self.view.on_ctrl_c())
    }

    fn command(&self) -> Option<&'static str> {
        Some(self.guard.command)
    }
}

impl Renderable for ModuleView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.guard.run((), || self.view.render(area, buf));
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.guard.run(0, || self.view.desired_height(width))
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.guard.run(None, || self.view.cursor_pos(area))
    }
}

struct Guard {
    command: &'static str,
    failed: Cell<bool>,
    failures: ViewFailures,
}

impl Guard {
    /// `call`'s result; `fallback` once the view has panicked, in `call` or
    /// before.
    fn run<R>(&self, fallback: R, call: impl FnOnce() -> R) -> R {
        if self.failed.get() {
            return fallback;
        }
        match catch_panic(call) {
            Ok(result) => result,
            Err(message) => {
                self.failed.set(true);
                self.failures.borrow_mut().push(ViewFailure {
                    command: self.command,
                    message,
                });
                fallback
            }
        }
    }
}
