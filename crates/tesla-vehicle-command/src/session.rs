// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::{Duration, SystemTime};

use prost::Message;

use crate::CommandError;
use crate::crypto::{self, SessionKey, constant_time_eq};
use crate::domain::Domain;
use crate::key::PrivateKey;
use crate::message::{self, RoutableMessage};
use crate::metadata::Metadata;
use crate::proto::signatures::SessionInfo;
use crate::replay::ReplayWindow;

const EPOCH_SECONDS_LIMIT: u32 = 1 << 30;
const EPOCH_LEN: usize = 16;

pub(crate) struct SignRequest<'a> {
    pub(crate) vin: &'a str,
    pub(crate) local_public: &'a [u8],
    pub(crate) domain: Domain,
    pub(crate) now: SystemTime,
    pub(crate) ttl: Duration,
    pub(crate) nonce: Option<&'a [u8; 12]>,
}

pub(crate) struct RequestId(Vec<u8>);

impl RequestId {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

struct ResponseGuard {
    id: Vec<u8>,
    window: ReplayWindow,
}

pub(crate) struct Session {
    key: SessionKey,
    remote_public: [u8; crypto::PUBLIC_KEY_LEN],
    epoch: [u8; EPOCH_LEN],
    counter: u32,
    clock_time: u32,
    observed_at: SystemTime,
    supports_detached: bool,
    responses: Option<ResponseGuard>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("counter", &self.counter)
            .field("clock_time", &self.clock_time)
            .field("supports_detached", &self.supports_detached)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub(crate) fn authenticate(
        local: &PrivateKey,
        vin: &str,
        challenge: &[u8],
        encoded_info: &[u8],
        tag: &[u8],
        now: SystemTime,
    ) -> Result<Self, CommandError> {
        let info = decode_info(encoded_info)?;
        let remote = public_key_bytes(&info)?;
        let secret = local.secret_key()?;
        let shared = crypto::shared_secret(&secret, &remote)?;
        let expected =
            crypto::session_info_tag(shared.expose(), vin.as_bytes(), challenge, encoded_info)?;
        if !constant_time_eq(&expected, tag) {
            return Err(CommandError::InvalidSession);
        }
        check_status(&info)?;
        let mut epoch = [0_u8; EPOCH_LEN];
        epoch.copy_from_slice(&info.epoch);
        Ok(Self {
            key: shared,
            remote_public: remote,
            epoch,
            counter: info.counter,
            clock_time: info.clock_time,
            observed_at: now,
            supports_detached: supports_detached(info.flags),
            responses: None,
        })
    }

    /// Applies a later authenticated session info message.
    ///
    /// A same-epoch clock that moves backwards is ignored. A new epoch adopts the vehicle's
    /// counter even when it is lower. The reference client never lowers the counter; this follows
    /// the protocol note that a new epoch starts a new counter space.
    pub(crate) fn update(
        &mut self,
        vin: &str,
        challenge: &[u8],
        encoded_info: &[u8],
        tag: &[u8],
        now: SystemTime,
    ) -> Result<(), CommandError> {
        let expected =
            crypto::session_info_tag(self.key.expose(), vin.as_bytes(), challenge, encoded_info)?;
        if !constant_time_eq(&expected, tag) {
            return Err(CommandError::InvalidSession);
        }
        let info = decode_info(encoded_info)?;
        let remote = public_key_bytes(&info)?;
        if !constant_time_eq(&remote, &self.remote_public) {
            return Err(CommandError::RemoteKeyChanged);
        }
        check_status(&info)?;
        let epoch_changed = info.epoch.as_slice() != self.epoch.as_slice();
        if !epoch_changed && info.clock_time < self.clock_time {
            return Ok(());
        }
        if epoch_changed {
            self.epoch.copy_from_slice(&info.epoch);
            self.counter = info.counter;
        } else if info.counter > self.counter {
            self.counter = info.counter;
        }
        self.clock_time = info.clock_time;
        self.observed_at = now;
        self.supports_detached = supports_detached(info.flags);
        Ok(())
    }

    pub(crate) fn authorize(
        &mut self,
        message: &mut RoutableMessage,
        request: &SignRequest<'_>,
    ) -> Result<RequestId, CommandError> {
        if self.supports_detached {
            self.authorize_detached(message, request)
        } else {
            self.authorize_hmac(message, request)
        }
    }

    pub(crate) fn open_response(
        &mut self,
        vin: &str,
        message: &mut RoutableMessage,
        request_id: &RequestId,
    ) -> Result<(), CommandError> {
        let Some(gcm) = message::response_gcm(message) else {
            return Ok(());
        };
        let nonce = gcm.nonce.clone();
        let tag = gcm.tag.clone();
        let counter = gcm.counter;
        let ciphertext = message::protobuf_bytes(message);
        let aad = response_aad(vin, message, counter, &request_id.0)?;
        let plaintext = crypto::open(self.key.expose(), &nonce, &ciphertext, &tag, &aad)?;
        self.observe_response(&request_id.0, counter)?;
        message.payload = Some(
            crate::proto::universal_message::routable_message::Payload::ProtobufMessageAsBytes(
                plaintext,
            ),
        );
        Ok(())
    }

    fn authorize_hmac(
        &mut self,
        message: &mut RoutableMessage,
        request: &SignRequest<'_>,
    ) -> Result<RequestId, CommandError> {
        let counter = self.next_counter()?;
        let expires_at = self.expires_at(request.now, request.ttl)?;
        let plaintext = message::protobuf_bytes(message);
        let mut meta = Metadata::new();
        add_common(
            &mut meta,
            &CommonMeta {
                signature_type: 8,
                domain: request.domain,
                vin: request.vin,
                epoch: &self.epoch,
                expires_at,
                counter,
                flags: message.flags,
            },
        )?;
        let body = meta.finish(&plaintext);
        let tag = crypto::command_tag(self.key.expose(), &body)?;
        message::set_hmac_signature(
            message,
            request.local_public,
            &self.epoch,
            counter,
            expires_at,
            &tag,
        );
        Ok(RequestId(
            message::request_id(message).ok_or(CommandError::Metadata)?,
        ))
    }

    fn authorize_detached(
        &mut self,
        message: &mut RoutableMessage,
        request: &SignRequest<'_>,
    ) -> Result<RequestId, CommandError> {
        let plaintext = message::protobuf_bytes(message);
        let (prefix, inner) = message::split_detached(&plaintext, request.domain.detached_depth())?;
        let counter = self.next_counter()?;
        let expires_at = self.expires_at(request.now, request.ttl)?;
        let mut meta = Metadata::new();
        add_common(
            &mut meta,
            &CommonMeta {
                signature_type: 10,
                domain: request.domain,
                vin: request.vin,
                epoch: &self.epoch,
                expires_at,
                counter,
                flags: message.flags,
            },
        )?;
        meta.add(10, &prefix)?;
        let aad = crypto::sha256(&meta.finish(&[]));
        let nonce = match request.nonce {
            Some(nonce) => *nonce,
            None => crypto::random_bytes()?,
        };
        let (ciphertext, tag) = crypto::seal(self.key.expose(), &nonce, &inner, &aad)?;
        message::set_detached_signature(
            message,
            &message::DetachedSignature {
                public_key: request.local_public,
                counter,
                expires_at,
                nonce: &nonce,
                tag: &tag,
                prefix: &prefix,
                ciphertext,
            },
        );
        Ok(RequestId(
            message::request_id(message).ok_or(CommandError::Metadata)?,
        ))
    }

    fn next_counter(&mut self) -> Result<u32, CommandError> {
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or(CommandError::CounterExhausted)?;
        Ok(self.counter)
    }

    fn expires_at(&self, now: SystemTime, ttl: Duration) -> Result<u32, CommandError> {
        let ttl_secs = u32::try_from(ttl.as_secs()).map_err(|_| CommandError::TtlTooLong)?;
        let current = self.vehicle_time(now)?;
        let expires = current
            .checked_add(ttl_secs)
            .ok_or(CommandError::TtlTooLong)?;
        if expires > EPOCH_SECONDS_LIMIT {
            return Err(CommandError::TtlTooLong);
        }
        Ok(expires)
    }

    fn vehicle_time(&self, now: SystemTime) -> Result<u32, CommandError> {
        let elapsed = now
            .duration_since(self.observed_at)
            .unwrap_or(Duration::ZERO);
        let secs = u32::try_from(elapsed.as_secs()).map_err(|_| CommandError::TtlTooLong)?;
        self.clock_time
            .checked_add(secs)
            .ok_or(CommandError::TtlTooLong)
    }

    fn observe_response(&mut self, request_id: &[u8], counter: u32) -> Result<(), CommandError> {
        let same = self
            .responses
            .as_ref()
            .is_some_and(|guard| constant_time_eq(&guard.id, request_id));
        if !same {
            self.responses = Some(ResponseGuard {
                id: request_id.to_vec(),
                window: ReplayWindow::default(),
            });
        }
        let guard = self
            .responses
            .as_mut()
            .ok_or(CommandError::ReplayedResponse)?;
        if guard.window.update(counter) {
            Ok(())
        } else {
            Err(CommandError::ReplayedResponse)
        }
    }
}

struct CommonMeta<'a> {
    signature_type: u8,
    domain: Domain,
    vin: &'a str,
    epoch: &'a [u8],
    expires_at: u32,
    counter: u32,
    flags: u32,
}

fn add_common(meta: &mut Metadata, fields: &CommonMeta<'_>) -> Result<(), CommandError> {
    meta.add(0, &[fields.signature_type])?;
    meta.add(1, &[fields.domain.code()])?;
    meta.add(2, fields.vin.as_bytes())?;
    meta.add(3, fields.epoch)?;
    meta.add_u32(4, fields.expires_at)?;
    meta.add_u32(5, fields.counter)?;
    if fields.flags > 0 {
        meta.add_u32(7, fields.flags)?;
    }
    Ok(())
}

fn response_aad(
    vin: &str,
    message: &RoutableMessage,
    counter: u32,
    request_id: &[u8],
) -> Result<[u8; 32], CommandError> {
    let fault = u32::try_from(message::fault_code(message)).unwrap_or(0);
    let mut meta = Metadata::new();
    meta.add(0, &[9])?;
    meta.add(1, &[message::from_domain_byte(message)])?;
    meta.add(2, vin.as_bytes())?;
    meta.add_u32(5, counter)?;
    meta.add_u32(7, message.flags)?;
    meta.add(8, request_id)?;
    meta.add_u32(9, fault)?;
    Ok(crypto::sha256(&meta.finish(&[])))
}

fn decode_info(bytes: &[u8]) -> Result<SessionInfo, CommandError> {
    SessionInfo::decode(bytes).map_err(|_| CommandError::InvalidSession)
}

fn public_key_bytes(info: &SessionInfo) -> Result<[u8; crypto::PUBLIC_KEY_LEN], CommandError> {
    if info.public_key.len() != crypto::PUBLIC_KEY_LEN || info.public_key.first() != Some(&0x04) {
        return Err(CommandError::InvalidRemoteKey);
    }
    if info.epoch.len() != EPOCH_LEN {
        return Err(CommandError::InvalidSession);
    }
    let mut remote = [0_u8; crypto::PUBLIC_KEY_LEN];
    remote.copy_from_slice(&info.public_key);
    Ok(remote)
}

fn check_status(info: &SessionInfo) -> Result<(), CommandError> {
    match info.status {
        0 => Ok(()),
        1 => Err(CommandError::KeyNotPaired),
        _ => Err(CommandError::InvalidSession),
    }
}

fn supports_detached(flags: u32) -> bool {
    flags & 1 == 1
}
