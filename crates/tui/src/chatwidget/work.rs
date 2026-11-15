//! Update each background activity in place, including its terminal event.

use super::ChatWidget;
use crate::agent_bridge::{Warning, WarningSource, WorkSnapshot};
use crate::history_cell::{LiveCell, WorkCell};

impl ChatWidget {
    pub(super) fn on_work(&mut self, work: WorkSnapshot) {
        let completed = work.completed_at.is_some();
        let cell = match self.work_entries.get(&work.id) {
            Some(cell) if cell.cell().finished() => return,
            Some(cell) => {
                let cell = cell.clone();
                cell.update(|cell| cell.update(work.clone()));
                cell
            }
            None => {
                let cell = LiveCell::new(WorkCell::new(work.clone()));
                self.insert_live_entry(cell.clone());
                self.work_entries.insert(work.id, cell.clone());
                cell
            }
        };
        if let Some(warning) = work.warning {
            self.warnings
                .push(Warning::new(WarningSource::Compaction, warning));
        }
        if completed {
            self.complete_live_entry(cell);
        }
        self.request_redraw();
    }
}
