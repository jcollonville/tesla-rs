// SPDX-License-Identifier: MIT OR Apache-2.0

use prost::Message;

use crate::crypto::constant_time_eq;
use crate::domain::Domain;
use crate::proto::signatures::key_identity::IdentityType;
use crate::proto::signatures::signature_data::SigType;
use crate::proto::signatures::{
    AesGcmDetachedSignatureData, AesGcmResponseSignatureData, HmacPersonalizedSignatureData,
    KeyIdentity, SignatureData,
};
use crate::proto::universal_message::destination::SubDestination;
use crate::proto::universal_message::routable_message::{Payload, SubSigData};
use crate::proto::universal_message::{Destination, SessionInfoRequest};

use crate::CommandError;
pub(crate) use crate::proto::universal_message::RoutableMessage;

/// `1 << FLAG_ENCRYPT_RESPONSE`. Vehicles with 2024.38+ encrypt the response.
pub const FLAG_ENCRYPT_RESPONSE: u32 = 1 << 1;

pub(crate) fn encode(message: &RoutableMessage) -> Vec<u8> {
    message.encode_to_vec()
}

pub(crate) fn decode(
    bytes: &[u8],
    may_have_succeeded: bool,
) -> Result<RoutableMessage, CommandError> {
    RoutableMessage::decode(bytes).map_err(|_| CommandError::BadResponse { may_have_succeeded })
}

pub(crate) fn domain_of(message: &RoutableMessage) -> Result<Domain, CommandError> {
    match message
        .to_destination
        .as_ref()
        .and_then(|destination| destination.sub_destination.as_ref())
    {
        Some(SubDestination::Domain(code)) => {
            Domain::from_code(*code).ok_or(CommandError::InvalidDomain)
        }
        _ => Err(CommandError::InvalidDomain),
    }
}

pub(crate) fn domain_destination(domain: Domain) -> Destination {
    Destination {
        sub_destination: Some(SubDestination::Domain(domain.as_i32())),
    }
}

pub(crate) fn routing_destination(address: &[u8]) -> Destination {
    Destination {
        sub_destination: Some(SubDestination::RoutingAddress(address.to_vec())),
    }
}

pub(crate) fn session_info_request(
    domain: Domain,
    public_key: &[u8],
    routing: &[u8],
    uuid: &[u8],
) -> RoutableMessage {
    RoutableMessage {
        to_destination: Some(domain_destination(domain)),
        from_destination: Some(routing_destination(routing)),
        uuid: uuid.to_vec(),
        payload: Some(Payload::SessionInfoRequest(SessionInfoRequest {
            public_key: public_key.to_vec(),
            challenge: Vec::new(),
        })),
        ..RoutableMessage::default()
    }
}

pub(crate) fn command_message(
    domain: Domain,
    routing: &[u8],
    uuid: &[u8],
    payload: &[u8],
) -> RoutableMessage {
    RoutableMessage {
        to_destination: Some(domain_destination(domain)),
        from_destination: Some(routing_destination(routing)),
        uuid: uuid.to_vec(),
        flags: FLAG_ENCRYPT_RESPONSE,
        payload: Some(Payload::ProtobufMessageAsBytes(payload.to_vec())),
        ..RoutableMessage::default()
    }
}

pub(crate) fn protobuf_bytes(message: &RoutableMessage) -> Vec<u8> {
    match &message.payload {
        Some(Payload::ProtobufMessageAsBytes(bytes)) => bytes.clone(),
        _ => Vec::new(),
    }
}

pub(crate) fn session_info_parts(message: &RoutableMessage) -> Option<(&[u8], &[u8])> {
    let info = match &message.payload {
        Some(Payload::SessionInfo(bytes)) => bytes.as_slice(),
        _ => return None,
    };
    let tag = match message.sub_sig_data.as_ref() {
        Some(SubSigData::SignatureData(data)) => match data.sig_type.as_ref() {
            Some(SigType::SessionInfoTag(tag)) => tag.tag.as_slice(),
            _ => return None,
        },
        _ => return None,
    };
    Some((info, tag))
}

pub(crate) fn fault_code(message: &RoutableMessage) -> i32 {
    message
        .signed_message_status
        .as_ref()
        .map_or(0, |status| status.signed_message_fault)
}

pub(crate) fn response_gcm(message: &RoutableMessage) -> Option<&AesGcmResponseSignatureData> {
    match message.sub_sig_data.as_ref() {
        Some(SubSigData::SignatureData(data)) => match data.sig_type.as_ref() {
            Some(SigType::AesGcmResponseData(data)) => Some(data),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn from_domain_byte(message: &RoutableMessage) -> u8 {
    match message
        .from_destination
        .as_ref()
        .and_then(|destination| destination.sub_destination.as_ref())
    {
        Some(SubDestination::Domain(code)) => u8::try_from(*code).unwrap_or(0),
        _ => 0,
    }
}

pub(crate) fn request_uuid_matches(message: &RoutableMessage, uuid: &[u8]) -> bool {
    message.request_uuid.is_empty() || constant_time_eq(&message.request_uuid, uuid)
}

pub(crate) fn set_hmac_signature(
    message: &mut RoutableMessage,
    public_key: &[u8],
    epoch: &[u8],
    counter: u32,
    expires_at: u32,
    tag: &[u8],
) {
    message.sub_sig_data = Some(SubSigData::SignatureData(SignatureData {
        signer_identity: Some(signer(public_key)),
        sig_type: Some(SigType::HmacPersonalizedData(
            HmacPersonalizedSignatureData {
                epoch: epoch.to_vec(),
                counter,
                expires_at,
                tag: tag.to_vec(),
            },
        )),
    }));
}

pub(crate) struct DetachedSignature<'a> {
    pub(crate) public_key: &'a [u8],
    pub(crate) counter: u32,
    pub(crate) expires_at: u32,
    pub(crate) nonce: &'a [u8],
    pub(crate) tag: &'a [u8],
    pub(crate) prefix: &'a [u8],
    pub(crate) ciphertext: Vec<u8>,
}

pub(crate) fn set_detached_signature(message: &mut RoutableMessage, parts: &DetachedSignature<'_>) {
    message.payload = Some(Payload::ProtobufMessageAsBytes(parts.ciphertext.clone()));
    message.sub_sig_data = Some(SubSigData::SignatureData(SignatureData {
        signer_identity: Some(signer(parts.public_key)),
        sig_type: Some(SigType::AesGcmDetachedData(AesGcmDetachedSignatureData {
            nonce: parts.nonce.to_vec(),
            counter: parts.counter,
            expires_at: parts.expires_at,
            tag: parts.tag.to_vec(),
            prefix: parts.prefix.to_vec(),
        })),
    }));
}

pub(crate) fn request_id(message: &RoutableMessage) -> Option<Vec<u8>> {
    let Some(SubSigData::SignatureData(data)) = message.sub_sig_data.as_ref() else {
        return None;
    };
    let domain = domain_of(message).ok();
    let (method, tag) = match data.sig_type.as_ref()? {
        SigType::HmacPersonalizedData(data) => (8_u8, data.tag.as_slice()),
        SigType::AesGcmPersonalizedData(data) => (5, data.tag.as_slice()),
        SigType::AesGcmDetachedData(data) => (10, data.tag.as_slice()),
        _ => return None,
    };
    let mac = if domain == Some(Domain::VehicleSecurity) && method == 8 {
        tag.get(..16)?
    } else {
        tag
    };
    let mut id = Vec::with_capacity(1 + mac.len());
    id.push(method);
    id.extend_from_slice(mac);
    Some(id)
}

/// Splits a command into the authenticated prefix and the innermost field.
///
/// Infotainment commands are `Action -> vehicle action -> command`. VCSEC commands are one level
/// deep. The prefix keeps the field tags and an empty innermost body.
pub(crate) fn split_detached(
    payload: &[u8],
    depth: usize,
) -> Result<(Vec<u8>, Vec<u8>), CommandError> {
    let mut rest = payload;
    let mut fields = Vec::with_capacity(depth);
    for _ in 0..depth {
        let (tag, tag_len) = read_varint(rest)?;
        rest = rest.get(tag_len..).ok_or(CommandError::BadPayload)?;
        if tag & 7 != 2 {
            return Err(CommandError::BadPayload);
        }
        let field = tag >> 3;
        let field = u32::try_from(field).map_err(|_| CommandError::BadPayload)?;
        let (len, len_len) = read_varint(rest)?;
        rest = rest.get(len_len..).ok_or(CommandError::BadPayload)?;
        let len = usize::try_from(len).map_err(|_| CommandError::BadPayload)?;
        if rest.len() != len {
            return Err(CommandError::BadPayload);
        }
        fields.push(field);
    }
    let inner = rest.to_vec();
    let mut prefix = Vec::new();
    for field in fields.into_iter().rev() {
        prefix = wrap(field, &prefix)?;
    }
    Ok((prefix, inner))
}

fn signer(public_key: &[u8]) -> KeyIdentity {
    KeyIdentity {
        identity_type: Some(IdentityType::PublicKey(public_key.to_vec())),
    }
}

fn wrap(field: u32, payload: &[u8]) -> Result<Vec<u8>, CommandError> {
    let mut out = Vec::new();
    write_varint(&mut out, (u64::from(field) << 3) | 2);
    let len = u64::try_from(payload.len()).map_err(|_| CommandError::BadPayload)?;
    write_varint(&mut out, len);
    out.extend_from_slice(payload);
    Ok(out)
}

fn read_varint(input: &[u8]) -> Result<(u64, usize), CommandError> {
    let mut value = 0_u64;
    for (index, byte) in input.iter().copied().enumerate() {
        if index >= 10 {
            return Err(CommandError::BadPayload);
        }
        let shift = u32::try_from(index * 7).map_err(|_| CommandError::BadPayload)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(CommandError::BadPayload)
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}
