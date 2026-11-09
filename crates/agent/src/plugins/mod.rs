//! Agent plugins that need application services, so they live here rather
//! than in a plugin crate.

mod context;
mod goal;
mod memory;
mod recap;
mod skills;
mod structured_request;
mod titles;
mod view_image;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use context::{permission_mode_plugin, workspace_context_plugin};
pub(crate) use goal::{goal_plugin, GoalService, GOAL_PLUGIN};
pub use goal::{AgentGoalBudgets, MAXIMUM_GOAL_OBJECTIVE_CHARS};
pub(crate) use memory::memory_plugin;
#[cfg(test)]
pub(crate) use memory::memory_tools;
pub use recap::AgentRecap;
pub(crate) use recap::{recap_plugin, RecapService, RECAP_PLUGIN};
pub(crate) use skills::skills_plugin;
pub(crate) use titles::{titles_plugin, TitleService, TITLES_PLUGIN};
pub(crate) use view_image::view_image_plugin;
