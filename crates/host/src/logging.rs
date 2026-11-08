use kardia_core::{Exporter, Level, Message};

/// Forwards kardia's runtime and plugin logs to the `log` facade under the
/// `kardia::<logger>` target, so the application's log level applies.
///
/// kardia has no `trace` level; content that AGENTS.md confines to `trace`
/// must never be written through a kardia logger.
#[derive(Default)]
pub struct LogExporter;

impl Exporter for LogExporter {
    fn export(&self, message: &Message) {
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
