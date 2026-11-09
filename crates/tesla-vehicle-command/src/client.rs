// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use crate::CommandError;
use crate::commands::{self};
use crate::crypto;
use crate::domain::Domain;
use crate::key::PrivateKey;
use crate::message::{self, FLAG_ENCRYPT_RESPONSE, RoutableMessage};
use crate::proto::universal_message::routable_message::Payload;
use crate::session::{RequestId, Session};
use crate::transport::{CommandTransport, WakeStatus};

const ATTEMPTS: u32 = 3;

/// A virtual-key session with one vehicle.
///
/// Lock and unlock use the vehicle-security domain. Climate, charging, horn, and lights use
/// infotainment. Each domain is handshaken on first use. [`Self::wake`] is separate and, on the
/// Fleet API transport, calls the billable `wake_up` endpoint once.
///
/// Command expiration is 5 seconds of domain clock time unless [`Self::set_command_ttl`] changes
/// it. Internet commands use HMAC-SHA256. When the handshake says the domain supports detached
/// AES-GCM, parameters are encrypted and the command type stays in the authenticated prefix, which
/// is what the reference client does for Fleet API.
pub struct Vehicle<T> {
    transport: T,
    key: PrivateKey,
    vin: String,
    sessions: HashMap<Domain, Session>,
    connection_id: [u8; 16],
    ttl: Duration,
    max_latency: Duration,
}

impl<T> std::fmt::Debug for Vehicle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vehicle")
            .field("vin", &self.vin)
            .field("ttl_secs", &self.ttl.as_secs())
            .finish_non_exhaustive()
    }
}

impl<T: CommandTransport> Vehicle<T> {
    /// Session for `transport` using `key`.
    ///
    /// The VIN is read from the transport and checked before any network call.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidVin`] when the transport VIN is not 17 ASCII letters or
    /// digits, and [`CommandError::Entropy`] when the connection id cannot be generated.
    pub fn new(transport: T, key: PrivateKey) -> Result<Self, CommandError> {
        let vin = transport.vin().to_owned();
        crate::error::validate_vin(&vin)?;
        Ok(Self {
            transport,
            key,
            vin,
            sessions: HashMap::new(),
            connection_id: crypto::random_bytes()?,
            ttl: Duration::from_secs(5),
            max_latency: Duration::from_secs(10),
        })
    }

    /// Uncompressed public key for this virtual key.
    #[must_use]
    pub fn public_key(&self) -> &[u8; 65] {
        self.key.public_key_uncompressed()
    }

    /// How long a signed command stays valid on the domain clock.
    pub fn set_command_ttl(&mut self, ttl: Duration) {
        self.ttl = ttl;
    }

    /// Drops a handshake whose round trip exceeds `latency`.
    ///
    /// The default is 10 seconds, matching the reference client's Fleet API limit.
    pub fn set_max_latency(&mut self, latency: Duration) {
        self.max_latency = latency;
    }

    /// Wakes the vehicle through the transport.
    ///
    /// # Errors
    ///
    /// Returns the transport error. Fleet API wake is billable and is not retried here.
    pub async fn wake(&self) -> Result<WakeStatus, CommandError> {
        self.transport.wake().await
    }

    /// Handshakes `domain` and replaces any session already stored for it.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidSession`] when the session-info tag does not match, and
    /// [`CommandError::KeyNotPaired`] when the vehicle says the key is not enrolled.
    pub async fn handshake(&mut self, domain: Domain) -> Result<(), CommandError> {
        let routing = self.routing_for(domain)?;
        let uuid = crypto::random_bytes::<16>()?;
        let request = message::session_info_request(
            domain,
            self.key.public_key_uncompressed(),
            &routing,
            &uuid,
        );
        let started = Instant::now();
        let response_bytes = self.transport.exchange(&message::encode(&request)).await?;
        if started.elapsed() > self.max_latency {
            return Err(CommandError::StaleSessionInfo);
        }
        let response = message::decode(&response_bytes, false)?;
        self.accept_handshake(domain, &uuid, &response, SystemTime::now())
    }

    /// Locks the doors through VCSEC.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors. Asleep vehicles are not woken.
    pub async fn lock(&mut self) -> Result<(), CommandError> {
        self.command(Domain::VehicleSecurity, &commands::lock())
            .await
    }

    /// Unlocks the doors through VCSEC.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors. Asleep vehicles are not woken.
    pub async fn unlock(&mut self) -> Result<(), CommandError> {
        self.command(Domain::VehicleSecurity, &commands::unlock())
            .await
    }

    /// Turns climate on through infotainment.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors.
    pub async fn climate_on(&mut self) -> Result<(), CommandError> {
        self.command(Domain::Infotainment, &commands::climate(true))
            .await
    }

    /// Turns climate off through infotainment.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors.
    pub async fn climate_off(&mut self) -> Result<(), CommandError> {
        self.command(Domain::Infotainment, &commands::climate(false))
            .await
    }

    /// Sets driver and passenger setpoint temperatures in Celsius.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidTemperature`] when a value is not finite or outside 0..=50.
    pub async fn set_temperatures(
        &mut self,
        driver_celsius: f32,
        passenger_celsius: f32,
    ) -> Result<(), CommandError> {
        let payload = commands::temperatures(driver_celsius, passenger_celsius)?;
        self.command(Domain::Infotainment, &payload).await
    }

    /// Starts charging.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors.
    pub async fn charge_start(&mut self) -> Result<(), CommandError> {
        self.command(Domain::Infotainment, &commands::charge_start())
            .await
    }

    /// Stops charging.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors.
    pub async fn charge_stop(&mut self) -> Result<(), CommandError> {
        self.command(Domain::Infotainment, &commands::charge_stop())
            .await
    }

    /// Sets the charge limit percent.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidChargeLimit`] when `percent` is outside 0..=100. The vehicle
    /// may still reject a limit below its own minimum.
    pub async fn set_charge_limit(&mut self, percent: i32) -> Result<(), CommandError> {
        let payload = commands::charge_limit(percent)?;
        self.command(Domain::Infotainment, &payload).await
    }

    /// Sets the charging current in amps.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidAmps`] when `amps` is outside 1..=1000.
    pub async fn set_charging_amps(&mut self, amps: i32) -> Result<(), CommandError> {
        let payload = commands::charging_amps(amps)?;
        self.command(Domain::Infotainment, &payload).await
    }

    /// Honks the horn.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors.
    pub async fn honk(&mut self) -> Result<(), CommandError> {
        self.command(Domain::Infotainment, &commands::honk()).await
    }

    /// Flashes the lights.
    ///
    /// # Errors
    ///
    /// Returns protocol, application, or transport errors.
    pub async fn flash_lights(&mut self) -> Result<(), CommandError> {
        self.command(Domain::Infotainment, &commands::flash()).await
    }

    /// Authenticates `payload` for `domain` and returns the decrypted response bytes.
    ///
    /// Application-layer success is not checked. Use [`Self::send_routable`] for a full message.
    ///
    /// # Errors
    ///
    /// Returns protocol or transport errors. Temporary authentication faults are retried up to
    /// three times with a new counter.
    pub async fn send_bytes(
        &mut self,
        domain: Domain,
        payload: &[u8],
    ) -> Result<Vec<u8>, CommandError> {
        let message = self.round_trip(domain, payload).await?;
        Ok(message::protobuf_bytes(&message))
    }

    /// Sends any [`RoutableMessage`].
    ///
    /// A message that already has `signature_data` is forwarded unchanged. A session-info request
    /// is sent unsigned and installs the session when the response tag verifies. A protobuf
    /// payload without a signature is authenticated for its destination domain.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidDomain`] when `to_destination` is not VCSEC or infotainment.
    pub async fn send_routable(
        &mut self,
        mut message: RoutableMessage,
    ) -> Result<RoutableMessage, CommandError> {
        let domain = message::domain_of(&message)?;
        if message.uuid.len() != 16 {
            message.uuid = crypto::random_bytes::<16>()?.to_vec();
        }
        if message.from_destination.is_none() {
            let routing = self.routing_for(domain)?;
            message.from_destination = Some(message::routing_destination(&routing));
        }
        if message.sub_sig_data.is_some() {
            let uuid = message.uuid.clone();
            let request_id = message::request_id(&message).map(RequestId::from_bytes);
            let started = Instant::now();
            let response = message::decode(
                &self.transport.exchange(&message::encode(&message)).await?,
                true,
            )?;
            return self.finish(domain, &uuid, request_id, response, started);
        }
        match message.payload.clone() {
            Some(Payload::SessionInfoRequest(_)) => {
                let uuid = message.uuid.clone();
                let started = Instant::now();
                let response = message::decode(
                    &self.transport.exchange(&message::encode(&message)).await?,
                    false,
                )?;
                if started.elapsed() > self.max_latency {
                    return Err(CommandError::StaleSessionInfo);
                }
                self.accept_handshake(domain, &uuid, &response, SystemTime::now())?;
                Ok(response)
            }
            Some(Payload::ProtobufMessageAsBytes(payload)) => {
                self.round_trip(domain, &payload).await
            }
            _ => Err(CommandError::BadPayload),
        }
    }

    async fn command(&mut self, domain: Domain, payload: &[u8]) -> Result<(), CommandError> {
        let mut last = CommandError::Busy;
        for _ in 0..ATTEMPTS {
            match self.round_trip(domain, payload).await {
                Ok(message) => {
                    match commands::interpret(domain, &message::protobuf_bytes(&message)) {
                        Ok(()) => return Ok(()),
                        Err(err) if err.is_retryable() => last = err,
                        Err(err) => return Err(err),
                    }
                }
                Err(err) if err.is_retryable() => last = err,
                Err(err) => return Err(err),
            }
        }
        Err(last)
    }

    async fn round_trip(
        &mut self,
        domain: Domain,
        payload: &[u8],
    ) -> Result<RoutableMessage, CommandError> {
        let mut last = CommandError::Busy;
        for _ in 0..ATTEMPTS {
            if !self.sessions.contains_key(&domain) {
                match self.handshake(domain).await {
                    Ok(()) => {}
                    Err(err) if err.is_retryable() => {
                        last = err;
                        continue;
                    }
                    Err(err) => return Err(err),
                }
            }
            match self.send_once(domain, payload).await {
                Ok(message) => return Ok(message),
                Err(err) if err.is_retryable() => last = err,
                Err(err) => return Err(err),
            }
        }
        Err(last)
    }

    async fn send_once(
        &mut self,
        domain: Domain,
        payload: &[u8],
    ) -> Result<RoutableMessage, CommandError> {
        let routing = self.routing_for(domain)?;
        let uuid = crypto::random_bytes::<16>()?;
        let mut message = message::command_message(domain, &routing, &uuid, payload);
        message.flags |= FLAG_ENCRYPT_RESPONSE;
        let request_id = self.authorize(domain, &mut message)?;
        let started = Instant::now();
        let response_bytes = self.transport.exchange(&message::encode(&message)).await?;
        let response = message::decode(&response_bytes, true)?;
        self.finish(domain, &uuid, Some(request_id), response, started)
    }

    fn authorize(
        &mut self,
        domain: Domain,
        message: &mut RoutableMessage,
    ) -> Result<RequestId, CommandError> {
        let public = *self.key.public_key_uncompressed();
        let ttl = self.ttl;
        let vin = self.vin.clone();
        let session = self
            .sessions
            .get_mut(&domain)
            .ok_or(CommandError::NoSession)?;
        session.authorize(
            message,
            &crate::session::SignRequest {
                vin: &vin,
                local_public: &public,
                domain,
                now: SystemTime::now(),
                ttl,
                nonce: None,
            },
        )
    }

    fn accept_handshake(
        &mut self,
        domain: Domain,
        uuid: &[u8],
        response: &RoutableMessage,
        now: SystemTime,
    ) -> Result<(), CommandError> {
        if !message::request_uuid_matches(response, uuid) {
            return Err(CommandError::InvalidSession);
        }
        let code = message::fault_code(response);
        if code != 0 {
            return Err(CommandError::fault(code));
        }
        let (info, tag) =
            message::session_info_parts(response).ok_or(CommandError::InvalidSession)?;
        let session = Session::authenticate(&self.key, &self.vin, uuid, info, tag, now)?;
        self.sessions.insert(domain, session);
        Ok(())
    }

    fn finish(
        &mut self,
        domain: Domain,
        uuid: &[u8],
        request_id: Option<RequestId>,
        mut response: RoutableMessage,
        started: Instant,
    ) -> Result<RoutableMessage, CommandError> {
        if !message::request_uuid_matches(&response, uuid) {
            return Err(CommandError::BadResponse {
                may_have_succeeded: true,
            });
        }
        if let Some((info, tag)) = message::session_info_parts(&response) {
            if started.elapsed() <= self.max_latency {
                self.apply_update(domain, uuid, info, tag);
            }
            let code = message::fault_code(&response);
            if code != 0 {
                return Err(CommandError::fault(code));
            }
            return Err(CommandError::BadResponse {
                may_have_succeeded: true,
            });
        }
        if message::response_gcm(&response).is_some() {
            let Some(request_id) = request_id else {
                return Err(CommandError::Decrypt);
            };
            let session = self
                .sessions
                .get_mut(&domain)
                .ok_or(CommandError::NoSession)?;
            session.open_response(&self.vin, &mut response, &request_id)?;
        }
        let code = message::fault_code(&response);
        if code != 0 {
            return Err(CommandError::fault(code));
        }
        Ok(response)
    }

    fn apply_update(&mut self, domain: Domain, uuid: &[u8], info: &[u8], tag: &[u8]) {
        let result = self
            .sessions
            .get_mut(&domain)
            .map(|session| session.update(&self.vin, uuid, info, tag, SystemTime::now()));
        if matches!(result, Some(Err(CommandError::RemoteKeyChanged))) {
            self.sessions.remove(&domain);
        }
    }

    fn routing_for(&self, domain: Domain) -> Result<[u8; 16], CommandError> {
        match domain {
            Domain::Infotainment => Ok(self.connection_id),
            Domain::VehicleSecurity => crypto::random_bytes(),
        }
    }

    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }
}
