// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use url::Url;
use zeroize::Zeroize;

use crate::auth::host_is_loopback;
use crate::error::OwnerError;

/// Comma-separated columns TeslaMate subscribes to, in wire order.
pub const STREAM_COLUMNS: &str = "speed,odometer,soc,elevation,est_heading,est_lat,est_lng,power,shift_state,range,est_range,heading";

const COLUMN_COUNT: usize = 12;
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// One `data:update` row.
///
/// Distances and speeds are miles. Elevation is feet. Power is kilowatts. Empty CSV fields are
/// [`None`]. An empty `shift_state` is [`None`] and means "no value in this frame", not Park.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamSample {
    /// Unix time in milliseconds.
    pub recorded_at_ms: i64,
    /// Miles per hour.
    pub speed_mph: Option<f64>,
    /// Miles.
    pub odometer_mi: Option<f64>,
    /// State of charge, percent.
    pub soc: Option<f64>,
    /// Feet.
    pub elevation_ft: Option<f64>,
    /// Estimated GPS heading, degrees.
    pub est_heading: Option<f64>,
    /// Estimated latitude, decimal degrees.
    pub est_lat: Option<f64>,
    /// Estimated longitude, decimal degrees.
    pub est_lng: Option<f64>,
    /// Kilowatts. Negative is regen.
    pub power_kw: Option<f64>,
    /// `P`, `D`, `R`, or `N`.
    pub shift_state: Option<String>,
    /// Rated range, miles.
    pub range_mi: Option<f64>,
    /// Estimated range, miles.
    pub est_range_mi: Option<f64>,
    /// Compass heading, degrees.
    pub heading: Option<f64>,
}

/// A decoded streaming frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A telemetry row.
    Update(StreamSample),
    /// Tesla asked the client to reconnect.
    Error {
        /// `error_type` from the frame, when present.
        error_type: String,
        /// `value` from the frame, when present.
        value: String,
    },
    /// `control:hello`.
    Hello,
    /// A frame this client does not use.
    Ignore,
}

/// Why the socket should reconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureClass {
    /// The car stopped sending, or the read timed out. Normal while parked.
    Timeout,
    /// The vehicle closed the stream. Normal while asleep.
    VehicleDisconnected,
    /// The vehicle reported an error.
    VehicleError,
    /// The access token was rejected. Refresh it before reconnecting.
    Auth,
    /// The TCP or websocket connection failed.
    Transport,
    /// The legacy endpoint refused the connection. Back off for a long time.
    Unavailable,
}

impl FailureClass {
    /// Stable label for logs and the state API. This is not a token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::VehicleDisconnected => "vehicle_disconnected",
            Self::VehicleError => "vehicle_error",
            Self::Auth => "auth",
            Self::Transport => "transport",
            Self::Unavailable => "unavailable",
        }
    }
}

/// How long to wait before the next connection attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPlan {
    /// Delay before the next attempt.
    pub after: Duration,
    /// When true, refresh the Owner API access token first.
    pub refresh_token: bool,
}

/// How a [`pump_stream`] call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpEnd {
    /// The socket ended and the caller should reconnect.
    Failed(FailureClass),
}

/// Parses one text frame.
///
/// # Errors
///
/// Returns [`OwnerError::Malformed`] when the frame is not JSON or the update row has too many
/// columns. A short row is accepted. Missing cells are [`None`].
pub fn parse_message(text: &str) -> Result<Incoming, OwnerError> {
    let value: Value = serde_json::from_str(text).map_err(|_| OwnerError::Malformed)?;
    let kind = value.get("msg_type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "data:update" => {
            let csv = value
                .get("value")
                .and_then(Value::as_str)
                .ok_or(OwnerError::Malformed)?;
            Ok(Incoming::Update(parse_csv(csv)?))
        }
        "data:error" => Ok(Incoming::Error {
            error_type: value
                .get("error_type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            value: value
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        }),
        "control:hello" => Ok(Incoming::Hello),
        _ => Ok(Incoming::Ignore),
    }
}

/// JSON subscribe frame. The string contains `access_token`. Do not log it.
#[must_use]
pub fn subscribe_frame(access_token: &str, vehicle_id: i64) -> String {
    serde_json::json!({
        "msg_type": "data:subscribe_oauth",
        "token": access_token,
        "value": STREAM_COLUMNS,
        "tag": vehicle_id.to_string(),
    })
    .to_string()
}

/// Classifies a `data:error` frame.
#[must_use]
pub fn classify_stream_error(error_type: &str, value: &str) -> FailureClass {
    let text = format!("{error_type} {value}").to_ascii_lowercase();
    if [
        "token",
        "unauthorized",
        "invalid_grant",
        "login_required",
        "401",
    ]
    .iter()
    .any(|needle| text.contains(needle))
    {
        FailureClass::Auth
    } else if text.contains("timeout") {
        FailureClass::Timeout
    } else if text.contains("disconnect") {
        FailureClass::VehicleDisconnected
    } else {
        FailureClass::VehicleError
    }
}

/// Classifies a websocket or HTTP handshake failure.
///
/// The message is a client error string. It must not be a subscribe frame.
#[must_use]
pub fn classify_transport_message(message: &str) -> FailureClass {
    let text = message.to_ascii_lowercase();
    if text.contains("401") || text.contains("unauthorized") {
        FailureClass::Auth
    } else if text.contains("403") || text.contains("404") || text.contains("410") {
        FailureClass::Unavailable
    } else {
        FailureClass::Transport
    }
}

/// Bounded reconnect delay.
///
/// `consecutive` is the number of failures since the last sample, starting at 1.
/// Timeout and disconnect stay at or below 60 seconds so a drive is noticed quickly.
/// Auth delays grow to 15 minutes and request a token refresh.
/// [`FailureClass::Unavailable`] waits 15 minutes and does not refresh.
#[must_use]
pub fn plan_reconnect(class: FailureClass, consecutive: u32) -> ReconnectPlan {
    let (base, cap, refresh_token) = match class {
        FailureClass::Timeout | FailureClass::VehicleDisconnected => (10, 60, false),
        FailureClass::VehicleError => (15, 300, false),
        FailureClass::Transport => (2, 60, false),
        FailureClass::Auth => (60, 900, true),
        FailureClass::Unavailable => (900, 900, false),
    };
    ReconnectPlan {
        after: capped_exp(base, cap, consecutive),
        refresh_token,
    }
}

/// Reads one streaming session until Tesla closes it or a frame asks for a reconnect.
///
/// Samples are sent on `samples`. The access token is copied into the subscribe frame only.
pub async fn pump_stream(
    url: &str,
    access_token: &SecretString,
    vehicle_id: i64,
    samples: mpsc::Sender<StreamSample>,
) -> PumpEnd {
    if ensure_stream_url(url).is_err() {
        return PumpEnd::Failed(FailureClass::Unavailable);
    }
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(64 * 1024);
    config.max_frame_size = Some(64 * 1024);
    let connection = connect_async_with_config(url, Some(config), true).await;
    let (socket, _) = match connection {
        Ok(pair) => pair,
        Err(err) => {
            return PumpEnd::Failed(classify_transport_message(&err.to_string()));
        }
    };
    let (mut write, mut read) = socket.split();
    let mut frame = subscribe_frame(access_token.expose_secret(), vehicle_id);
    let send = write.send(Message::Text(frame.clone().into())).await;
    frame.zeroize();
    if send.is_err() {
        return PumpEnd::Failed(FailureClass::Transport);
    }
    loop {
        let next = tokio::time::timeout(READ_TIMEOUT, read.next()).await;
        let message = match next {
            Err(_) => return PumpEnd::Failed(FailureClass::Timeout),
            Ok(None) => return PumpEnd::Failed(FailureClass::Transport),
            Ok(Some(Err(err))) => {
                return PumpEnd::Failed(classify_transport_message(&err.to_string()));
            }
            Ok(Some(Ok(message))) => message,
        };
        match message {
            Message::Text(text) => match parse_message(text.as_str()) {
                Ok(Incoming::Update(sample)) => {
                    if samples.send(sample).await.is_err() {
                        return PumpEnd::Failed(FailureClass::Transport);
                    }
                }
                Ok(Incoming::Error { error_type, value }) => {
                    return PumpEnd::Failed(classify_stream_error(&error_type, &value));
                }
                Ok(Incoming::Hello | Incoming::Ignore) | Err(_) => {}
            },
            Message::Ping(payload) => {
                if write.send(Message::Pong(payload)).await.is_err() {
                    return PumpEnd::Failed(FailureClass::Transport);
                }
            }
            Message::Close(_) => return PumpEnd::Failed(FailureClass::Transport),
            Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
        }
    }
}

fn parse_csv(csv: &str) -> Result<StreamSample, OwnerError> {
    let mut cells = csv.split(',');
    let stamp = cells.next().ok_or(OwnerError::Malformed)?;
    let recorded_at_ms = parse_timestamp(stamp).ok_or(OwnerError::Malformed)?;
    let mut fields = Vec::with_capacity(COLUMN_COUNT);
    for cell in cells {
        fields.push(cell);
        if fields.len() > COLUMN_COUNT {
            return Err(OwnerError::Malformed);
        }
    }
    let cell = |index: usize| fields.get(index).copied().unwrap_or("");
    Ok(StreamSample {
        recorded_at_ms,
        speed_mph: parse_f64(cell(0)),
        odometer_mi: parse_f64(cell(1)),
        soc: parse_f64(cell(2)),
        elevation_ft: parse_f64(cell(3)),
        est_heading: parse_f64(cell(4)),
        est_lat: parse_f64(cell(5)),
        est_lng: parse_f64(cell(6)),
        power_kw: parse_f64(cell(7)),
        shift_state: parse_shift(cell(8)),
        range_mi: parse_f64(cell(9)),
        est_range_mi: parse_f64(cell(10)),
        heading: parse_f64(cell(11)),
    })
}

fn parse_timestamp(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let raw: i64 = text.parse().ok()?;
    if raw <= 0 {
        return None;
    }
    if raw >= 10_000_000_000 {
        Some(raw)
    } else {
        raw.checked_mul(1000)
    }
}

fn parse_f64(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("null") {
        return None;
    }
    let value: f64 = text.parse().ok()?;
    value.is_finite().then_some(value)
}

fn parse_shift(text: &str) -> Option<String> {
    match text.trim() {
        "D" | "d" => Some("D".to_owned()),
        "R" | "r" => Some("R".to_owned()),
        "N" | "n" => Some("N".to_owned()),
        "P" | "p" => Some("P".to_owned()),
        _ => None,
    }
}

fn capped_exp(base_secs: u64, cap_secs: u64, consecutive: u32) -> Duration {
    let exp = consecutive.max(1).saturating_sub(1).min(6);
    let factor = 1_u64 << exp;
    Duration::from_secs(base_secs.saturating_mul(factor).min(cap_secs))
}

fn ensure_stream_url(url: &str) -> Result<(), OwnerError> {
    let url = Url::parse(url).map_err(|_| OwnerError::InsecureStreamUrl)?;
    match url.scheme() {
        "wss" => Ok(()),
        "ws" if host_is_loopback(&url) => Ok(()),
        _ => Err(OwnerError::InsecureStreamUrl),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FailureClass, Incoming, PumpEnd, STREAM_COLUMNS, classify_stream_error,
        classify_transport_message, parse_message, plan_reconnect, pump_stream, subscribe_frame,
    };
    use futures_util::{SinkExt, StreamExt};
    use secrecy::SecretString;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn parses_a_full_update_and_blank_cells() {
        let text = r#"{"msg_type":"data:update","tag":"7","value":"1710000000000,10,100.5,80,30,90,48.1,11.5,-3,D,200,180,91"}"#;
        let Incoming::Update(sample) = parse_message(text).unwrap() else {
            panic!("update");
        };
        assert_eq!(sample.recorded_at_ms, 1_710_000_000_000);
        assert_eq!(sample.speed_mph, Some(10.0));
        assert_eq!(sample.odometer_mi, Some(100.5));
        assert_eq!(sample.soc, Some(80.0));
        assert_eq!(sample.elevation_ft, Some(30.0));
        assert_eq!(sample.est_lat, Some(48.1));
        assert_eq!(sample.est_lng, Some(11.5));
        assert_eq!(sample.power_kw, Some(-3.0));
        assert_eq!(sample.shift_state.as_deref(), Some("D"));
        assert_eq!(sample.range_mi, Some(200.0));
        assert_eq!(sample.est_range_mi, Some(180.0));
        assert_eq!(sample.heading, Some(91.0));

        let blank = r#"{"msg_type":"data:update","value":"1710000001,,,,,,,"}"#;
        let Incoming::Update(sample) = parse_message(blank).unwrap() else {
            panic!("blank");
        };
        assert_eq!(sample.recorded_at_ms, 1_710_000_001_000);
        assert_eq!(sample.speed_mph, None);
        assert_eq!(sample.shift_state, None);
        assert_eq!(sample.est_lat, None);

        assert!(parse_message("not-json").is_err());
        assert!(matches!(
            parse_message(r#"{"msg_type":"control:hello"}"#).unwrap(),
            Incoming::Hello
        ));
        let error = r#"{"msg_type":"data:error","error_type":"vehicle_error","value":"timeout"}"#;
        let Incoming::Error { error_type, value } = parse_message(error).unwrap() else {
            panic!("error");
        };
        assert_eq!(error_type, "vehicle_error");
        assert_eq!(
            classify_stream_error(&error_type, &value),
            FailureClass::Timeout
        );
    }

    #[test]
    fn classifies_errors_and_caps_backoff() {
        assert_eq!(
            classify_stream_error("vehicle_disconnected", "closed"),
            FailureClass::VehicleDisconnected
        );
        assert_eq!(
            classify_stream_error("client_error", "invalid token"),
            FailureClass::Auth
        );
        assert_eq!(
            classify_stream_error("vehicle_error", "asleep"),
            FailureClass::VehicleError
        );
        assert_eq!(
            classify_transport_message("HTTP error: 404 Not Found"),
            FailureClass::Unavailable
        );
        assert_eq!(
            classify_transport_message("401 Unauthorized"),
            FailureClass::Auth
        );
        assert_eq!(
            classify_transport_message("connection reset"),
            FailureClass::Transport
        );

        let first = plan_reconnect(FailureClass::Timeout, 1);
        assert_eq!(first.after, Duration::from_secs(10));
        assert!(!first.refresh_token);
        assert_eq!(
            plan_reconnect(FailureClass::Timeout, 2).after,
            Duration::from_secs(20)
        );
        assert_eq!(
            plan_reconnect(FailureClass::Timeout, 4).after,
            Duration::from_secs(60)
        );
        assert_eq!(
            plan_reconnect(FailureClass::Timeout, 10).after,
            Duration::from_secs(60)
        );
        assert_eq!(
            plan_reconnect(FailureClass::VehicleDisconnected, 3).after,
            Duration::from_secs(40)
        );
        assert_eq!(
            plan_reconnect(FailureClass::VehicleError, 6).after,
            Duration::from_secs(300)
        );
        assert_eq!(
            plan_reconnect(FailureClass::Transport, 1).after,
            Duration::from_secs(2)
        );
        assert_eq!(
            plan_reconnect(FailureClass::Transport, 8).after,
            Duration::from_secs(60)
        );
        let auth = plan_reconnect(FailureClass::Auth, 1);
        assert!(auth.refresh_token);
        assert_eq!(auth.after, Duration::from_secs(60));
        assert_eq!(
            plan_reconnect(FailureClass::Auth, 8).after,
            Duration::from_secs(900)
        );
        let gone = plan_reconnect(FailureClass::Unavailable, 1);
        assert!(!gone.refresh_token);
        assert_eq!(gone.after, Duration::from_secs(900));
        assert_eq!(
            plan_reconnect(FailureClass::Unavailable, 5).after,
            Duration::from_secs(900)
        );
    }

    #[test]
    fn subscribe_frame_carries_the_token_and_columns() {
        let frame = subscribe_frame("test-token", 42);
        let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(value["msg_type"], "data:subscribe_oauth");
        assert_eq!(value["token"], "test-token");
        assert_eq!(value["tag"], "42");
        assert_eq!(value["value"], STREAM_COLUMNS);
        assert!(!frame.contains("client_secret"));
    }

    #[tokio::test]
    async fn pump_emits_a_sample_then_reports_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            let Message::Text(text) = message else {
                panic!("expected text");
            };
            assert!(text.as_str().contains("data:subscribe_oauth"));
            assert!(text.as_str().contains("\"tag\":\"7\""));
            let update = r#"{"msg_type":"data:update","tag":"7","value":"1710000000000,10,100,80,30,90,48.1,11.5,-3,D,200,180,90"}"#;
            socket.send(Message::Text(update.into())).await.unwrap();
            let error =
                r#"{"msg_type":"data:error","error_type":"vehicle_disconnected","value":"closed"}"#;
            socket.send(Message::Text(error.into())).await.unwrap();
        });
        let (tx, mut rx) = mpsc::channel(4);
        let end = pump_stream(
            &format!("ws://{addr}/streaming/"),
            &SecretString::from("test-token".to_owned()),
            7,
            tx,
        )
        .await;
        let sample = rx.recv().await.unwrap();
        assert_eq!(sample.shift_state.as_deref(), Some("D"));
        assert_eq!(sample.est_lat, Some(48.1));
        assert_eq!(end, PumpEnd::Failed(FailureClass::VehicleDisconnected));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn plain_websocket_to_a_public_host_is_not_used() {
        let (tx, _rx) = mpsc::channel(1);
        let end = pump_stream(
            "ws://example.com/streaming/",
            &SecretString::from("test-token".to_owned()),
            1,
            tx,
        )
        .await;
        assert_eq!(end, PumpEnd::Failed(FailureClass::Unavailable));
    }
}
