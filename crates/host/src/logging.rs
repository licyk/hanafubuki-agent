use hanafubuki_agent_foundation::trace_content::bounded_trace_content;
use kardia_core::{Exporter, Level, Message};

/// The prefix of a guest's logger: the plugin a host runs out of process
/// (an installed plugin's code) is named `guest:<id>`, so its fiber's
/// logger, and kardia's own messages about it, carry the prefix.
pub const GUEST_LOGGER_PREFIX: &str = "guest:";

/// Forwards kardia's runtime and plugin logs to the `log` facade under the
/// `kardia::<logger>` target, so the application's log level applies.
///
/// kardia has no `trace` level; content that AGENTS.md confines to `trace`
/// must never be written through a kardia logger, except a guest's: what a
/// logger named [`GUEST_LOGGER_PREFIX`]`…` writes (the guest's own log
/// lines and stderr, its errors) is the guest's content, forwarded only at
/// `trace` and bounded, whatever its kardia level.
#[derive(Default)]
pub struct LogExporter;

impl Exporter for LogExporter {
    fn export(&self, message: &Message) {
        if message.name.starts_with(GUEST_LOGGER_PREFIX) {
            log::trace!(
                target: "kardia",
                "[{}] {}",
                message.name,
                bounded_trace_content(&message.text)
            );
            return;
        }
        let level = match message.level {
            Level::Error => log::Level::Error,
            Level::Warn => log::Level::Warn,
            Level::Info => log::Level::Info,
            Level::Debug => log::Level::Debug,
        };
        log::log!(target: "kardia", level, "[{}] {}", message.name, message.text);
    }

    fn level(&self, _name: &str) -> Option<Level> {
        Some(Level::Debug)
    }
}

#[cfg(test)]
mod tests;
