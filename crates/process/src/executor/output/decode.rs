//! Decoding a stream's reads to UTF-8 by run of non-ASCII bytes.
//!
//! A Windows pipe can carry several encodings at once: PowerShell writes
//! UTF-8, a native program the ANSI code page. So no encoding is picked for
//! the stream: ASCII passes, each maximal run of non-ASCII bytes stays as it
//! is when it is UTF-8, and is decoded with the fallback code page when it is
//! not. Without a fallback, decoding only keeps scalars whole.

use std::time::{Duration, Instant};

use encoding_rs::Encoding;

/// How long a run at the end of a read waits for the read that may continue it.
pub(crate) const HOLD: Duration = Duration::from_millis(100);

pub(crate) struct RunDecoder {
    fallback: Option<&'static Encoding>,
    /// Without a fallback, an unfinished scalar; with one, the run of
    /// non-ASCII bytes the last read ended in.
    held: Vec<u8>,
    held_at: Option<Instant>,
}

impl RunDecoder {
    pub(crate) fn new(fallback: Option<&'static Encoding>) -> Self {
        Self {
            fallback,
            held: Vec::new(),
            held_at: None,
        }
    }

    /// Decodes `bytes`, read at `now`, into `out`, holding back what the next
    /// read may still change.
    pub(crate) fn push(&mut self, bytes: &[u8], now: Instant, out: &mut Vec<u8>) {
        let mut input = std::mem::take(&mut self.held);
        input.extend_from_slice(bytes);
        let ready = match self.fallback {
            None => {
                let complete = utf8_complete_len(&input);
                out.extend_from_slice(&input[..complete]);
                complete
            }
            Some(fallback) => decode_runs(&input, fallback, out),
        };
        self.held = input.split_off(ready);
        self.held_at = (!self.held.is_empty()).then_some(now);
    }

    /// Whether a held run has waited [`HOLD`] at `now`. An unfinished scalar
    /// without a fallback waits for its read, as it always has.
    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.fallback.is_some()
            && self
                .held_at
                .is_some_and(|held_at| now.saturating_duration_since(held_at) >= HOLD)
    }

    /// Decodes the held run as ended, keeping only an unfinished UTF-8 scalar
    /// at its end.
    pub(crate) fn release(&mut self, out: &mut Vec<u8>) {
        let Some(fallback) = self.fallback else {
            return;
        };
        let run = std::mem::take(&mut self.held);
        self.held_at = None;
        match std::str::from_utf8(&run) {
            Err(error) if error.error_len().is_none() => {
                let (complete, rest) = run.split_at(error.valid_up_to());
                out.extend_from_slice(complete);
                self.held = rest.to_vec();
            }
            _ => {
                decode_run(&run, None, fallback, out);
            }
        }
    }

    /// Passes whatever is still held once the stream has ended.
    pub(crate) fn finish(self, out: &mut Vec<u8>) {
        match self.fallback {
            None => out.extend_from_slice(&self.held),
            Some(fallback) => {
                decode_run(&self.held, None, fallback, out);
            }
        }
    }
}

/// Decodes every complete run of `input` into `out` and returns how much of
/// it was consumed: all but a run that reaches its end.
fn decode_runs(input: &[u8], fallback: &'static Encoding, out: &mut Vec<u8>) -> usize {
    let mut index = 0;
    loop {
        let ascii = input[index..]
            .iter()
            .position(|byte| !byte.is_ascii())
            .map_or(input.len(), |offset| index + offset);
        out.extend_from_slice(&input[index..ascii]);
        let Some(end) = input[ascii..]
            .iter()
            .position(u8::is_ascii)
            .map(|offset| ascii + offset)
        else {
            return ascii;
        };
        index = end + decode_run(&input[ascii..end], Some(input[end]), fallback, out);
    }
}

/// Decodes one run of non-ASCII bytes, ended by the ASCII byte `next` when it
/// is known, and returns how many bytes after the run it decoded: one when
/// `next` was decoded with it. Only a run that
/// is not UTF-8 can take `next`: a double-byte code page's trail byte can be
/// ASCII (GBK's 0x40–0x7E) after a lead byte that ends the run.
fn decode_run(
    run: &[u8],
    next: Option<u8>,
    fallback: &'static Encoding,
    out: &mut Vec<u8>,
) -> usize {
    if std::str::from_utf8(run).is_ok() {
        out.extend_from_slice(run);
        return 0;
    }
    if let Some(next) = next {
        let mut with_next = run.to_vec();
        with_next.push(next);
        let (text, _) = fallback.decode_without_bom_handling(&with_next);
        // Either `next` comes back as itself or as a lead byte's trail; a
        // replacement at the end means it was swallowed by an invalid
        // sequence, and it is kept as ASCII instead.
        if !text.ends_with(char::REPLACEMENT_CHARACTER) {
            out.extend_from_slice(text.as_bytes());
            return 1;
        }
    }
    let (text, _) = fallback.decode_without_bom_handling(run);
    out.extend_from_slice(text.as_bytes());
    0
}

/// Length of the longest prefix of `bytes` that does not end inside a UTF-8
/// scalar. Malformed bytes count as complete, so at most three bytes of a
/// valid but unfinished scalar are excluded.
pub(crate) fn utf8_complete_len(bytes: &[u8]) -> usize {
    let mut boundary = bytes.len().saturating_sub(char::MAX.len_utf8() - 1);
    while boundary < bytes.len() {
        match std::str::from_utf8(&bytes[boundary..]) {
            Ok(text) => return boundary + text.len(),
            Err(error) => {
                boundary += error.valid_up_to();
                match error.error_len() {
                    Some(invalid) => boundary += invalid,
                    None => return boundary,
                }
            }
        }
    }
    bytes.len()
}

/// The encoding of the Windows code page `code_page`, for those a non-UTF-8
/// program on Windows writes: the ANSI code pages and Cyrillic's OEM 866.
#[cfg(any(windows, test))]
pub(crate) fn code_page_encoding(code_page: u32) -> Option<&'static Encoding> {
    Some(match code_page {
        866 => encoding_rs::IBM866,
        874 => encoding_rs::WINDOWS_874,
        932 => encoding_rs::SHIFT_JIS,
        936 => encoding_rs::GBK,
        949 => encoding_rs::EUC_KR,
        950 => encoding_rs::BIG5,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1252 => encoding_rs::WINDOWS_1252,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        _ => return None,
    })
}

/// The fallback for pipe output (D15): the ANSI code page on Windows, which a
/// native program that ignores the console writes; none elsewhere, where
/// commands get `LANG=C.UTF-8`.
pub(crate) fn pipe_fallback() -> Option<&'static Encoding> {
    #[cfg(windows)]
    {
        // SAFETY: `GetACP` takes no arguments and only reads process state.
        code_page_encoding(unsafe { windows_sys::Win32::Globalization::GetACP() })
    }
    #[cfg(not(windows))]
    {
        None
    }
}
