// SPDX-License-Identifier: MIT OR Apache-2.0

//! FlatBuffers websocket envelope used by Tesla vehicles.
//!
//! `teslamotors/fleet-telemetry` does not publish the `.fbs` schema. The layout
//! below is reconstructed from the generated Go in `messages/tesla/` at commit
//! `bb7c8a931e46da3b3eb6d20a67541c8f755f1ee6` (`FlatbuffersEnvelope.go`,
//! `FlatbuffersStream.go`, `FlatbuffersStreamAck.go`, `Message.go`, and
//! `flatbuffers_extension.go`), which was produced by the Go flatbuffers
//! compiler 23.3.3.
//!
//! Vtable slot `n` is accessor id `4 + 2n`. Strings are FlatBuffers strings
//! (`CreateByteString`: a length-prefixed byte vector plus a trailing NUL). The
//! generated readers treat them as byte vectors, so the NUL is not part of the
//! value.
//!
//! ```text
//! enum Message : byte { NONE = 0, FlatbuffersStream = 4, FlatbuffersStreamAck = 5 }
//!
//! table FlatbuffersStream {
//!   created_at:uint32;             // slot 0, unix seconds
//!   sender_id:[ubyte];             // slot 1, deprecated "type.id"
//!   payload:[ubyte];               // slot 2, protobuf body
//!   device_type:[ubyte];           // slot 3, issuer-derived
//!   device_id:[ubyte];             // slot 4, subject CN
//!   delivered_at_epoch_ms:uint64;  // slot 5, server receive time
//! }
//!
//! table FlatbuffersEnvelope {
//!   txid:[ubyte];        // slot 0
//!   topic:[ubyte];       // slot 1, "V", "alerts", "errors", "connectivity"
//!   message_type:byte;   // slot 2, the Message enum, not a flatc union tag
//!   message:offset;      // slot 3, FlatbuffersStream, or an empty string for acks
//!   message_id:[ubyte];  // slot 4
//! }
//! ```
//!
//! `FlatbuffersStreamAck` is an empty table, but `FlatbuffersStreamAckToBytes`
//! does not write it. An ack is an envelope with `message_type = 5` whose
//! `message` offset points at an empty FlatBuffers string. The reference
//! server's ack copies `txid` and `topic` and leaves `message_id` empty.
//! A parse error is a stream (`message_type = 4`) whose payload is the ASCII
//! text `incorrect message format`.

use crate::error::TelemetryError;

const MESSAGE_STREAM: u8 = 4;
const MESSAGE_ACK: u8 = 5;

/// Which envelope the vehicle or the server sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// `FlatbuffersStream` (4). Carries a protobuf payload.
    Stream,
    /// `FlatbuffersStreamAck` (5). Acknowledges a `txid`.
    Ack,
    /// A type byte this crate does not know. The reference server still acks it.
    Other(u8),
}

/// One FlatBuffers websocket binary frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// Stream, ack, or an unknown type byte.
    pub kind: MessageKind,
    /// Transaction id echoed by the ack.
    pub txid: Vec<u8>,
    /// Record topic, for example `V` or `alerts`.
    pub topic: Vec<u8>,
    /// Optional envelope message id. Acks from the reference server leave this empty.
    pub message_id: Vec<u8>,
    /// Deprecated sender id from the stream table.
    pub sender_id: Vec<u8>,
    /// Device type from the stream table.
    pub device_type: Vec<u8>,
    /// Device id from the stream table.
    pub device_id: Vec<u8>,
    /// Protobuf body for a stream, or empty for an ack.
    pub payload: Vec<u8>,
    /// Stream `created_at` as unix seconds. Zero when the field is absent.
    pub created_at: u32,
    /// Stream `delivered_at_epoch_ms`. Zero when the field is absent.
    pub delivered_at_epoch_ms: u64,
}

/// Fields written into a stream envelope.
#[derive(Clone, Copy, Debug)]
pub struct StreamParts<'a> {
    /// Transaction id.
    pub txid: &'a [u8],
    /// Topic, for example `b"V"`.
    pub topic: &'a [u8],
    /// Envelope message id. May be empty.
    pub message_id: &'a [u8],
    /// Deprecated sender id.
    pub sender_id: &'a [u8],
    /// Device type.
    pub device_type: &'a [u8],
    /// Device id.
    pub device_id: &'a [u8],
    /// Protobuf body.
    pub payload: &'a [u8],
    /// Unix seconds.
    pub created_at: u32,
    /// Receive time in milliseconds. Zero omits the field, matching flatbuffers defaults.
    pub delivered_at_epoch_ms: u64,
}

/// Decodes one websocket binary frame.
///
/// # Errors
///
/// Returns [`TelemetryError::MalformedEnvelope`] when the buffer is not a
/// FlatBuffers table with the reconstructed envelope fields.
pub fn decode_envelope(buf: &[u8]) -> Result<Envelope, TelemetryError> {
    let table = root_table(buf)?;
    let txid = optional_bytes(buf, table, 0)?;
    let topic = optional_bytes(buf, table, 1)?;
    let kind_byte = optional_u8(buf, table, 2)?;
    let message_id = optional_bytes(buf, table, 4)?;
    let kind = match kind_byte {
        MESSAGE_STREAM => MessageKind::Stream,
        MESSAGE_ACK => MessageKind::Ack,
        other => MessageKind::Other(other),
    };
    let mut envelope = Envelope {
        kind,
        txid,
        topic,
        message_id,
        sender_id: Vec::new(),
        device_type: Vec::new(),
        device_id: Vec::new(),
        payload: Vec::new(),
        created_at: 0,
        delivered_at_epoch_ms: 0,
    };
    if kind_byte == MESSAGE_STREAM {
        let Some(field) = field_position(buf, table, 3)? else {
            return Err(TelemetryError::MalformedEnvelope);
        };
        let stream = follow_offset(buf, field)?;
        envelope.created_at = optional_u32(buf, stream, 0)?;
        envelope.sender_id = optional_bytes(buf, stream, 1)?;
        envelope.payload = optional_bytes(buf, stream, 2)?;
        envelope.device_type = optional_bytes(buf, stream, 3)?;
        envelope.device_id = optional_bytes(buf, stream, 4)?;
        envelope.delivered_at_epoch_ms = optional_u64(buf, stream, 5)?;
    }
    Ok(envelope)
}

/// Encodes a stream the way `FlatbuffersStreamToBytes` does.
///
/// # Errors
///
/// Returns [`TelemetryError::MalformedEnvelope`] if a field offset does not fit
/// the FlatBuffers `u16` vtable entry. Telemetry frames are far below that.
pub fn encode_stream(parts: &StreamParts<'_>) -> Result<Vec<u8>, TelemetryError> {
    let mut builder = Builder::new();
    let sender = builder.create_string(parts.sender_id)?;
    let payload = builder.create_string(parts.payload)?;
    let device_type = builder.create_string(parts.device_type)?;
    let device_id = builder.create_string(parts.device_id)?;
    builder.start_object(6)?;
    builder.prepend_offset_slot(1, sender)?;
    builder.prepend_u32_slot(0, parts.created_at)?;
    builder.prepend_offset_slot(2, payload)?;
    builder.prepend_offset_slot(3, device_type)?;
    builder.prepend_offset_slot(4, device_id)?;
    builder.prepend_u64_slot(5, parts.delivered_at_epoch_ms)?;
    let message = builder.end_object()?;
    finish_envelope(
        &mut builder,
        parts.txid,
        parts.topic,
        parts.message_id,
        message,
        MESSAGE_STREAM,
    )
}

/// Encodes the ack the reference server returns for a parsed record.
///
/// # Errors
///
/// Returns [`TelemetryError::MalformedEnvelope`] if the builder cannot represent
/// the frame.
pub fn encode_ack(txid: &[u8], topic: &[u8]) -> Result<Vec<u8>, TelemetryError> {
    let mut builder = Builder::new();
    let empty = builder.create_string(&[])?;
    finish_envelope(&mut builder, txid, topic, &[], empty, MESSAGE_ACK)
}

/// Encodes the reference server's error frame.
///
/// The payload is `incorrect message format` and the message type is a stream,
/// not an ack.
///
/// # Errors
///
/// Returns [`TelemetryError::MalformedEnvelope`] if the builder cannot represent
/// the frame.
pub fn encode_format_error(txid: &[u8]) -> Result<Vec<u8>, TelemetryError> {
    encode_stream(&StreamParts {
        txid,
        topic: &[],
        message_id: &[],
        sender_id: &[],
        device_type: &[],
        device_id: &[],
        payload: b"incorrect message format",
        created_at: 0,
        delivered_at_epoch_ms: 0,
    })
}

fn finish_envelope(
    builder: &mut Builder,
    txid: &[u8],
    topic: &[u8],
    message_id: &[u8],
    message: u32,
    message_type: u8,
) -> Result<Vec<u8>, TelemetryError> {
    let txid = builder.create_string(txid)?;
    let topic = builder.create_string(topic)?;
    let message_id = builder.create_string(message_id)?;
    builder.start_object(5)?;
    builder.prepend_offset_slot(0, txid)?;
    builder.prepend_offset_slot(1, topic)?;
    builder.prepend_u8_slot(2, message_type)?;
    builder.prepend_offset_slot(3, message)?;
    builder.prepend_offset_slot(4, message_id)?;
    let root = builder.end_object()?;
    builder.finish(root)
}

struct Builder {
    bytes: Vec<u8>,
    head: usize,
    min_align: usize,
    vtable: Vec<u32>,
    object_end: u32,
    nested: bool,
}

impl Builder {
    fn new() -> Self {
        let initial = 1024;
        Self {
            bytes: vec![0; initial],
            head: initial,
            min_align: 1,
            vtable: Vec::new(),
            object_end: 0,
            nested: false,
        }
    }

    fn offset(&self) -> u32 {
        u32::try_from(self.bytes.len() - self.head).unwrap_or(u32::MAX)
    }

    fn create_string(&mut self, data: &[u8]) -> Result<u32, TelemetryError> {
        if self.nested {
            return Err(TelemetryError::MalformedEnvelope);
        }
        self.nested = true;
        let extra = data.len().saturating_add(1);
        self.prep(4, extra)?;
        self.place_u8(0)?;
        if self.head < data.len() {
            return Err(TelemetryError::MalformedEnvelope);
        }
        self.head -= data.len();
        self.bytes[self.head..self.head + data.len()].copy_from_slice(data);
        let len = u32::try_from(data.len()).map_err(|_| TelemetryError::MalformedEnvelope)?;
        self.place_u32(len)?;
        self.nested = false;
        Ok(self.offset())
    }

    fn start_object(&mut self, fields: usize) -> Result<(), TelemetryError> {
        if self.nested {
            return Err(TelemetryError::MalformedEnvelope);
        }
        self.nested = true;
        self.vtable.clear();
        self.vtable.resize(fields, 0);
        self.object_end = self.offset();
        Ok(())
    }

    fn prepend_offset_slot(&mut self, slot: usize, offset: u32) -> Result<(), TelemetryError> {
        if offset == 0 {
            return Ok(());
        }
        self.prepend_offset(offset)?;
        self.slot(slot);
        Ok(())
    }

    fn prepend_u8_slot(&mut self, slot: usize, value: u8) -> Result<(), TelemetryError> {
        if value == 0 {
            return Ok(());
        }
        self.prep(1, 0)?;
        self.place_u8(value)?;
        self.slot(slot);
        Ok(())
    }

    fn prepend_u32_slot(&mut self, slot: usize, value: u32) -> Result<(), TelemetryError> {
        if value == 0 {
            return Ok(());
        }
        self.prep(4, 0)?;
        self.place_u32(value)?;
        self.slot(slot);
        Ok(())
    }

    fn prepend_u64_slot(&mut self, slot: usize, value: u64) -> Result<(), TelemetryError> {
        if value == 0 {
            return Ok(());
        }
        self.prep(8, 0)?;
        self.place_u64(value)?;
        self.slot(slot);
        Ok(())
    }

    fn end_object(&mut self) -> Result<u32, TelemetryError> {
        if !self.nested {
            return Err(TelemetryError::MalformedEnvelope);
        }
        self.prep(4, 0)?;
        let placeholder = self.offset().saturating_add(4);
        self.place_i32(i32::try_from(placeholder).unwrap_or(i32::MAX))?;
        let object_offset = self.offset();
        while self.vtable.last() == Some(&0) {
            self.vtable.pop();
        }
        let slots = self.vtable.clone();
        for slot in slots.into_iter().rev() {
            let relative = if slot == 0 {
                0
            } else {
                object_offset.saturating_sub(slot)
            };
            let narrow = u16::try_from(relative).map_err(|_| TelemetryError::MalformedEnvelope)?;
            self.prep(2, 0)?;
            self.place_u16(narrow)?;
        }
        let object_size =
            u16::try_from(object_offset.saturating_sub(self.object_end)).unwrap_or(u16::MAX);
        self.prep(2, 0)?;
        self.place_u16(object_size)?;
        let vtable_bytes = u16::try_from((self.vtable.len() + 2) * 2)
            .map_err(|_| TelemetryError::MalformedEnvelope)?;
        self.prep(2, 0)?;
        self.place_u16(vtable_bytes)?;
        let vtable_offset = self.offset();
        let index = self
            .bytes
            .len()
            .checked_sub(usize::try_from(object_offset).unwrap_or(usize::MAX))
            .ok_or(TelemetryError::MalformedEnvelope)?;
        let soffset = i32::try_from(vtable_offset).unwrap_or(i32::MAX)
            - i32::try_from(object_offset).unwrap_or(i32::MAX);
        self.bytes
            .get_mut(index..index + 4)
            .ok_or(TelemetryError::MalformedEnvelope)?
            .copy_from_slice(&soffset.to_le_bytes());
        self.vtable.clear();
        self.nested = false;
        Ok(object_offset)
    }

    fn finish(&mut self, root: u32) -> Result<Vec<u8>, TelemetryError> {
        if self.nested {
            return Err(TelemetryError::MalformedEnvelope);
        }
        self.prep(self.min_align, 4)?;
        self.prepend_offset(root)?;
        Ok(self.bytes[self.head..].to_vec())
    }

    fn prepend_offset(&mut self, offset: u32) -> Result<(), TelemetryError> {
        self.prep(4, 0)?;
        if offset > self.offset() {
            return Err(TelemetryError::MalformedEnvelope);
        }
        let relative = self.offset() - offset + 4;
        self.place_u32(relative)
    }

    fn slot(&mut self, slot: usize) {
        let offset = self.offset();
        if let Some(entry) = self.vtable.get_mut(slot) {
            *entry = offset;
        }
    }

    fn prep(&mut self, size: usize, additional: usize) -> Result<(), TelemetryError> {
        if size > self.min_align {
            self.min_align = size;
        }
        let written = self
            .bytes
            .len()
            .checked_sub(self.head)
            .ok_or(TelemetryError::MalformedEnvelope)?;
        let align = written.wrapping_add(additional).wrapping_neg() & size.saturating_sub(1);
        while self.head <= align.saturating_add(size).saturating_add(additional) {
            self.grow()?;
        }
        for _ in 0..align {
            self.place_u8(0)?;
        }
        Ok(())
    }

    fn grow(&mut self) -> Result<(), TelemetryError> {
        let len = self.bytes.len();
        if len > 0x3FFF_FFFF {
            return Err(TelemetryError::MalformedEnvelope);
        }
        let new_len = len.saturating_mul(2).max(1);
        self.bytes.resize(new_len, 0);
        let middle = new_len / 2;
        self.bytes.copy_within(0..middle, middle);
        self.head = self.head.saturating_add(new_len - len);
        Ok(())
    }

    fn place_u8(&mut self, value: u8) -> Result<(), TelemetryError> {
        self.head = self
            .head
            .checked_sub(1)
            .ok_or(TelemetryError::MalformedEnvelope)?;
        self.bytes[self.head] = value;
        Ok(())
    }

    fn place_u16(&mut self, value: u16) -> Result<(), TelemetryError> {
        self.place_bytes(&value.to_le_bytes())
    }

    fn place_u32(&mut self, value: u32) -> Result<(), TelemetryError> {
        self.place_bytes(&value.to_le_bytes())
    }

    fn place_i32(&mut self, value: i32) -> Result<(), TelemetryError> {
        self.place_bytes(&value.to_le_bytes())
    }

    fn place_u64(&mut self, value: u64) -> Result<(), TelemetryError> {
        self.place_bytes(&value.to_le_bytes())
    }

    fn place_bytes(&mut self, value: &[u8]) -> Result<(), TelemetryError> {
        self.head = self
            .head
            .checked_sub(value.len())
            .ok_or(TelemetryError::MalformedEnvelope)?;
        self.bytes[self.head..self.head + value.len()].copy_from_slice(value);
        Ok(())
    }
}

fn root_table(buf: &[u8]) -> Result<usize, TelemetryError> {
    let offset = read_u32(buf, 0)? as usize;
    if offset >= buf.len() {
        return Err(TelemetryError::MalformedEnvelope);
    }
    Ok(offset)
}

fn field_position(buf: &[u8], table: usize, index: usize) -> Result<Option<usize>, TelemetryError> {
    let soffset = read_i32(buf, table)?;
    // FlatBuffers stores a backward soffset. The Go reader computes
    // `table - soffset` (`Table.Offset` in flatbuffers 23.3.3).
    let vtable = sub_offset(table, soffset)?;
    let vtable_len = read_u16(buf, vtable)? as usize;
    let entry = 4usize.saturating_add(index.saturating_mul(2));
    if entry.saturating_add(2) > vtable_len {
        return Ok(None);
    }
    let vtable_end = vtable
        .checked_add(vtable_len)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    if vtable_end > buf.len() {
        return Err(TelemetryError::MalformedEnvelope);
    }
    let relative = read_u16(buf, vtable + entry)? as usize;
    if relative == 0 {
        return Ok(None);
    }
    let position = table
        .checked_add(relative)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    if position >= buf.len() {
        return Err(TelemetryError::MalformedEnvelope);
    }
    Ok(Some(position))
}

fn optional_bytes(buf: &[u8], table: usize, index: usize) -> Result<Vec<u8>, TelemetryError> {
    let Some(position) = field_position(buf, table, index)? else {
        return Ok(Vec::new());
    };
    let start = follow_offset(buf, position)?;
    let len = read_u32(buf, start)? as usize;
    let data = start
        .checked_add(4)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    let end = data
        .checked_add(len)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    buf.get(data..end)
        .map(<[u8]>::to_vec)
        .ok_or(TelemetryError::MalformedEnvelope)
}

fn optional_u8(buf: &[u8], table: usize, index: usize) -> Result<u8, TelemetryError> {
    let Some(position) = field_position(buf, table, index)? else {
        return Ok(0);
    };
    buf.get(position)
        .copied()
        .ok_or(TelemetryError::MalformedEnvelope)
}

fn optional_u32(buf: &[u8], table: usize, index: usize) -> Result<u32, TelemetryError> {
    let Some(position) = field_position(buf, table, index)? else {
        return Ok(0);
    };
    read_u32(buf, position)
}

fn optional_u64(buf: &[u8], table: usize, index: usize) -> Result<u64, TelemetryError> {
    let Some(position) = field_position(buf, table, index)? else {
        return Ok(0);
    };
    read_u64(buf, position)
}

fn follow_offset(buf: &[u8], position: usize) -> Result<usize, TelemetryError> {
    let relative = read_u32(buf, position)? as usize;
    position
        .checked_add(relative)
        .filter(|next| *next < buf.len())
        .ok_or(TelemetryError::MalformedEnvelope)
}

fn sub_offset(position: usize, delta: i32) -> Result<usize, TelemetryError> {
    let next = i64::try_from(position).unwrap_or(i64::MAX) - i64::from(delta);
    usize::try_from(next).map_err(|_| TelemetryError::MalformedEnvelope)
}

fn read_u16(buf: &[u8], position: usize) -> Result<u16, TelemetryError> {
    let bytes = buf
        .get(position..position + 2)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    let mut out = [0; 2];
    out.copy_from_slice(bytes);
    Ok(u16::from_le_bytes(out))
}

fn read_u32(buf: &[u8], position: usize) -> Result<u32, TelemetryError> {
    let bytes = buf
        .get(position..position + 4)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    let mut out = [0; 4];
    out.copy_from_slice(bytes);
    Ok(u32::from_le_bytes(out))
}

fn read_i32(buf: &[u8], position: usize) -> Result<i32, TelemetryError> {
    let bytes = buf
        .get(position..position + 4)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    let mut out = [0; 4];
    out.copy_from_slice(bytes);
    Ok(i32::from_le_bytes(out))
}

fn read_u64(buf: &[u8], position: usize) -> Result<u64, TelemetryError> {
    let bytes = buf
        .get(position..position + 8)
        .ok_or(TelemetryError::MalformedEnvelope)?;
    let mut out = [0; 8];
    out.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(out))
}
