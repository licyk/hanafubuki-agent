//! Maintenance belongs to the standalone host, which supplies these operations.
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiMaintenanceAction {
    CheckUpdates,
    Update,
    Doctor,
}

#[derive(Clone)]
pub struct TuiMaintenance(
    pub Arc<dyn Fn(TuiMaintenanceAction) -> Result<(), String> + Send + Sync>,
);

impl std::fmt::Debug for TuiMaintenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuiMaintenance")
    }
}
