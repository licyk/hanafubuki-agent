use crate::ModelError;

const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;

/// Incremental SSE frame decoder. It accepts arbitrary byte fragmentation,
/// including CR/LF and UTF-8 code points split across HTTP body chunks.
pub(crate) struct SseDecoder {
    line_buffer: Vec<u8>,
    data_buffer: Vec<u8>,
    previous_was_carriage_return: bool,
    body_unverified: bool,
}

impl SseDecoder {
    pub(crate) fn new() -> Self {
        Self {
            line_buffer: Vec::new(),
            data_buffer: Vec::new(),
            previous_was_carriage_return: false,
            body_unverified: false,
        }
    }

    /// A decoder for a body whose content type is not `text/event-stream`.
    /// Some compatible servers send SSE under another type, so the body is
    /// decoded unless it is clearly not SSE: its first non-whitespace byte
    /// opens a JSON or markup document, which no SSE line starts with.
    pub(crate) fn for_unlabelled_body() -> Self {
        Self {
            body_unverified: true,
            ..Self::new()
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ModelError> {
        if self.body_unverified {
            self.verify_body(bytes)?;
        }
        let mut events = Vec::new();
        for byte in bytes {
            if self.previous_was_carriage_return {
                self.previous_was_carriage_return = false;
                if *byte == b'\n' {
                    continue;
                }
            }
            if matches!(*byte, b'\r' | b'\n') {
                let line = std::mem::take(&mut self.line_buffer);
                self.process_line(&line, &mut events)?;
                self.previous_was_carriage_return = *byte == b'\r';
            } else {
                self.line_buffer.push(*byte);
                if self.line_buffer.len() > MAX_SSE_EVENT_BYTES {
                    return Err(ModelError::Protocol(
                        "model SSE line exceeded the maximum event size".into(),
                    ));
                }
            }
        }
        Ok(events)
    }

    pub(crate) fn finish(&self) -> Result<(), ModelError> {
        if self.line_buffer.is_empty() && self.data_buffer.is_empty() {
            Ok(())
        } else {
            Err(ModelError::Protocol(
                "model SSE stream ended with an incomplete event".into(),
            ))
        }
    }

    fn verify_body(&mut self, bytes: &[u8]) -> Result<(), ModelError> {
        let Some(first) = bytes.iter().find(|byte| !byte.is_ascii_whitespace()) else {
            return Ok(());
        };
        if matches!(first, b'{' | b'[' | b'<') {
            return Err(ModelError::Protocol(
                "model response content type is not text/event-stream".into(),
            ));
        }
        self.body_unverified = false;
        Ok(())
    }

    fn process_line(&mut self, line: &[u8], events: &mut Vec<Vec<u8>>) -> Result<(), ModelError> {
        if line.is_empty() {
            if !self.data_buffer.is_empty() {
                self.data_buffer.pop();
                events.push(std::mem::take(&mut self.data_buffer));
            }
            return Ok(());
        }
        if line.first() == Some(&b':') {
            return Ok(());
        }

        let colon = line.iter().position(|byte| *byte == b':');
        let (field, mut value) = match colon {
            Some(index) => (&line[..index], &line[index + 1..]),
            None => (line, &[][..]),
        };
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }
        if field != b"data" {
            return Ok(());
        }
        if self
            .data_buffer
            .len()
            .saturating_add(value.len())
            .saturating_add(1)
            > MAX_SSE_EVENT_BYTES
        {
            return Err(ModelError::Protocol(
                "model SSE event exceeded the maximum event size".into(),
            ));
        }
        self.data_buffer.extend_from_slice(value);
        self.data_buffer.push(b'\n');
        Ok(())
    }
}

#[cfg(test)]
mod tests;
