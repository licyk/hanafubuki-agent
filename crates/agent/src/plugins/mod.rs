//! Agent plugins that need application services, so they live here rather
//! than in a plugin crate.

mod context;
mod memory;
mod skills;
mod titles;
mod view_image;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use context::{permission_mode_plugin, workspace_context_plugin};
pub(crate) use memory::memory_plugin;
#[cfg(test)]
pub(crate) use memory::memory_tools;
pub(crate) use skills::skills_plugin;
pub(crate) use titles::titles_plugin;
pub(crate) use view_image::view_image_plugin;
