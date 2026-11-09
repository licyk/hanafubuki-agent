//! The thread that reads the terminal's input into the app loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::app_event::{AppEvent, AppEventSender};

/// How long one wait for input lasts. Between waits the reader releases
/// crossterm's event reader, so a cursor-position query (after a resize)
/// gets its answer, and sees whether it should stop.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) struct InputReader {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl InputReader {
    pub(crate) fn start(app_event_tx: AppEventSender) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("tui-input".to_string())
            .spawn(move || read_input(&app_event_tx, &stopping))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_input(app_event_tx: &AppEventSender, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match event::poll(POLL_INTERVAL) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return,
        }
        let event = match event::read() {
            Ok(event) => event,
            Err(_) => return,
        };
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                app_event_tx.send(AppEvent::Key(key))
            }
            Event::Paste(text) => app_event_tx.send(AppEvent::Paste(text)),
            Event::Mouse(mouse) => app_event_tx.send(AppEvent::Mouse(mouse)),
            Event::Resize(_, _) | Event::FocusGained => app_event_tx.send(AppEvent::Redraw),
            _ => {}
        }
    }
}
