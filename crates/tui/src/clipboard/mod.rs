// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Copying text to the system clipboard: the native clipboard first
//! (`arboard`), the terminal's (an OSC 52 escape) when that fails, and both
//! over SSH, where the native clipboard is the remote host's.
//!
//! A terminal write has no acknowledgement, so a copy that went only to the
//! terminal is reported as unconfirmed. On Linux the process that wrote the
//! clipboard serves its content (X11, some Wayland compositors), so the
//! native clipboard is kept open for the rest of the run ([`Clipboard`]).
//!
//! Codex's copy worker, its tmux, GNU screen, Herdr and WSL routes and its
//! Markdown-to-HTML copies are not ported.
//!
//! Pasting an image (Ctrl+V) reads the clipboard (`paste`), with a WSL
//! route; a pasted image is checked and a clipboard image kept as a file
//! (`image_file`).

pub(crate) mod image_file;
pub(crate) mod paste;

use std::io::Write;

use base64::Engine;

/// The most bytes an OSC 52 copy carries; terminals reject larger ones.
const OSC52_MAX_RAW_BYTES: usize = 100_000;

/// Whether a copy reached a clipboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CopyStatus {
    /// The native clipboard holds it.
    Confirmed,
    /// The terminal was asked to copy it.
    Unconfirmed,
}

impl CopyStatus {
    /// What the interface says after copying `characters` characters.
    pub(crate) fn message(self, characters: usize) -> String {
        let noun = if characters == 1 {
            "character"
        } else {
            "characters"
        };
        match self {
            Self::Confirmed => format!("Copied {characters} {noun}"),
            Self::Unconfirmed => {
                format!("Sent {characters} {noun} to the terminal's clipboard (unconfirmed)")
            }
        }
    }
}

/// The interface's clipboard, which keeps the native one it wrote open.
#[derive(Default)]
pub(crate) struct Clipboard {
    #[cfg(target_os = "linux")]
    _lease: Option<arboard::Clipboard>,
}

impl Clipboard {
    pub(crate) fn copy(&mut self, text: &str) -> Result<CopyStatus, String> {
        copy_with(
            text,
            is_ssh_session(),
            |text| self.copy_natively(text),
            |text| write_osc52(&osc52_sequence(text)?),
        )
    }

    #[cfg(not(target_os = "android"))]
    fn copy_natively(&mut self, text: &str) -> Result<(), String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))?;
        clipboard
            .set_text(text)
            .map_err(|error| format!("failed to set clipboard text: {error}"))?;
        #[cfg(target_os = "linux")]
        {
            self._lease = Some(clipboard);
        }
        Ok(())
    }

    #[cfg(target_os = "android")]
    fn copy_natively(&mut self, _text: &str) -> Result<(), String> {
        Err("there is no native clipboard on Android".to_string())
    }
}

/// Copies natively, and through the terminal as well over SSH or when the
/// native copy fails.
fn copy_with(
    text: &str,
    ssh: bool,
    native: impl FnOnce(&str) -> Result<(), String>,
    terminal: impl Fn(&str) -> Result<(), String>,
) -> Result<CopyStatus, String> {
    if text.is_empty() {
        return Err("nothing to copy: the selection is empty".to_string());
    }
    // Native first: an X11 selection change caused by the terminal's copy
    // could otherwise clear what the native copy just set.
    let native = native(text);
    let terminal_result = ssh.then(|| terminal(text));
    match native {
        Ok(()) => Ok(CopyStatus::Confirmed),
        Err(native_error) => terminal_result
            .unwrap_or_else(|| terminal(text))
            .map(|()| CopyStatus::Unconfirmed)
            .map_err(|terminal_error| {
                format!("native clipboard: {native_error}; terminal clipboard: {terminal_error}")
            }),
    }
}

fn is_ssh_session() -> bool {
    crate::transcript_mode::is_ssh_session()
}

/// The OSC 52 escape that asks the terminal to put `text` on its clipboard.
fn osc52_sequence(text: &str) -> Result<String, String> {
    let raw_bytes = text.len();
    if raw_bytes > OSC52_MAX_RAW_BYTES {
        return Err(format!(
            "OSC 52 payload too large ({raw_bytes} bytes; max {OSC52_MAX_RAW_BYTES})"
        ));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    Ok(format!("\x1b]52;c;{encoded}\x07"))
}

/// Writes the escape to the controlling terminal, or to standard output
/// where there is none to open.
fn write_osc52(sequence: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        if let Ok(tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            if write_osc52_to_writer(tty, sequence).is_ok() {
                return Ok(());
            }
        }
    }
    write_osc52_to_writer(std::io::stdout().lock(), sequence)
}

fn write_osc52_to_writer(mut writer: impl Write, sequence: &str) -> Result<(), String> {
    writer
        .write_all(sequence.as_bytes())
        .map_err(|error| format!("failed to write OSC 52: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush OSC 52: {error}"))
}

#[cfg(test)]
mod tests;
