//! The GenerateContent REST request and the single candidate selected from its stream.
mod decode;
mod encode;
mod output;

pub(super) use decode::Decoder;
pub(super) use encode::encode;

#[cfg(test)]
mod tests;
