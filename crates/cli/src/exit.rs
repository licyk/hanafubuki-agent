//! How a command ends: its exit code, and the failure it prints.

use std::process::ExitCode;

/// The turn completed, or the command did what it was asked.
pub(crate) const SUCCESS: u8 = 0;
/// The turn failed or was cancelled by the Agent, or the command could not
/// do what it was asked.
pub(crate) const FAILURE: u8 = 1;
/// The command line is not one the binary accepts.
pub(crate) const USAGE: u8 = 2;
/// The model asked a question `exec` could not put to anyone, so the turn
/// was cancelled.
pub(crate) const UNANSWERED: u8 = 3;
/// The turn was interrupted (Ctrl-C).
pub(crate) const CANCELLED: u8 = 130;

/// A command that did not succeed: what it prints, and its exit code.
#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) message: String,
    pub(crate) code: u8,
}

impl Failure {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: FAILURE,
        }
    }

    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: USAGE,
        }
    }

    pub(crate) fn report(self) -> ExitCode {
        if !self.message.is_empty() {
            eprintln!("error: {}", self.message);
        }
        ExitCode::from(self.code)
    }
}
