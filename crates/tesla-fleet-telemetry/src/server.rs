// SPDX-License-Identifier: MIT OR Apache-2.0

//! Mutual-TLS websocket receiver.
//!
//! Vehicles dial the reference server with a client certificate and push binary
//! FlatBuffers frames. This module accepts that handshake, checks the client
//! certificate against a CA bundle, acks the way `server/streaming/socket.go`
//! does, and sends decoded records on a channel. It does not store records.
//!
//! The embedded CA bundles are `config/files/prod_ca.crt` and `eng_ca.crt` from
//! `teslamotors/fleet-telemetry` commit `bb7c8a931e46da3b3eb6d20a67541c8f755f1ee6`.
//! The reference server trusts the prod bundle unless `use_default_eng_ca` is set,
//! and it can append a custom PEM file. Chain validation uses `rustls` / webpki.
//! That is stricter than Go's `crypto/x509`, and this crate has not been checked
//! against a live vehicle certificate.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use prost::Message;
use rustls::RootCertStore;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::envelope::{self, MessageKind};
use crate::error::TelemetryError;
use crate::identity::Identity;
use crate::proto::vehicle_connectivity::{ConnectivityEvent, VehicleConnectivity};
use crate::record::{self, DecodeOptions, Decoded};

const MAX_FRAME_BYTES: usize = 1_000_000;
const PROD_CA: &str = include_str!("../certs/prod_ca.crt");
const ENG_CA: &str = include_str!("../certs/eng_ca.crt");

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// Which client CA bundle the listener requires.
#[derive(Clone, Debug)]
pub enum ClientCa {
    /// Production Tesla product CAs from the reference server.
    TeslaProd,
    /// Engineering Tesla product CAs from the reference server.
    TeslaEng,
    /// Caller-supplied PEM. Tests use this for a local CA.
    Pem(String),
    /// Production bundle plus extra PEM certificates.
    TeslaProdAndPem(String),
    /// Engineering bundle plus extra PEM certificates.
    TeslaEngAndPem(String),
}

/// Listener settings.
pub struct ServerConfig {
    /// Address to bind. `127.0.0.1:0` asks the kernel for a free port.
    pub bind: SocketAddr,
    /// PEM server certificate chain, leaf first.
    pub certificate_pem: String,
    /// PEM private key for [`Self::certificate_pem`].
    pub private_key_pem: String,
    /// CAs that may sign vehicle client certificates.
    pub client_ca: ClientCa,
    /// Bound on the decoded-record channel. Zero is rejected.
    pub channel_capacity: usize,
}

impl ServerConfig {
    /// Prod Tesla client CAs and a channel capacity of 256.
    #[must_use]
    pub fn new(
        bind: SocketAddr,
        certificate_pem: impl Into<String>,
        private_key_pem: impl Into<String>,
    ) -> Self {
        Self {
            bind,
            certificate_pem: certificate_pem.into(),
            private_key_pem: private_key_pem.into(),
            client_ca: ClientCa::TeslaProd,
            channel_capacity: 256,
        }
    }
}

/// One accepted telemetry record, including synthetic connect and disconnect events.
#[derive(Clone, Debug)]
pub struct IncomingRecord {
    /// Websocket topic, or `connectivity` for a synthetic session event.
    pub topic: String,
    /// Transaction id from the envelope. Synthetic events use the connection id.
    pub txid: Vec<u8>,
    /// Envelope message id.
    pub message_id: Vec<u8>,
    /// Sender id from the envelope, lossy-decoded as UTF-8.
    pub sender_id: String,
    /// Device type from the envelope, when the vehicle set it.
    pub device_type: String,
    /// Device id from the envelope, when the vehicle set it.
    pub device_id: String,
    /// Device type parsed from the client certificate.
    pub cert_device_type: String,
    /// Device id parsed from the client certificate. This is the VIN for vehicles.
    pub cert_device_id: String,
    /// Stream `created_at` in unix seconds.
    pub created_at: u32,
    /// Stream `delivered_at_epoch_ms` as sent by the vehicle, usually zero.
    pub delivered_at_epoch_ms: u64,
    /// Local receive time in unix milliseconds.
    pub received_at_epoch_ms: u64,
    /// `Version` header from the websocket handshake.
    pub device_client_version: String,
    /// `X-Network-Interface` header.
    pub network_interface: String,
    /// Id of this TCP session.
    pub connection_id: String,
    /// Protobuf bytes as received. Synthetic connectivity events are encoded by this crate.
    pub payload: Vec<u8>,
    /// Decoded body. The VIN inside protobuf bodies is the certificate device id.
    pub decoded: Decoded,
}

impl IncomingRecord {
    /// One-line description for logs and the example receiver.
    #[must_use]
    pub fn summary(&self) -> String {
        match &self.decoded {
            Decoded::VehicleData(data) => {
                let fields = data
                    .iter()
                    .map(|datum| format!("{}={}", datum.field.name(), datum.value))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("V vin={} {fields}", data.vin())
            }
            Decoded::Alerts(alerts) => {
                format!(
                    "alerts vin={} count={}",
                    alerts.vin(),
                    alerts.alerts().len()
                )
            }
            Decoded::Errors(errors) => {
                format!(
                    "errors vin={} count={}",
                    errors.vin(),
                    errors.errors().len()
                )
            }
            Decoded::Connectivity(event) => format!(
                "connectivity vin={} status={} id={}",
                event.vin(),
                event.status().name(),
                event.connection_id()
            ),
            Decoded::Unrecognized => format!("topic={} device={}", self.topic, self.cert_device_id),
        }
    }
}

trait StatusName {
    fn name(&self) -> String;
}

impl StatusName for record::TypedEnum<ConnectivityEvent> {
    fn name(&self) -> String {
        match self {
            record::TypedEnum::Known(value) => value.as_str_name().to_owned(),
            record::TypedEnum::Unknown(value) => format!("unknown:{value}"),
        }
    }
}

/// Websocket listener. Dropping it stops accept and aborts open connections.
pub struct TelemetryServer {
    local_addr: SocketAddr,
    incoming: mpsc::Receiver<IncomingRecord>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl TelemetryServer {
    /// Binds `config.bind` and starts accepting vehicle connections.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::InvalidConfig`] for an empty channel or unusable PEM, and
    /// [`TelemetryError::Listener`] when the socket cannot bind.
    pub async fn bind(config: ServerConfig) -> Result<Self, TelemetryError> {
        if config.channel_capacity == 0 {
            return Err(TelemetryError::InvalidConfig(
                "channel capacity must be non-zero",
            ));
        }
        let tls = tls_acceptor(&config)?;
        let listener = TcpListener::bind(config.bind)
            .await
            .map_err(TelemetryError::listener)?;
        let local_addr = listener.local_addr().map_err(TelemetryError::listener)?;
        let (tx, rx) = mpsc::channel(config.channel_capacity);
        let (shutdown, notify) = watch::channel(false);
        let task = tokio::spawn(async move {
            accept_loop(listener, tls, tx, notify).await;
        });
        Ok(Self {
            local_addr,
            incoming: rx,
            shutdown,
            task,
        })
    }

    /// Address the listener bound, including an ephemeral port.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Decoded records and synthetic connectivity events.
    pub fn incoming(&mut self) -> &mut mpsc::Receiver<IncomingRecord> {
        &mut self.incoming
    }
}

impl Drop for TelemetryServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}

fn tls_acceptor(config: &ServerConfig) -> Result<TlsAcceptor, TelemetryError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = trust_anchors(&config.client_ca)?;
    let verifier = WebPkiClientVerifier::builder_with_provider(roots, Arc::clone(&provider))
        .build()
        .map_err(TelemetryError::tls)?;
    let certificates = load_certs(&config.certificate_pem)?;
    let key = load_key(&config.private_key_pem)?;
    let server = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(TelemetryError::tls)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certificates, key)
        .map_err(TelemetryError::tls)?;
    Ok(TlsAcceptor::from(Arc::new(server)))
}

fn trust_anchors(source: &ClientCa) -> Result<Arc<RootCertStore>, TelemetryError> {
    let mut pem = String::new();
    match source {
        ClientCa::TeslaProd | ClientCa::TeslaProdAndPem(_) => pem.push_str(PROD_CA),
        ClientCa::TeslaEng | ClientCa::TeslaEngAndPem(_) => pem.push_str(ENG_CA),
        ClientCa::Pem(_) => {}
    }
    if let ClientCa::Pem(extra)
    | ClientCa::TeslaProdAndPem(extra)
    | ClientCa::TeslaEngAndPem(extra) = source
    {
        if !pem.is_empty() {
            pem.push('\n');
        }
        pem.push_str(extra);
    }
    let mut roots = RootCertStore::empty();
    let mut added = 0;
    for certificate in load_certs(&pem)? {
        if roots.add(certificate).is_ok() {
            added += 1;
        }
    }
    if added == 0 {
        return Err(TelemetryError::InvalidConfig(
            "client CA bundle did not contain a usable certificate",
        ));
    }
    Ok(Arc::new(roots))
}

fn load_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>, TelemetryError> {
    let certificates = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(TelemetryError::tls)?;
    if certificates.is_empty() {
        return Err(TelemetryError::InvalidConfig(
            "PEM did not contain a certificate",
        ));
    }
    Ok(certificates)
}

fn load_key(pem: &str) -> Result<PrivateKeyDer<'static>, TelemetryError> {
    PrivateKeyDer::from_pem_slice(pem.as_bytes()).map_err(TelemetryError::tls)
}

async fn accept_loop(
    listener: TcpListener,
    tls: TlsAcceptor,
    tx: mpsc::Sender<IncomingRecord>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { break };
                let tls = tls.clone();
                let tx = tx.clone();
                connections.spawn(async move {
                    if let Err(error) = handle_connection(stream, peer, tls, tx).await {
                        let _ = error;
                    }
                });
            }
        }
    }
    connections.abort_all();
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    tls: TlsAcceptor,
    tx: mpsc::Sender<IncomingRecord>,
) -> Result<(), TelemetryError> {
    let tls_stream = tls.accept(stream).await.map_err(TelemetryError::tls)?;
    let leaf = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(|certificate| certificate.as_ref().to_vec())
        .ok_or(TelemetryError::Certificate)?;
    let identity = Identity::from_der(&leaf)?;
    let headers = Arc::new(Mutex::new(HandshakeHeaders::default()));
    let mut socket =
        tokio_tungstenite::accept_hdr_async(tls_stream, CaptureHeaders(Arc::clone(&headers)))
            .await
            .map_err(TelemetryError::socket)?;
    let headers = headers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let connection_id = format!("{peer}-{}", NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed));
    emit_connectivity(
        &tx,
        &identity,
        &headers,
        &connection_id,
        ConnectivityEvent::Connected,
    )
    .await;
    let result = read_frames(&mut socket, &identity, &headers, &connection_id, &tx).await;
    emit_connectivity(
        &tx,
        &identity,
        &headers,
        &connection_id,
        ConnectivityEvent::Disconnected,
    )
    .await;
    result
}

async fn read_frames<S>(
    socket: &mut S,
    identity: &Identity,
    headers: &HandshakeHeaders,
    connection_id: &str,
    tx: &mpsc::Sender<IncomingRecord>,
) -> Result<(), TelemetryError>
where
    S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    while let Some(message) = socket.next().await {
        let message = message.map_err(TelemetryError::socket)?;
        match message {
            WsMessage::Binary(bytes) => {
                let response =
                    handle_frame(bytes.as_ref(), identity, headers, connection_id, tx).await?;
                if let Some(response) = response {
                    socket
                        .send(WsMessage::from(response))
                        .await
                        .map_err(TelemetryError::socket)?;
                }
            }
            WsMessage::Ping(payload) => {
                socket
                    .send(WsMessage::Pong(payload))
                    .await
                    .map_err(TelemetryError::socket)?;
            }
            WsMessage::Close(_) | WsMessage::Text(_) => break,
            WsMessage::Pong(_) | WsMessage::Frame(_) => {}
        }
    }
    Ok(())
}

async fn handle_frame(
    bytes: &[u8],
    identity: &Identity,
    headers: &HandshakeHeaders,
    connection_id: &str,
    tx: &mpsc::Sender<IncomingRecord>,
) -> Result<Option<Vec<u8>>, TelemetryError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Ok(Some(envelope::encode_format_error(&[])?));
    }
    let Ok(envelope) = envelope::decode_envelope(bytes) else {
        return Ok(Some(envelope::encode_format_error(&[])?));
    };
    match envelope.kind {
        // Ignore acks so two peers cannot acknowledge each other forever.
        MessageKind::Ack => return Ok(None),
        // The reference server acks unknown types so the vehicle does not retry.
        MessageKind::Other(_) => {
            return Ok(Some(envelope::encode_ack(&envelope.txid, &envelope.topic)?));
        }
        MessageKind::Stream => {}
    }
    let topic = String::from_utf8_lossy(&envelope.topic).into_owned();
    let options = DecodeOptions {
        vin_override: Some(identity.device_id.clone()),
        device_client_version: Some(headers.version.clone()),
    };
    let decoded = match topic.as_str() {
        "V" | "alerts" | "errors" | "connectivity" => {
            match record::decode_topic(&topic, &envelope.payload, &options) {
                Ok(decoded) => decoded,
                Err(_) => return Ok(Some(envelope::encode_format_error(&envelope.txid)?)),
            }
        }
        _ => Decoded::Unrecognized,
    };
    let record = record_from_envelope(&envelope, identity, headers, connection_id, decoded);
    if tx.send(record).await.is_err() {
        return Err(TelemetryError::socket("record receiver was dropped"));
    }
    Ok(Some(envelope::encode_ack(&envelope.txid, &envelope.topic)?))
}

fn record_from_envelope(
    envelope: &crate::envelope::Envelope,
    identity: &Identity,
    headers: &HandshakeHeaders,
    connection_id: &str,
    decoded: Decoded,
) -> IncomingRecord {
    IncomingRecord {
        topic: String::from_utf8_lossy(&envelope.topic).into_owned(),
        txid: envelope.txid.clone(),
        message_id: envelope.message_id.clone(),
        sender_id: String::from_utf8_lossy(&envelope.sender_id).into_owned(),
        device_type: String::from_utf8_lossy(&envelope.device_type).into_owned(),
        device_id: String::from_utf8_lossy(&envelope.device_id).into_owned(),
        cert_device_type: identity.device_type.clone(),
        cert_device_id: identity.device_id.clone(),
        created_at: envelope.created_at,
        delivered_at_epoch_ms: envelope.delivered_at_epoch_ms,
        received_at_epoch_ms: received_at_epoch_ms(),
        device_client_version: headers.version.clone(),
        network_interface: headers.network_interface.clone(),
        connection_id: connection_id.to_owned(),
        payload: envelope.payload.clone(),
        decoded,
    }
}

#[derive(Clone, Debug, Default)]
struct HandshakeHeaders {
    version: String,
    network_interface: String,
}

struct CaptureHeaders(Arc<Mutex<HandshakeHeaders>>);

impl tokio_tungstenite::tungstenite::handshake::server::Callback for CaptureHeaders {
    fn on_request(
        self,
        request: &tokio_tungstenite::tungstenite::handshake::server::Request,
        response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        let mut headers = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        headers.version = header_value(request, "version");
        headers.network_interface = header_value(request, "x-network-interface");
        Ok(response)
    }
}

fn header_value(
    request: &tokio_tungstenite::tungstenite::handshake::server::Request,
    name: &str,
) -> String {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

async fn emit_connectivity(
    tx: &mpsc::Sender<IncomingRecord>,
    identity: &Identity,
    headers: &HandshakeHeaders,
    connection_id: &str,
    status: ConnectivityEvent,
) {
    let created = prost_types::Timestamp {
        seconds: i64::try_from(received_at_epoch_ms() / 1000).unwrap_or(0),
        nanos: 0,
    };
    let message = VehicleConnectivity {
        vin: identity.device_id.clone(),
        connection_id: connection_id.to_owned(),
        status: status as i32,
        created_at: Some(created),
        network_interface: headers.network_interface.clone(),
    };
    let payload = message.encode_to_vec();
    let Ok(decoded) = record::decode_connectivity(
        &payload,
        &DecodeOptions {
            vin_override: Some(identity.device_id.clone()),
            device_client_version: None,
        },
    ) else {
        return;
    };
    let record = IncomingRecord {
        topic: "connectivity".to_owned(),
        txid: connection_id.as_bytes().to_vec(),
        message_id: Vec::new(),
        sender_id: identity.sender_id.clone(),
        device_type: identity.device_type.clone(),
        device_id: identity.device_id.clone(),
        cert_device_type: identity.device_type.clone(),
        cert_device_id: identity.device_id.clone(),
        created_at: u32::try_from(created.seconds).unwrap_or(0),
        delivered_at_epoch_ms: 0,
        received_at_epoch_ms: received_at_epoch_ms(),
        device_client_version: headers.version.clone(),
        network_interface: headers.network_interface.clone(),
        connection_id: connection_id.to_owned(),
        payload,
        decoded: Decoded::Connectivity(decoded),
    };
    let _ = tx.send(record).await;
}

fn received_at_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
        IsCa, KeyPair, KeyUsagePurpose,
    };
    use rustls::RootCertStore;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;

    use super::{ClientCa, ServerConfig, TelemetryServer};

    struct Material {
        ca_pem: String,
        server_cert: String,
        server_key: String,
        client_cert: String,
        client_key: String,
    }

    fn material() -> Material {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_name = DistinguishedName::new();
        ca_name.push(DnType::CommonName, "TeslaMotors");
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        ca_params.distinguished_name = ca_name;
        let ca = ca_params.self_signed(&ca_key).unwrap();

        let server_key = KeyPair::generate().unwrap();
        let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        server_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        server_params.use_authority_key_identifier_extension = true;
        let server = server_params.signed_by(&server_key, &ca, &ca_key).unwrap();

        let client_key = KeyPair::generate().unwrap();
        let mut client_params = CertificateParams::default();
        let mut client_name = DistinguishedName::new();
        client_name.push(DnType::CommonName, "5YJ3E1EA7KF000001");
        client_params.distinguished_name = client_name;
        client_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        client_params.use_authority_key_identifier_extension = true;
        let client = client_params.signed_by(&client_key, &ca, &ca_key).unwrap();

        Material {
            ca_pem: ca.pem(),
            server_cert: server.pem(),
            server_key: server_key.serialize_pem(),
            client_cert: client.pem(),
            client_key: client_key.serialize_pem(),
        }
    }

    #[test]
    fn vendored_tesla_ca_bundles_contain_trust_anchors() {
        super::trust_anchors(&ClientCa::TeslaProd).unwrap();
        super::trust_anchors(&ClientCa::TeslaEng).unwrap();
    }

    #[tokio::test]
    async fn mutual_tls_listener_decodes_a_go_frame_and_acks_it() {
        let material = material();
        let mut config = ServerConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            material.server_cert,
            material.server_key,
        );
        config.client_ca = ClientCa::Pem(material.ca_pem.clone());
        let mut server = TelemetryServer::bind(config).await.unwrap();
        let address = server.local_addr();

        let ca_pem = material.ca_pem.clone();
        let client_cert = material.client_cert.clone();
        let client_key = material.client_key.clone();
        let frame = include_bytes!("../tests/fixtures/stream_vehicle_data.bin").to_vec();
        let expected_ack = include_bytes!("../tests/fixtures/stream_ack.bin").to_vec();
        let client = tokio::spawn(async move {
            let mut roots = RootCertStore::empty();
            let ca = CertificateDer::from_pem_slice(ca_pem.as_bytes()).unwrap();
            roots.add(ca).unwrap();
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let client_config = rustls::ClientConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_root_certificates(roots)
                .with_client_auth_cert(
                    CertificateDer::pem_slice_iter(client_cert.as_bytes())
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap(),
                    PrivateKeyDer::from_pem_slice(client_key.as_bytes()).unwrap(),
                )
                .unwrap();
            let connector = TlsConnector::from(Arc::new(client_config));
            let tcp = TcpStream::connect(address).await.unwrap();
            let tls = connector
                .connect(ServerName::try_from("localhost").unwrap(), tcp)
                .await
                .unwrap();
            let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost/", tls)
                .await
                .unwrap();
            use futures_util::{SinkExt, StreamExt};
            socket
                .send(tokio_tungstenite::tungstenite::Message::from(frame))
                .await
                .unwrap();
            let ack = socket.next().await.unwrap().unwrap();
            let tokio_tungstenite::tungstenite::Message::Binary(bytes) = ack else {
                panic!("expected binary ack");
            };
            assert_eq!(bytes.as_ref(), expected_ack.as_slice());
            let _ = socket.close(None).await;
        });

        let connected = tokio::time::timeout(Duration::from_secs(5), server.incoming().recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connected.topic, "connectivity");
        assert_eq!(connected.cert_device_id, "5YJ3E1EA7KF000001");
        assert_eq!(connected.cert_device_type, "vehicle_device");

        let record = tokio::time::timeout(Duration::from_secs(5), server.incoming().recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.topic, "V");
        assert!(record.summary().contains("Soc=81.5"));
        assert!(record.summary().contains("5YJ3E1EA7KF000001"));
        client.await.unwrap();

        let disconnected = tokio::time::timeout(Duration::from_secs(5), server.incoming().recv())
            .await
            .unwrap()
            .unwrap();
        assert!(disconnected.summary().contains("DISCONNECTED"));
    }
}
