// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use tesla_fleet_api::FleetError;

use crate::domain::Domain;

/// Protocol fault carried in `signedMessageStatus.signed_message_fault`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageFault {
    /// Required vehicle subsystem is busy.
    Busy,
    /// Vehicle subsystem did not respond.
    Timeout,
    /// The authorizing key is disabled.
    InactiveKey,
    /// The MAC or signature did not verify.
    InvalidSignature,
    /// The anti-replay counter was rejected.
    InvalidTokenOrCounter,
    /// The key's role or the vehicle state does not allow the command.
    InsufficientPrivileges,
    /// The domain was missing or unrecognized.
    InvalidDomains,
    /// The vehicle did not recognize the command.
    InvalidCommand,
    /// The vehicle could not parse the command.
    Decoding,
    /// Internal vehicle error, often during boot.
    Internal,
    /// The command was personalized for a different VIN.
    WrongPersonalization,
    /// A command parameter was rejected.
    BadParameter,
    /// The vehicle keychain is full.
    KeychainFull,
    /// The session epoch does not match.
    IncorrectEpoch,
    /// An AES-GCM nonce was not 12 bytes.
    IvIncorrectLength,
    /// The command expiration is in the past.
    TimeExpired,
    /// The vehicle has no VIN provisioned.
    NotProvisioned,
    /// The vehicle could not hash command metadata.
    CouldNotHashMetadata,
    /// The command expiration is too far in the future.
    TtlTooLong,
    /// The owner disabled mobile access.
    RemoteAccessDisabled,
    /// A service key is not allowed to run remote service commands.
    RemoteServiceAccessDisabled,
    /// The command must be sent over Fleet API.
    RequiresAccountCredentials,
    /// A request field exceeded the transport MTU.
    RequestMtuExceeded,
    /// The vehicle received the command, but the response exceeded the MTU.
    ResponseMtuExceeded,
    /// The counter was repeated.
    RepeatedCounter,
    /// The key handle was not valid.
    InvalidKeyHandle,
    /// The vehicle requires an encrypted response.
    RequiresResponseEncryption,
    /// A fault code this crate does not name.
    Other(i32),
}

impl MessageFault {
    pub(crate) fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Busy,
            2 => Self::Timeout,
            4 => Self::InactiveKey,
            5 => Self::InvalidSignature,
            6 => Self::InvalidTokenOrCounter,
            7 => Self::InsufficientPrivileges,
            8 => Self::InvalidDomains,
            9 => Self::InvalidCommand,
            10 => Self::Decoding,
            11 => Self::Internal,
            12 => Self::WrongPersonalization,
            13 => Self::BadParameter,
            14 => Self::KeychainFull,
            15 => Self::IncorrectEpoch,
            16 => Self::IvIncorrectLength,
            17 => Self::TimeExpired,
            18 => Self::NotProvisioned,
            19 => Self::CouldNotHashMetadata,
            20 => Self::TtlTooLong,
            21 => Self::RemoteAccessDisabled,
            22 => Self::RemoteServiceAccessDisabled,
            23 => Self::RequiresAccountCredentials,
            24 => Self::RequestMtuExceeded,
            25 => Self::ResponseMtuExceeded,
            26 => Self::RepeatedCounter,
            27 => Self::InvalidKeyHandle,
            28 => Self::RequiresResponseEncryption,
            other => Self::Other(other),
        }
    }

    pub(crate) fn is_temporary(self) -> bool {
        matches!(
            self,
            Self::Busy
                | Self::Timeout
                | Self::InvalidSignature
                | Self::InvalidTokenOrCounter
                | Self::Internal
                | Self::IncorrectEpoch
                | Self::TimeExpired
                | Self::TtlTooLong
        )
    }

    pub(crate) fn may_have_succeeded(self) -> bool {
        matches!(self, Self::ResponseMtuExceeded)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::InactiveKey => "inactive key",
            Self::InvalidSignature => "invalid signature",
            Self::InvalidTokenOrCounter => "invalid token or counter",
            Self::InsufficientPrivileges => "insufficient privileges",
            Self::InvalidDomains => "invalid domain",
            Self::InvalidCommand => "invalid command",
            Self::Decoding => "decoding error",
            Self::Internal => "internal vehicle error",
            Self::WrongPersonalization => "wrong vin",
            Self::BadParameter => "bad parameter",
            Self::KeychainFull => "keychain full",
            Self::IncorrectEpoch => "incorrect epoch",
            Self::IvIncorrectLength => "incorrect nonce length",
            Self::TimeExpired => "command expired",
            Self::NotProvisioned => "vehicle not provisioned",
            Self::CouldNotHashMetadata => "metadata hash failed",
            Self::TtlTooLong => "expiration too far in the future",
            Self::RemoteAccessDisabled => "remote access disabled",
            Self::RemoteServiceAccessDisabled => "remote service access disabled",
            Self::RequiresAccountCredentials => "command requires fleet api",
            Self::RequestMtuExceeded => "request too large",
            Self::ResponseMtuExceeded => "response too large",
            Self::RepeatedCounter => "repeated counter",
            Self::InvalidKeyHandle => "invalid key handle",
            Self::RequiresResponseEncryption => "response encryption required",
            Self::Other(_) => "unrecognized fault",
        }
    }
}

impl fmt::Display for MessageFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(code) => write!(f, "unrecognized fault {code}"),
            named => f.write_str(named.as_str()),
        }
    }
}

/// Failure from key handling, a session, a command, or the transport.
///
/// Display text does not include private keys, session keys, or token material.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommandError {
    /// The operating system could not supply random bytes.
    #[error("operating system entropy source failed")]
    Entropy,
    /// The key was not a P-256 SEC1 or PKCS#8 private key.
    #[error("private key is not a P-256 SEC1 or PKCS8 key")]
    InvalidPrivateKey,
    /// The PEM is encrypted. This crate does not prompt for a passphrase.
    #[error("encrypted private keys are not supported")]
    EncryptedPrivateKey,
    /// The vehicle public key was not an uncompressed P-256 point.
    #[error("vehicle public key is not a valid uncompressed P-256 point")]
    InvalidRemoteKey,
    /// The VIN was not 17 ASCII letters or digits.
    #[error("vin must be 17 ascii letters or digits")]
    InvalidVin,
    /// The session-info HMAC did not match, or the session info was malformed.
    #[error("session info authentication failed")]
    InvalidSession,
    /// A response MAC or AES-GCM tag did not match.
    #[error("message authentication failed")]
    InvalidMac,
    /// A response counter was reused for the same request.
    #[error("response was replayed")]
    ReplayedResponse,
    /// An encrypted response could not be opened.
    #[error("response could not be decrypted")]
    Decrypt,
    /// The command expiration would exceed the protocol limit.
    #[error("command expiration is too far in the future")]
    TtlTooLong,
    /// The 32-bit session counter cannot be incremented.
    #[error("session counter is exhausted")]
    CounterExhausted,
    /// A later session info message named a different vehicle public key.
    #[error("the vehicle public key changed")]
    RemoteKeyChanged,
    /// A command was attempted before a handshake with that domain.
    #[error("no session for this domain")]
    NoSession,
    /// The handshake took longer than the configured latency budget.
    #[error("session info arrived too late to trust the vehicle clock")]
    StaleSessionInfo,
    /// The vehicle does not have this public key enrolled.
    #[error("the vehicle does not recognize this public key")]
    KeyNotPaired,
    /// A protocol fault from `signedMessageStatus`.
    #[error("vehicle protocol fault: {0}")]
    Fault(MessageFault),
    /// The vehicle authenticated the command and refused to run it.
    #[error("{domain} could not execute command: {reason}")]
    Nominal {
        /// Domain that refused the command.
        domain: Domain,
        /// Short vehicle-provided or mapped reason. Control characters are removed.
        reason: String,
    },
    /// The domain asked the client to retry.
    #[error("vehicle is busy")]
    Busy,
    /// The bytes were not a `RoutableMessage` this crate could parse.
    #[error("vehicle response could not be decoded")]
    BadResponse {
        /// The command may already have been delivered.
        may_have_succeeded: bool,
    },
    /// The message had no domain destination.
    #[error("message has no domain destination")]
    InvalidDomain,
    /// A local charge-limit check failed.
    #[error("charge limit percent must be between 0 and 100")]
    InvalidChargeLimit,
    /// A local charging-amps check failed.
    #[error("charging amps must be from 1 to 1000")]
    InvalidAmps,
    /// A temperature was not a finite Celsius value in range.
    #[error("temperature must be a finite celsius value from 0 to 50")]
    InvalidTemperature,
    /// VCSEC rejected a whitelist operation.
    #[error("whitelist operation failed ({0})")]
    Whitelist(i32),
    /// Reading or writing a private-key file failed. The message does not include key bytes.
    #[error("private key file error")]
    KeyFile,
    /// The command protobuf could not be split for detached encryption.
    #[error("command payload is not a nested protobuf message")]
    BadPayload,
    /// A metadata field was longer than 255 bytes or tags were out of order.
    #[error("command metadata is invalid")]
    Metadata,
    /// Fleet API transport failure.
    #[error(transparent)]
    Fleet(#[from] FleetError),
}

impl CommandError {
    /// Whether retrying the same command with a fresh counter might succeed.
    ///
    /// Faults that mean the vehicle may already have run the command are not retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.is_temporary() && !self.may_have_succeeded()
    }

    /// Whether the failure can be transient.
    #[must_use]
    pub fn is_temporary(&self) -> bool {
        match self {
            Self::Fault(fault) => fault.is_temporary(),
            Self::Busy | Self::InvalidSession | Self::StaleSessionInfo => true,
            _ => false,
        }
    }

    /// Whether the vehicle may already have executed the command.
    #[must_use]
    pub fn may_have_succeeded(&self) -> bool {
        match self {
            Self::Fault(fault) => fault.may_have_succeeded(),
            Self::BadResponse { may_have_succeeded } => *may_have_succeeded,
            Self::Decrypt | Self::InvalidMac => true,
            _ => false,
        }
    }

    pub(crate) fn fault(code: i32) -> Self {
        if code == 3 {
            return Self::KeyNotPaired;
        }
        Self::Fault(MessageFault::from_code(code))
    }

    pub(crate) fn nominal(domain: Domain, reason: impl Into<String>) -> Self {
        Self::Nominal {
            domain,
            reason: sanitize_reason(&reason.into()),
        }
    }
}

fn sanitize_reason(reason: &str) -> String {
    let mut out = String::new();
    for ch in reason.chars().take(180) {
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "unspecified error".to_owned()
    } else {
        out
    }
}

pub(crate) fn validate_vin(vin: &str) -> Result<(), CommandError> {
    let ok = vin.len() == 17 && vin.chars().all(|ch| ch.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(CommandError::InvalidVin)
    }
}
