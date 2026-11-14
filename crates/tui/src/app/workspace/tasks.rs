use std::io::{self, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent::AgentSessionId;
use ratatui::{backend::Backend, crossterm::event::KeyEvent};

use crate::app::App;
use crate::tasks::{TaskAction, Tasks, TasksEvent};
use crate::tui::Tui;

impl App {
    pub(super) fn tasks_event<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: TasksEvent,
    ) {
        match event {
            TasksEvent::Open => {
                let Some(root) = self.main_session_id() else {
                    self.chat_widget
                        .add_error_message("Start a conversation before opening /tasks.".into());
                    return;
                };
                self.enter_alt_screen(tui, true);
                self.workspace.tasks = Some(Tasks::new(root));
                self.refresh_tasks(root);
            }
            TasksEvent::Read { root, result } => {
                let Some(tasks) = self
                    .workspace
                    .tasks
                    .as_mut()
                    .filter(|tasks| tasks.root == root)
                else {
                    return;
                };
                tasks.ingest(result);
                if std::mem::take(&mut tasks.refresh_pending) {
                    self.refresh_tasks(root);
                }
            }
            TasksEvent::Done { root, result } => {
                if let Some(tasks) = self
                    .workspace
                    .tasks
                    .as_mut()
                    .filter(|tasks| tasks.root == root)
                {
                    match result {
                        Ok(()) => self.refresh_tasks(root),
                        Err(error) => tasks.error = Some(error),
                    }
                }
            }
        }
        self.schedule_frame(Instant::now());
    }

    pub(crate) fn refresh_tasks(&mut self, root: AgentSessionId) {
        let Some(tasks) = self
            .workspace
            .tasks
            .as_mut()
            .filter(|tasks| tasks.root == root)
        else {
            return;
        };
        if tasks.loading {
            tasks.refresh_pending = true;
            return;
        }
        tasks.loading = true;
        self.bridge.read_tasks(root, self.app_event_tx.untagged());
    }

    pub(crate) fn handle_tasks_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        let Some(tasks) = self.workspace.tasks.as_mut() else {
            return false;
        };
        let root = tasks.root;
        match tasks.key(key) {
            Some(TaskAction::Leave) => {
                self.workspace.tasks = None;
                self.leave_alt_screen(tui);
            }
            Some(TaskAction::Open(child)) => {
                self.workspace.tasks = None;
                self.leave_alt_screen(tui);
                self.open_child(tui, child);
            }
            Some(action) => self
                .bridge
                .task_action(root, action, self.app_event_tx.untagged()),
            None => {}
        }
        self.schedule_frame(Instant::now());
        true
    }

    pub(crate) fn draw_tasks<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<bool> {
        let Some(tasks) = self.workspace.tasks.as_ref() else {
            return Ok(false);
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        tui.draw(u16::MAX, |frame| {
            tasks.render(frame.area(), frame.buffer_mut(), now)
        })?;
        Ok(true)
    }
}
