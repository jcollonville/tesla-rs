// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::CommandError;

/// Tag-length-value metadata, terminated by `0xFF` when finished.
///
/// Tags must be appended in non-decreasing order. Values are at most 255 bytes.
pub(crate) struct Metadata {
    buf: Vec<u8>,
    last: Option<u8>,
}

impl Metadata {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            last: None,
        }
    }

    pub(crate) fn add(&mut self, tag: u8, value: &[u8]) -> Result<(), CommandError> {
        if self.last.is_some_and(|last| tag < last) {
            return Err(CommandError::Metadata);
        }
        let len = u8::try_from(value.len()).map_err(|_| CommandError::Metadata)?;
        self.last = Some(tag);
        self.buf.push(tag);
        self.buf.push(len);
        self.buf.extend_from_slice(value);
        Ok(())
    }

    pub(crate) fn add_u32(&mut self, tag: u8, value: u32) -> Result<(), CommandError> {
        self.add(tag, &value.to_be_bytes())
    }

    pub(crate) fn finish(mut self, message: &[u8]) -> Vec<u8> {
        self.buf.push(0xff);
        self.buf.extend_from_slice(message);
        self.buf
    }
}
