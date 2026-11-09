// SPDX-License-Identifier: MIT OR Apache-2.0

//! Errors from decoding and from the mutual-TLS receiver.

use std::fmt;

/// Failure while decoding telemetry or accepting a vehicle connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// The outer FlatBuffers envelope is truncated or not a Tesla stream message.
    #[error("fleet telemetry envelope is truncated or malformed")]
    MalformedEnvelope,
    /// A protobuf body could not be decoded.
    #[error("protobuf payload could not be decoded")]
    Protobuf(#[source] prost::DecodeError),
    /// A config value or server option is missing or inconsistent.
    #[error("{0}")]
    InvalidConfig(&'static str),
    /// The client certificate could not be parsed.
    #[error("client certificate could not be parsed")]
    Certificate,
    /// The certificate issuer and extensions are not a Tesla device identity.
    #[error("client certificate issuer `{issuer}` is not a known Tesla device CA")]
    UntrustedIssuer {
        /// Issuer common name.
        issuer: String,
    },
    /// Tesla lists this certificate's extended-key-usage OID as excluded.
    #[error("client certificate is excluded from device identity")]
    ExcludedCertificate,
    /// TLS setup or the handshake failed.
    #[error("tls: {0}")]
    Tls(String),
    /// The websocket handshake or a socket read failed.
    #[error("websocket: {0}")]
    Socket(String),
    /// The TCP listener could not bind.
    #[error("listener: {0}")]
    Listener(String),
}

impl TelemetryError {
    pub(crate) fn tls(err: impl fmt::Display) -> Self {
        Self::Tls(err.to_string())
    }

    pub(crate) fn socket(err: impl fmt::Display) -> Self {
        Self::Socket(err.to_string())
    }

    pub(crate) fn listener(err: impl fmt::Display) -> Self {
        Self::Listener(err.to_string())
    }
}
