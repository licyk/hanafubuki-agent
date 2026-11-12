//! Answers the terminal queries a program inside a PTY waits on.
//!
//! A program (or ConPTY itself) may ask its terminal for device attributes
//! (DA1, DA2) or, before printing anything, the cursor position, and then block
//! until the reply arrives. Nothing on the other side of an Executor or
//! Instance Terminal PTY is a terminal emulator that would answer in time, so
//! the PTY reader filters these queries out of the output and writes the
//! reply back as input.

const ESC: u8 = 0x1b;
const LBRACKET: u8 = 0x5b;
const FINAL_C: u8 = 0x63;
const FINAL_N: u8 = 0x6e;
const PREFIX_GT: u8 = 0x3e;
const PREFIX_EQ: u8 = 0x3d;

const DA1_REPLY: &[u8] = b"\x1b[?1;2c";
const DA2_REPLY: &[u8] = b"\x1b[>0;276;0c";
const DSR_CPR_REPLY: &[u8] = b"\x1b[1;1R";

const HOLD_MAX: usize = 256;

#[derive(Clone, Copy)]
enum State {
    Idle,
    AfterEsc,
    InsideCsi,
}

pub struct DaFilter {
    state: State,
    hold: Vec<u8>,
    cpr_replied: bool,
    saw_output: bool,
}

impl Default for DaFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl DaFilter {
    pub fn new() -> Self {
        DaFilter {
            state: State::Idle,
            hold: Vec::with_capacity(16),
            cpr_replied: false,
            saw_output: false,
        }
    }

    pub fn process<F: FnMut(&[u8])>(&mut self, input: &[u8], out: &mut Vec<u8>, mut respond: F) {
        if matches!(self.state, State::Idle) && !input.contains(&ESC) {
            out.extend_from_slice(input);
            if !input.is_empty() {
                self.saw_output = true;
            }
            return;
        }

        for &byte in input {
            match self.state {
                State::Idle => {
                    if byte == ESC {
                        self.state = State::AfterEsc;
                        self.hold.clear();
                        self.hold.push(byte);
                    } else {
                        out.push(byte);
                    }
                }
                State::AfterEsc => {
                    if byte == LBRACKET {
                        self.state = State::InsideCsi;
                        self.hold.push(byte);
                    } else if byte == ESC {
                        out.extend_from_slice(&self.hold);
                        self.hold.clear();
                        self.hold.push(byte);
                    } else {
                        out.extend_from_slice(&self.hold);
                        out.push(byte);
                        self.hold.clear();
                        self.state = State::Idle;
                    }
                }
                State::InsideCsi => {
                    self.hold.push(byte);
                    if (0x40..=0x7e).contains(&byte) {
                        if byte == FINAL_C {
                            let middle = &self.hold[2..self.hold.len() - 1];
                            let is_response = middle.contains(&b'?') || middle.contains(&b';');
                            let prefix = middle.first().copied().unwrap_or(0);
                            if is_response {
                                out.extend_from_slice(&self.hold);
                            } else {
                                match prefix {
                                    PREFIX_GT => respond(DA2_REPLY),
                                    PREFIX_EQ => {}
                                    0 | b'0'..=b'9' => respond(DA1_REPLY),
                                    _ => out.extend_from_slice(&self.hold),
                                }
                            }
                        } else if byte == FINAL_N
                            && self.hold.len() == 4
                            && self.hold[2] == b'6'
                            && !self.cpr_replied
                            && !self.saw_output
                            && out.is_empty()
                        {
                            respond(DSR_CPR_REPLY);
                            self.cpr_replied = true;
                        } else {
                            out.extend_from_slice(&self.hold);
                        }
                        self.hold.clear();
                        self.state = State::Idle;
                    } else if self.hold.len() >= HOLD_MAX {
                        out.extend_from_slice(&self.hold);
                        self.hold.clear();
                        self.state = State::Idle;
                    }
                }
            }
        }
        if !out.is_empty() {
            self.saw_output = true;
        }
    }

    /// Pass on the bytes held for a sequence the output ended inside, at end
    /// of file.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.hold);
        self.state = State::Idle;
    }
}

#[cfg(test)]
mod tests;
