// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost::Message;
use secrecy::ExposeSecret;

use crate::crypto::{self, constant_time_eq};
use crate::domain::Domain;
use crate::key::{PrivateKey, PrivateKeyFormat};
use crate::message::{self, RoutableMessage};
use crate::metadata::Metadata;
use crate::proto::signatures::SessionInfo;
use crate::proto::universal_message::routable_message::{Payload, SubSigData};
use crate::session::Session;
use crate::transport::{CommandTransport, WakeStatus};
use crate::{CommandError, Vehicle};

const VIN: &str = "5YJ30123456789ABC";
const CLIENT_PEM: &str = "-----BEGIN EC PRIVATE KEY-----
MHcCAQEEICU4zcKal8GcHpmmN9bPT4yXDBGLVu3h5jI+bRYsSzDboAoGCCqGSM49
AwEHoUQDQgAEsra8aMLaBmXOZWgVWUmWxiOU7di+qQX+eBp1T+aoRacUMwkC8iXp
Jp1GbgWzSZgf2p2FzCPG+0RKpztikQXcbg==
-----END EC PRIVATE KEY-----";
const CLIENT_PUBLIC: &str = "04b2b6bc68c2da0665ce656815594996c62394edd8bea905fe781a754fe6a845a714330902f225e9269d466e05b349981fda9d85cc23c6fb444aa73b629105dc6e";
const VEHICLE_PUBLIC: &str = "04c7a1f47138486aa4729971494878d33b1a24e39571f748a6e16c5955b3d877d3a6aaa0e955166474af5d32c410f439a2234137ad1bb085fd4e8813c958f11d97";
const SHARED: &str = "1b2fce19967b79db696f909cff89ea9a";
const SESSION_INFO: &str = "0806124104c7a1f47138486aa4729971494878d33b1a24e39571f748a6e16c5955b3d877d3a6aaa0e955166474af5d32c410f439a2234137ad1bb085fd4e8813c958f11d971a104c463f9cc0d3d26906e982ed224adde6255a0a0000";
const CHALLENGE: &str = "1588d5a30eabc6f8fc9a951b11f6fd11";
const SESSION_TAG: &str = "996c1fe38331be138f8039c194b14db2198846ed7d8251e6749284d7b32ea002";
const HVAC: &str = "120452020801";
const HMAC_TAG: &str = "f3c24554a00e42160e6acef6dd4f1c8ef9c30ed6391ec818d75663bee00a47f0";
const AES_META: &str = "000105010103021135594a333031323334353637383941424303104c463f9cc0d3d26906e982ed224adde6040400000a5f050400000007070400000002ff";
const AES_NONCE: &str = "dbf79447fa156674dae1caed";
const AES_CT: &str = "38038e8c0f2e";
const AES_TAG: &str = "c228e0ff64991481db3a7bbc133696c5";

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

fn client_key() -> PrivateKey {
    PrivateKey::from_pem(CLIENT_PEM).unwrap()
}

fn sign(
    session: &mut Session,
    message: &mut RoutableMessage,
    now: SystemTime,
    nonce: Option<&[u8; 12]>,
) {
    let key = client_key();
    let public = *key.public_key_uncompressed();
    session
        .authorize(
            message,
            &crate::session::SignRequest {
                vin: VIN,
                local_public: &public,
                domain: Domain::Infotainment,
                now,
                ttl: Duration::from_secs(5),
                nonce,
            },
        )
        .unwrap();
}

fn fixed_now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

#[test]
fn pem_round_trip_matches_published_public_key() {
    let key = client_key();
    assert_eq!(key.public_key_uncompressed().as_slice(), hex(CLIENT_PUBLIC));
    let rendered = format!("{key:?}");
    assert!(!rendered.contains("2538cdc2"));
    assert!(!rendered.contains("MHcCAQEE"));
    let sec1 = key.to_sec1_pem().unwrap();
    let again = PrivateKey::from_pem(sec1.expose_secret()).unwrap();
    assert_eq!(
        again.public_key_uncompressed(),
        key.public_key_uncompressed()
    );
    let pkcs8 = key.to_pkcs8_pem().unwrap();
    assert!(pkcs8.expose_secret().contains("BEGIN PRIVATE KEY"));
    let from_pkcs8 = PrivateKey::from_pem(pkcs8.expose_secret()).unwrap();
    assert_eq!(
        from_pkcs8.public_key_uncompressed(),
        key.public_key_uncompressed()
    );
    let public_pem = key.public_key_pem().unwrap();
    assert!(public_pem.contains("BEGIN PUBLIC KEY"));
    assert!(!format!("{pkcs8:?}").contains("BEGIN PRIVATE KEY"));
}

#[test]
fn rejects_encrypted_and_garbage_pem() {
    let encrypted =
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMII=\n-----END ENCRYPTED PRIVATE KEY-----";
    assert!(matches!(
        PrivateKey::from_pem(encrypted),
        Err(CommandError::EncryptedPrivateKey)
    ));
    assert!(matches!(
        PrivateKey::from_pem("-----BEGIN PRIVATE KEY-----\nYQ==\n-----END PRIVATE KEY-----"),
        Err(CommandError::InvalidPrivateKey)
    ));
    assert!(matches!(
        PrivateKey::from_pem("not a key"),
        Err(CommandError::InvalidPrivateKey)
    ));
}

#[test]
fn shared_secret_matches_protocol_vector() {
    let key = client_key().secret_key().unwrap();
    let shared = crypto::shared_secret(&key, &hex(VEHICLE_PUBLIC)).unwrap();
    assert!(constant_time_eq(shared.expose(), &hex(SHARED)));
    let mut flipped = hex(VEHICLE_PUBLIC);
    flipped[10] ^= 0x01;
    assert!(crypto::shared_secret(&key, &flipped).is_err());
}

#[test]
fn session_info_tag_matches_protocol_vector() {
    let key = client_key().secret_key().unwrap();
    let shared = crypto::shared_secret(&key, &hex(VEHICLE_PUBLIC)).unwrap();
    let tag = crypto::session_info_tag(
        shared.expose(),
        VIN.as_bytes(),
        &hex(CHALLENGE),
        &hex(SESSION_INFO),
    )
    .unwrap();
    assert_eq!(tag.as_slice(), hex(SESSION_TAG));
}

#[test]
fn metadata_sha256_matches_reference_vector() {
    let mut meta = Metadata::new();
    meta.add(0, &[0x05]).unwrap();
    meta.add(1, &[0x02]).unwrap();
    meta.add(2, b"testVIN").unwrap();
    meta.add(3, &hex("aada928a4f215f55f9e6e45e66b6521e"))
        .unwrap();
    meta.add(4, &hex("00000e74")).unwrap();
    meta.add(5, &hex("0000053a")).unwrap();
    let digest = crypto::sha256(&meta.finish(&[]));
    assert_eq!(
        digest.as_slice(),
        hex("abab04d804499813382efd74a06791ce2de777439603246dfbaa8392ca05868e")
    );
}

#[test]
fn aes_gcm_matches_protocol_vector() {
    let key: [u8; 16] = hex(SHARED).try_into().unwrap();
    let nonce: [u8; 12] = hex(AES_NONCE).try_into().unwrap();
    let aad = crypto::sha256(&hex(AES_META));
    let (ciphertext, tag) = crypto::seal(&key, &nonce, &hex(HVAC), &aad).unwrap();
    assert_eq!(ciphertext, hex(AES_CT));
    assert_eq!(tag.as_slice(), hex(AES_TAG));
    let opened = crypto::open(&key, &nonce, &ciphertext, &tag, &aad).unwrap();
    assert_eq!(opened, hex(HVAC));
    let mut bad = tag;
    bad[0] ^= 1;
    assert!(crypto::open(&key, &nonce, &ciphertext, &bad, &aad).is_err());
}

#[test]
fn session_info_protobuf_matches_documented_encoding() {
    let info = SessionInfo {
        counter: 6,
        public_key: hex(VEHICLE_PUBLIC),
        epoch: hex("4c463f9cc0d3d26906e982ed224adde6"),
        clock_time: 2650,
        ..SessionInfo::default()
    };
    assert_eq!(info.encode_to_vec(), hex(SESSION_INFO));
}

#[test]
fn hmac_command_matches_independent_vector() {
    let now = fixed_now();
    let mut session = Session::authenticate(
        &client_key(),
        VIN,
        &hex(CHALLENGE),
        &hex(SESSION_INFO),
        &hex(SESSION_TAG),
        now,
    )
    .unwrap();
    let mut message =
        message::command_message(Domain::Infotainment, &[0x11; 16], &[0x22; 16], &hex(HVAC));
    sign(&mut session, &mut message, now, None);
    let tag = match message.sub_sig_data.as_ref() {
        Some(SubSigData::SignatureData(data)) => match data.sig_type.as_ref() {
            Some(crate::proto::signatures::signature_data::SigType::HmacPersonalizedData(data)) => {
                assert_eq!(data.counter, 7);
                assert_eq!(data.expires_at, 2655);
                data.tag.clone()
            }
            other => panic!("unexpected signature {other:?}"),
        },
        _ => panic!("missing signature"),
    };
    assert_eq!(tag, hex(HMAC_TAG));
    let mut tampered = hex(SESSION_TAG);
    tampered[0] ^= 1;
    assert!(matches!(
        Session::authenticate(
            &client_key(),
            VIN,
            &hex(CHALLENGE),
            &hex(SESSION_INFO),
            &tampered,
            now
        ),
        Err(CommandError::InvalidSession)
    ));
}

#[test]
fn climate_on_encoding_matches_protocol_document() {
    assert_eq!(crate::commands::climate(true), hex(HVAC));
    assert_eq!(crate::commands::lock(), hex("1001"));
}

#[test]
fn detached_encryption_matches_independent_vector() {
    let now = fixed_now();
    let mut info = SessionInfo::decode(hex(SESSION_INFO).as_slice()).unwrap();
    info.flags = 1;
    let encoded = info.encode_to_vec();
    let key = client_key().secret_key().unwrap();
    let shared = crypto::shared_secret(&key, &hex(VEHICLE_PUBLIC)).unwrap();
    let tag = crypto::session_info_tag(shared.expose(), VIN.as_bytes(), &hex(CHALLENGE), &encoded)
        .unwrap();
    let mut session =
        Session::authenticate(&client_key(), VIN, &hex(CHALLENGE), &encoded, &tag, now).unwrap();
    let nonce: [u8; 12] = hex("00112233445566778899aabb").try_into().unwrap();
    let mut message =
        message::command_message(Domain::Infotainment, &[7; 16], &[8; 16], &hex(HVAC));
    sign(&mut session, &mut message, now, Some(&nonce));
    assert_eq!(message::protobuf_bytes(&message), hex("0cd4"));
}

#[test]
fn encrypted_response_opens_and_rejects_replay() {
    let now = fixed_now();
    let mut session = Session::authenticate(
        &client_key(),
        VIN,
        &hex(CHALLENGE),
        &hex(SESSION_INFO),
        &hex(SESSION_TAG),
        now,
    )
    .unwrap();
    let request_hash = hex("08f3c24554a00e42160e6acef6dd4f1c8ef9c30ed6391ec818d75663bee00a47f0");
    let mut message = RoutableMessage {
        from_destination: Some(message::domain_destination(Domain::Infotainment)),
        payload: Some(Payload::ProtobufMessageAsBytes(hex(
            "3b1fa09863104f71eef893",
        ))),
        sub_sig_data: Some(SubSigData::SignatureData(
            crate::proto::signatures::SignatureData {
                signer_identity: None,
                sig_type: Some(
                    crate::proto::signatures::signature_data::SigType::AesGcmResponseData(
                        crate::proto::signatures::AesGcmResponseSignatureData {
                            nonce: hex("aabbccddeeff001122334455"),
                            counter: 9,
                            tag: hex("9009b2e425ec50648e026417e74fcbaf"),
                        },
                    ),
                ),
            },
        )),
        ..RoutableMessage::default()
    };
    let id = crate::session::RequestId::from_bytes(request_hash);
    session.open_response(VIN, &mut message, &id).unwrap();
    assert_eq!(message::protobuf_bytes(&message), b"response-ok");
    message.payload = Some(Payload::ProtobufMessageAsBytes(hex(
        "3b1fa09863104f71eef893",
    )));
    message.sub_sig_data = Some(SubSigData::SignatureData(
        crate::proto::signatures::SignatureData {
            signer_identity: None,
            sig_type: Some(
                crate::proto::signatures::signature_data::SigType::AesGcmResponseData(
                    crate::proto::signatures::AesGcmResponseSignatureData {
                        nonce: hex("aabbccddeeff001122334455"),
                        counter: 9,
                        tag: hex("9009b2e425ec50648e026417e74fcbaf"),
                    },
                ),
            ),
        },
    ));
    assert!(matches!(
        session.open_response(VIN, &mut message, &id),
        Err(CommandError::ReplayedResponse)
    ));
}

#[test]
fn same_epoch_clock_regression_is_ignored_and_new_epoch_adopts_counter() {
    let now = fixed_now();
    let mut session = Session::authenticate(
        &client_key(),
        VIN,
        &hex(CHALLENGE),
        &hex(SESSION_INFO),
        &hex(SESSION_TAG),
        now,
    )
    .unwrap();
    let mut backwards = SessionInfo::decode(hex(SESSION_INFO).as_slice()).unwrap();
    backwards.clock_time = 100;
    backwards.counter = 1;
    let encoded = backwards.encode_to_vec();
    let key = client_key().secret_key().unwrap();
    let shared = crypto::shared_secret(&key, &hex(VEHICLE_PUBLIC)).unwrap();
    let tag = crypto::session_info_tag(shared.expose(), VIN.as_bytes(), &hex(CHALLENGE), &encoded)
        .unwrap();
    session
        .update(VIN, &hex(CHALLENGE), &encoded, &tag, now)
        .unwrap();
    let mut message =
        message::command_message(Domain::Infotainment, &[1; 16], &[2; 16], &hex(HVAC));
    sign(&mut session, &mut message, now, None);
    let counter = match message.sub_sig_data.as_ref() {
        Some(SubSigData::SignatureData(data)) => match data.sig_type.as_ref() {
            Some(crate::proto::signatures::signature_data::SigType::HmacPersonalizedData(data)) => {
                data.counter
            }
            _ => panic!("hmac"),
        },
        _ => panic!("sig"),
    };
    assert_eq!(counter, 7);

    let mut rotated = SessionInfo::decode(hex(SESSION_INFO).as_slice()).unwrap();
    rotated.epoch = vec![9; 16];
    rotated.counter = 2;
    rotated.clock_time = 10;
    let encoded = rotated.encode_to_vec();
    let tag = crypto::session_info_tag(shared.expose(), VIN.as_bytes(), &hex(CHALLENGE), &encoded)
        .unwrap();
    session
        .update(VIN, &hex(CHALLENGE), &encoded, &tag, now)
        .unwrap();
    let mut message =
        message::command_message(Domain::Infotainment, &[1; 16], &[3; 16], &hex(HVAC));
    sign(&mut session, &mut message, now, None);
    let counter = match message.sub_sig_data.as_ref() {
        Some(SubSigData::SignatureData(data)) => match data.sig_type.as_ref() {
            Some(crate::proto::signatures::signature_data::SigType::HmacPersonalizedData(data)) => {
                data.counter
            }
            _ => panic!("hmac"),
        },
        _ => panic!("sig"),
    };
    assert_eq!(counter, 3);
}

struct Script {
    seen: Mutex<Vec<Vec<u8>>>,
}

impl CommandTransport for Script {
    fn vin(&self) -> &str {
        VIN
    }

    fn exchange(
        &self,
        message: &[u8],
    ) -> impl std::future::Future<Output = Result<Vec<u8>, CommandError>> + Send {
        self.seen.lock().expect("seen").push(message.to_vec());
        let result = (|| {
            let request =
                RoutableMessage::decode(message).map_err(|_| CommandError::BadResponse {
                    may_have_succeeded: false,
                })?;
            if matches!(request.payload, Some(Payload::SessionInfoRequest(_))) {
                return Ok(handshake_response(&request));
            }
            let response = RoutableMessage {
                to_destination: request.from_destination.clone(),
                from_destination: Some(message::domain_destination(Domain::VehicleSecurity)),
                request_uuid: request.uuid.clone(),
                payload: Some(Payload::ProtobufMessageAsBytes(Vec::new())),
                ..RoutableMessage::default()
            };
            Ok(message::encode(&response))
        })();
        std::future::ready(result)
    }

    fn wake(&self) -> impl std::future::Future<Output = Result<WakeStatus, CommandError>> + Send {
        std::future::ready(Ok(WakeStatus {
            state: Some("online".to_owned()),
            online: true,
        }))
    }
}

fn handshake_response(request: &RoutableMessage) -> Vec<u8> {
    let info = SessionInfo {
        counter: 6,
        public_key: hex(VEHICLE_PUBLIC),
        epoch: hex("4c463f9cc0d3d26906e982ed224adde6"),
        clock_time: 2650,
        ..SessionInfo::default()
    };
    let encoded = info.encode_to_vec();
    let public = match request.payload.as_ref() {
        Some(Payload::SessionInfoRequest(body)) => body.public_key.clone(),
        _ => panic!("expected session info request"),
    };
    let vehicle = PrivateKey::from_pem(
        "-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIDRO5bRmp88e6xK29QMx2y5exYNO9fS+/P2MvlXCUo1woAoGCCqGSM49
AwEHoUQDQgAEx6H0cThIaqRymXFJSHjTOxok45Vx90im4WxZVbPYd9OmqqDpVRZk
dK9dMsQQ9DmiI0E3rRuwhf1OiBPJWPEdlw==
-----END EC PRIVATE KEY-----",
    )
    .unwrap();
    let shared = crypto::shared_secret(&vehicle.secret_key().unwrap(), &public).unwrap();
    let tag =
        crypto::session_info_tag(shared.expose(), VIN.as_bytes(), &request.uuid, &encoded).unwrap();
    let response = RoutableMessage {
        to_destination: request.from_destination.clone(),
        from_destination: Some(message::domain_destination(Domain::VehicleSecurity)),
        request_uuid: request.uuid.clone(),
        payload: Some(Payload::SessionInfo(encoded)),
        sub_sig_data: Some(SubSigData::SignatureData(
            crate::proto::signatures::SignatureData {
                signer_identity: None,
                sig_type: Some(
                    crate::proto::signatures::signature_data::SigType::SessionInfoTag(
                        crate::proto::signatures::HmacSignatureData { tag: tag.to_vec() },
                    ),
                ),
            },
        )),
        ..RoutableMessage::default()
    };
    message::encode(&response)
}

#[tokio::test]
async fn lock_handshakes_then_sends_hmac_and_accepts_empty_vcsec_success() {
    let transport = Script {
        seen: Mutex::new(Vec::new()),
    };
    let mut vehicle = Vehicle::new(transport, client_key()).unwrap();
    let wake = vehicle.wake().await.unwrap();
    assert!(wake.online);
    vehicle.lock().await.unwrap();
    let seen = vehicle.transport().seen.lock().expect("seen").clone();
    assert_eq!(seen.len(), 2);
    let request = RoutableMessage::decode(seen[0].as_slice()).unwrap();
    assert!(matches!(
        request.payload,
        Some(Payload::SessionInfoRequest(_))
    ));
    let command = RoutableMessage::decode(seen[1].as_slice()).unwrap();
    assert!(matches!(
        command.sub_sig_data,
        Some(SubSigData::SignatureData(_))
    ));
    assert_eq!(command.flags, message::FLAG_ENCRYPT_RESPONSE);
    assert_eq!(message::protobuf_bytes(&command), crate::commands::lock());
}

#[test]
fn key_file_round_trip_uses_restrictive_mode() {
    let dir = std::env::temp_dir().join(format!("tesla-key-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("virtual.key");
    let key = PrivateKey::generate().unwrap();
    key.write_pem_file(&path, PrivateKeyFormat::Sec1).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let loaded = PrivateKey::read_file(&path).unwrap();
    assert_eq!(
        loaded.public_key_uncompressed(),
        key.public_key_uncompressed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fleet_transport_posts_signed_command_locally() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/api/1/vehicles/5YJ30123456789ABC/signed_command",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_string(r#"{"response":"b2s="}"#),
        )
        .mount(&server)
        .await;
    let fleet = tesla_fleet_api::FleetClient::builder(
        &server.uri(),
        tesla_fleet_api::BearerToken::new("token"),
    )
    .unwrap()
    .build()
    .unwrap();
    let transport = crate::FleetTransport::new(fleet, VIN).unwrap();
    let mut vehicle = Vehicle::new(transport, client_key()).unwrap();
    let error = vehicle.handshake(Domain::Infotainment).await.unwrap_err();
    assert!(matches!(
        error,
        CommandError::BadResponse { .. } | CommandError::InvalidSession
    ));
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(body.contains("routable_message"));
    assert!(!body.contains("token"));
}

#[test]
fn charge_and_temperature_payloads_reject_bad_inputs() {
    assert!(crate::commands::charge_limit(101).is_err());
    assert!(crate::commands::charge_limit(80).is_ok());
    assert!(crate::commands::charging_amps(0).is_err());
    assert!(crate::commands::temperatures(f32::NAN, 20.0).is_err());
    assert!(crate::commands::temperatures(21.5, 21.5).is_ok());
    assert_ne!(crate::commands::honk(), Vec::<u8>::new());
    assert_ne!(crate::commands::flash(), Vec::<u8>::new());
    assert_ne!(crate::commands::charge_start(), Vec::<u8>::new());
    assert_ne!(crate::commands::charge_stop(), Vec::<u8>::new());
}
