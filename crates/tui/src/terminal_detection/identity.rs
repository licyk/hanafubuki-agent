// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! The terminal's device attributes, asked of the terminal itself: the
//! primary (`CSI ? c`) and secondary (`CSI > c`) replies a terminal sends
//! when queried, which over SSH are the only word on which terminal is at
//! the other end. Terminal.app's pair is recognized; everything else is not
//! it, and an incomplete or missing reply is unknown.

use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::time::{Duration, Instant};

/// How long the terminal has to answer; one that does not is left unknown.
const TIMEOUT: Duration = Duration::from_millis(250);

/// The query: the primary device attributes, then the secondary, so one
/// reply that comes back for the other is not mistaken for both.
const QUERY: &[u8] = b"\x1b[?c\x1b[>c";

/// Asks the terminal for its device attributes and returns what it said
/// until both replies are in or the wait is over; `None` when there is no
/// terminal to ask.
pub(super) fn probe() -> Option<Vec<u8>> {
    let read = Reader::open()?;
    // Asked on the controlling terminal where there is one, as crossterm
    // asks its own queries; otherwise on standard output, which the
    // interface requires to be the terminal too.
    if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
        tty.write_all(QUERY).ok()?;
        tty.flush().ok()?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout.write_all(QUERY).ok()?;
        stdout.flush().ok()?;
    }
    Some(read.reply())
}

/// The stream the terminal's reply comes on, nonblocking while it is read.
///
/// A duplicate of standard input, the stream the input thread reads later
/// and the one the terminal answers on. The duplicate shares the open file
/// description with it, so `O_NONBLOCK` is a change to both; it is made for
/// the probe and undone when the reader is dropped. Without a terminal on
/// standard input (embedded, redirected) the controlling terminal stands in.
struct Reader {
    file: File,
    original_flags: libc::c_int,
}

impl Reader {
    fn open() -> Option<Self> {
        // SAFETY: `dup` of a valid descriptor gives a new owned descriptor,
        // which `from_raw_fd` takes; a failure is reported, not a descriptor.
        let duplicated = unsafe { libc::dup(libc::STDIN_FILENO) };
        let file = if duplicated < 0 {
            OpenOptions::new().read(true).open("/dev/tty").ok()?
        } else {
            // SAFETY: as above.
            unsafe { File::from_raw_fd(duplicated) }
        };
        // SAFETY: `fcntl` on an open descriptor `file` owns; the flags are
        // read first, and put back on drop.
        let original_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if original_flags < 0
            || unsafe {
                libc::fcntl(
                    file.as_raw_fd(),
                    libc::F_SETFL,
                    original_flags | libc::O_NONBLOCK,
                )
            } < 0
        {
            return None;
        }
        Some(Self {
            file,
            original_flags,
        })
    }

    /// What the terminal answers within [`TIMEOUT`], stopping once both
    /// replies have arrived.
    fn reply(mut self) -> Vec<u8> {
        let deadline = Instant::now() + TIMEOUT;
        let mut reply = Vec::new();
        let mut chunk = [0u8; 256];
        while Instant::now() < deadline && !both_replies(&reply) {
            if !self.readable(deadline) {
                break;
            }
            match self.file.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => reply.extend_from_slice(&chunk[..count]),
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock) => continue,
                Err(_) => break,
            }
        }
        reply
    }

    /// Waits up to `deadline` for the reader to have something to read.
    fn readable(&self, deadline: Instant) -> bool {
        let mut poller = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32;
        // SAFETY: `poller` is a valid `pollfd` for the open reader's descriptor,
        // and `poll` only reads it.
        let ready = unsafe { libc::poll(&mut poller, 1, timeout) };
        ready > 0 && poller.revents & libc::POLLIN != 0
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: restoring on an open descriptor we own the flags read from
        // it in `open`.
        unsafe {
            libc::fcntl(self.file.as_raw_fd(), libc::F_SETFL, self.original_flags);
        }
    }
}

/// Whether Terminal.app's pair of replies is in `input`; `None` until both
/// a primary and a secondary reply are complete.
///
/// A pair inside a bracketed paste is what the person pasted, not what the
/// terminal said, so it is skipped; a `c` after another CSI reply is text
/// typed after it, not the reply's end.
pub(super) fn is_apple_terminal(input: &[u8]) -> Option<bool> {
    let primary = device_attributes(input, b"[?")?;
    let secondary = device_attributes(input, b"[>")?;
    Some(primary == b"1;2" && secondary == b"1;95;0")
}

fn device_attributes<'a>(input: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    let mut inside_paste = false;
    for sequence in input.split(|byte| *byte == b'\x1b').skip(1) {
        match sequence {
            [b'[', b'2', b'0', b'0', b'~', ..] => inside_paste = true,
            [b'[', b'2', b'0', b'1', b'~', ..] => inside_paste = false,
            _ if !inside_paste => {
                let Some(payload) = sequence.strip_prefix(prefix) else {
                    continue;
                };
                let Some(end) = payload
                    .iter()
                    .take(64)
                    .position(|byte| (0x40..=0x7e).contains(byte))
                else {
                    continue;
                };
                // The reply ends at `c`; another final byte is another
                // reply, and the text after it was typed.
                if payload[end] == b'c' {
                    return Some(&payload[..end]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether both a primary and a secondary reply have arrived, so the wait
/// can end early.
fn both_replies(reply: &[u8]) -> bool {
    device_attributes(reply, b"[?").is_some() && device_attributes(reply, b"[>").is_some()
}

#[cfg(test)]
mod tests;
