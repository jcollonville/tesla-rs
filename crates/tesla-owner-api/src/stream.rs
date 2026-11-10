// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
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

/// One event from a streaming session.
///
/// `Sample` is the common variant. It stays unboxed so a GPS row is not allocated twice.
/// [`StreamEvent::Trace`] is observability only. It is not telemetry and it never carries the
/// OAuth token.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    /// The socket is up and the subscribe frame was written.
    Subscribed,
    /// A telemetry row.
    Sample(StreamSample),
    /// A frame or socket event the caller can show in a debug console.
    Trace(StreamTrace),
}

/// A websocket event that is not a telemetry row.
///
/// Error text is shortened before it is sent. A long opaque value is replaced so a token echoed
/// by the vehicle is not forwarded. The subscribe token is never placed in a trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamTrace {
    /// `control:hello` from the vehicle.
    Hello,
    /// Websocket ping from the vehicle.
    Ping,
    /// Websocket pong written in reply to a ping.
    Pong,
    /// Websocket pong received from the vehicle.
    PongReceived,
    /// The vehicle closed the socket. `detail` is the close code and reason.
    Closed {
        /// Short close description, for example `close code=1000 reason=bye`.
        detail: String,
    },
    /// `data:error`. `value` has already been shortened.
    Error {
        /// `error_type` from the frame, when present.
        error_type: String,
        /// Short, redacted `value` from the frame.
        value: String,
    },
    /// A text frame that could not be parsed.
    ParseFailed {
        /// Stable reason, for example `too many columns` or `value is not a string`.
        reason: &'static str,
        /// `msg_type` when the frame was JSON and the type was a short label.
        msg_type: Option<String>,
    },
    /// A text frame this client does not use.
    Ignored {
        /// `msg_type`, or `unknown` when the frame did not name one.
        msg_type: String,
    },
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

/// Frames seen on one websocket connection.
///
/// Counts are incoming frames only. A ping reply is not a ping. The subscribe frame is outgoing
/// and is not included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamStats {
    /// How long the websocket stayed open after the handshake.
    pub connected: Duration,
    /// Delay between the previous incoming frame and the end of the connection.
    ///
    /// [`None`] when the vehicle sent no frame.
    pub since_last_frame: Option<Duration>,
    /// `control:hello` frames.
    pub hello: u32,
    /// Websocket pings.
    pub ping: u32,
    /// `data:update` frames.
    pub updates: u32,
    /// `data:error` frames.
    pub errors: u32,
    /// Text frames that failed to parse.
    pub parse_failures: u32,
}

/// Why [`pump_stream`] stopped, plus what that connection carried.
///
/// [`Self::detail`] is a close code and reason, or a short IO error. It does not contain the
/// access token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamStop {
    /// Reconnect class.
    pub class: FailureClass,
    /// Close code and reason, or the underlying protocol or IO error.
    pub detail: String,
    /// Frames and timing for this connection.
    pub stats: StreamStats,
}

impl StreamStop {
    fn offline(class: FailureClass, detail: impl Into<String>) -> Self {
        Self {
            class,
            detail: detail.into(),
            stats: StreamStats {
                connected: Duration::ZERO,
                since_last_frame: None,
                hello: 0,
                ping: 0,
                updates: 0,
                errors: 0,
                parse_failures: 0,
            },
        }
    }
}

/// How a [`pump_stream`] call ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PumpEnd {
    /// The socket ended and the caller should reconnect.
    Failed(StreamStop),
}

impl PumpEnd {
    /// Reconnect class for this end.
    #[must_use]
    pub const fn class(&self) -> FailureClass {
        match self {
            Self::Failed(stop) => stop.class,
        }
    }

    /// Close or IO detail and the frame counts for this connection.
    #[must_use]
    pub const fn stop(&self) -> &StreamStop {
        match self {
            Self::Failed(stop) => stop,
        }
    }
}

/// Parses one text frame.
///
/// # Errors
///
/// Returns [`OwnerError::Malformed`] when the frame is not JSON or the update row has too many
/// columns. A short row is accepted. Missing cells are [`None`].
pub fn parse_message(text: &str) -> Result<Incoming, OwnerError> {
    read_frame(text)
        .map(|decoded| decoded.incoming)
        .map_err(|_| OwnerError::Malformed)
}

struct Decoded {
    incoming: Incoming,
    msg_type: Option<String>,
}

struct ParseFault {
    reason: &'static str,
    msg_type: Option<String>,
}

fn read_frame(text: &str) -> Result<Decoded, ParseFault> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            return Err(ParseFault {
                reason: "not json",
                msg_type: None,
            });
        }
    };
    if !value.is_object() {
        return Err(ParseFault {
            reason: "not a json object",
            msg_type: None,
        });
    }
    let msg_type = clean_msg_type(value.get("msg_type").and_then(Value::as_str));
    match msg_type.as_deref().unwrap_or("") {
        "data:update" => match value.get("value") {
            Some(Value::String(csv)) => match parse_csv(csv) {
                Ok(sample) => Ok(Decoded {
                    incoming: Incoming::Update(sample),
                    msg_type,
                }),
                Err(reason) => Err(ParseFault { reason, msg_type }),
            },
            Some(_) => Err(ParseFault {
                reason: "value is not a string",
                msg_type,
            }),
            None => Err(ParseFault {
                reason: "missing value",
                msg_type,
            }),
        },
        "data:error" => match value.get("value") {
            Some(value) if !value.is_string() => Err(ParseFault {
                reason: "value is not a string",
                msg_type,
            }),
            other => Ok(Decoded {
                incoming: Incoming::Error {
                    error_type: value
                        .get("error_type")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    value: other.and_then(Value::as_str).unwrap_or("").to_owned(),
                },
                msg_type,
            }),
        },
        "control:hello" => Ok(Decoded {
            incoming: Incoming::Hello,
            msg_type,
        }),
        _ => Ok(Decoded {
            incoming: Incoming::Ignore,
            msg_type,
        }),
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
/// [`StreamEvent::Subscribed`] is sent after the subscribe frame is written, before any GPS row.
/// Samples follow on the same channel. The access token is copied into the subscribe frame only.
/// Parse failures, hello, ping, and `data:error` are forwarded as [`StreamEvent::Trace`]. The
/// subscribe token is not included in those traces.
pub async fn pump_stream(
    url: &str,
    access_token: &SecretString,
    vehicle_id: i64,
    samples: mpsc::Sender<StreamEvent>,
) -> PumpEnd {
    if ensure_stream_url(url).is_err() {
        return PumpEnd::Failed(StreamStop::offline(
            FailureClass::Unavailable,
            "insecure stream url",
        ));
    }
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(64 * 1024);
    config.max_frame_size = Some(64 * 1024);
    let connection = connect_async_with_config(url, Some(config), true).await;
    let (socket, _) = match connection {
        Ok(pair) => pair,
        Err(err) => {
            let detail = scrub_error_value(&err.to_string());
            return PumpEnd::Failed(StreamStop::offline(
                classify_transport_message(&detail),
                detail,
            ));
        }
    };
    let (mut write, mut read) = socket.split();
    let mut frame = subscribe_frame(access_token.expose_secret(), vehicle_id);
    let send = write.send(Message::Text(frame.clone().into())).await;
    frame.zeroize();
    let mut live = Live::new();
    if send.is_err() {
        return live.finish(FailureClass::Transport, "subscribe send failed");
    }
    if !emit(&samples, StreamEvent::Subscribed).await {
        return live.finish(FailureClass::Transport, "event channel closed");
    }
    loop {
        let next = tokio::time::timeout(READ_TIMEOUT, read.next()).await;
        let message = match next {
            Err(_) => return live.finish(FailureClass::Timeout, "read timeout"),
            Ok(None) => return live.finish(FailureClass::Transport, "stream ended"),
            Ok(Some(Err(err))) => {
                let detail = scrub_error_value(&err.to_string());
                return live.finish(classify_transport_message(&detail), detail);
            }
            Ok(Some(Ok(message))) => message,
        };
        match message {
            Message::Text(text) => match read_frame(text.as_str()) {
                Ok(decoded) => match decoded.incoming {
                    Incoming::Update(sample) => {
                        live.updates = live.updates.saturating_add(1);
                        live.saw_frame();
                        if !emit(&samples, StreamEvent::Sample(sample)).await {
                            return live.finish(FailureClass::Transport, "event channel closed");
                        }
                    }
                    Incoming::Error { error_type, value } => {
                        let class = classify_stream_error(&error_type, &value);
                        let value = scrub_error_value(&value);
                        live.errors = live.errors.saturating_add(1);
                        live.saw_frame();
                        let detail = format!("data:error type={error_type} value={value}");
                        if !emit(
                            &samples,
                            StreamEvent::Trace(StreamTrace::Error { error_type, value }),
                        )
                        .await
                        {
                            return live.finish(FailureClass::Transport, "event channel closed");
                        }
                        return live.finish(class, detail);
                    }
                    Incoming::Hello => {
                        live.hello = live.hello.saturating_add(1);
                        live.saw_frame();
                        if !emit(&samples, StreamEvent::Trace(StreamTrace::Hello)).await {
                            return live.finish(FailureClass::Transport, "event channel closed");
                        }
                    }
                    Incoming::Ignore => {
                        live.saw_frame();
                        let msg_type = decoded.msg_type.unwrap_or_else(|| "unknown".to_owned());
                        if !emit(
                            &samples,
                            StreamEvent::Trace(StreamTrace::Ignored { msg_type }),
                        )
                        .await
                        {
                            return live.finish(FailureClass::Transport, "event channel closed");
                        }
                    }
                },
                Err(fault) => {
                    live.parse_failures = live.parse_failures.saturating_add(1);
                    live.saw_frame();
                    if !emit(
                        &samples,
                        StreamEvent::Trace(StreamTrace::ParseFailed {
                            reason: fault.reason,
                            msg_type: fault.msg_type,
                        }),
                    )
                    .await
                    {
                        return live.finish(FailureClass::Transport, "event channel closed");
                    }
                }
            },
            Message::Ping(payload) => {
                live.ping = live.ping.saturating_add(1);
                live.saw_frame();
                if !emit(&samples, StreamEvent::Trace(StreamTrace::Ping)).await {
                    return live.finish(FailureClass::Transport, "event channel closed");
                }
                if write.send(Message::Pong(payload)).await.is_err() {
                    return live.finish(FailureClass::Transport, "pong send failed");
                }
                if !emit(&samples, StreamEvent::Trace(StreamTrace::Pong)).await {
                    return live.finish(FailureClass::Transport, "event channel closed");
                }
            }
            Message::Close(frame) => {
                let detail = close_detail(frame);
                let _ = emit(
                    &samples,
                    StreamEvent::Trace(StreamTrace::Closed {
                        detail: detail.clone(),
                    }),
                )
                .await;
                return live.finish(FailureClass::Transport, detail);
            }
            Message::Pong(_) => {
                live.saw_frame();
                if !emit(&samples, StreamEvent::Trace(StreamTrace::PongReceived)).await {
                    return live.finish(FailureClass::Transport, "event channel closed");
                }
            }
            Message::Binary(_) | Message::Frame(_) => live.saw_frame(),
        }
    }
}

struct Live {
    started: std::time::Instant,
    last_frame: Option<std::time::Instant>,
    hello: u32,
    ping: u32,
    updates: u32,
    errors: u32,
    parse_failures: u32,
}

impl Live {
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            last_frame: None,
            hello: 0,
            ping: 0,
            updates: 0,
            errors: 0,
            parse_failures: 0,
        }
    }

    fn saw_frame(&mut self) {
        self.last_frame = Some(std::time::Instant::now());
    }

    fn finish(&self, class: FailureClass, detail: impl Into<String>) -> PumpEnd {
        PumpEnd::Failed(StreamStop {
            class,
            detail: detail.into(),
            stats: StreamStats {
                connected: self.started.elapsed(),
                since_last_frame: self.last_frame.map(|at| at.elapsed()),
                hello: self.hello,
                ping: self.ping,
                updates: self.updates,
                errors: self.errors,
                parse_failures: self.parse_failures,
            },
        })
    }
}

/// Keepalives are dropped when the channel is full. Samples, errors, and parse failures wait.
async fn emit(samples: &mpsc::Sender<StreamEvent>, event: StreamEvent) -> bool {
    let lossy = matches!(
        event,
        StreamEvent::Trace(
            StreamTrace::Hello | StreamTrace::Ping | StreamTrace::Pong | StreamTrace::PongReceived
        )
    );
    if lossy {
        return !matches!(
            samples.try_send(event),
            Err(mpsc::error::TrySendError::Closed(_))
        );
    }
    samples.send(event).await.is_ok()
}

fn parse_csv(csv: &str) -> Result<StreamSample, &'static str> {
    let mut cells = csv.split(',');
    let stamp = cells.next().ok_or("missing timestamp")?;
    let recorded_at_ms = parse_timestamp(stamp).ok_or("bad timestamp")?;
    let mut fields = Vec::with_capacity(COLUMN_COUNT);
    for cell in cells {
        fields.push(cell);
        if fields.len() > COLUMN_COUNT {
            return Err("too many columns");
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

fn clean_msg_type(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if value.len() > 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-'))
    {
        return Some("unknown".to_owned());
    }
    Some(value.to_owned())
}

fn close_detail(frame: Option<CloseFrame>) -> String {
    let Some(frame) = frame else {
        return "close without a code".to_owned();
    };
    let code: u16 = frame.code.into();
    let reason = scrub_error_value(frame.reason.as_ref());
    if reason.is_empty() {
        format!("close code={code}")
    } else {
        format!("close code={code} reason={reason}")
    }
}

fn scrub_error_value(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if let Some(at) = lower.find("bearer") {
        let after = at + "bearer".len();
        if value.len() > after && value.as_bytes()[after].is_ascii_whitespace() {
            return "Bearer ***".to_owned();
        }
    }
    if value.len() >= 24
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return "***".to_owned();
    }
    value.chars().take(180).collect()
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
        FailureClass, Incoming, STREAM_COLUMNS, StreamEvent, StreamTrace, classify_stream_error,
        classify_transport_message, parse_message, plan_reconnect, pump_stream, subscribe_frame,
    };
    use futures_util::{SinkExt, StreamExt};
    use secrecy::SecretString;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;

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
        assert!(parse_message(r#"{"msg_type":"data:update","value":1}"#).is_err());
        assert!(
            parse_message(
                r#"{"msg_type":"data:update","value":"1,2,3,4,5,6,7,8,9,10,11,12,13,14"}"#
            )
            .is_err()
        );
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
        let (tx, mut rx) = mpsc::channel(8);
        let end = pump_stream(
            &format!("ws://{addr}/streaming/"),
            &SecretString::from("test-token".to_owned()),
            7,
            tx,
        )
        .await;
        assert_eq!(rx.recv().await.unwrap(), StreamEvent::Subscribed);
        let StreamEvent::Sample(sample) = rx.recv().await.unwrap() else {
            panic!("sample");
        };
        assert_eq!(sample.shift_state.as_deref(), Some("D"));
        assert_eq!(sample.est_lat, Some(48.1));
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Error {
                error_type: "vehicle_disconnected".to_owned(),
                value: "closed".to_owned(),
            })
        );
        assert_eq!(end.class(), FailureClass::VehicleDisconnected);
        assert_eq!(end.stop().stats.updates, 1);
        assert_eq!(end.stop().stats.errors, 1);
        assert!(
            end.stop().detail.contains("vehicle_disconnected"),
            "{end:?}"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pump_is_subscribed_on_hello_without_a_gps_sample() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            let Message::Text(text) = message else {
                panic!("expected text");
            };
            let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
            assert_eq!(value["msg_type"], "data:subscribe_oauth");
            assert_eq!(value["tag"], "7");
            socket
                .send(Message::Text(r#"{"msg_type":"control:hello"}"#.into()))
                .await
                .unwrap();
            socket.send(Message::Ping(vec![1].into())).await.unwrap();
            socket.send(Message::Close(None)).await.unwrap();
        });
        let (tx, mut rx) = mpsc::channel(8);
        let end = pump_stream(
            &format!("ws://{addr}/streaming/"),
            &SecretString::from("test-token".to_owned()),
            7,
            tx,
        )
        .await;
        assert_eq!(rx.recv().await.unwrap(), StreamEvent::Subscribed);
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Hello)
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Ping)
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Pong)
        );
        match rx.recv().await.unwrap() {
            StreamEvent::Trace(StreamTrace::Closed { detail }) => {
                assert!(detail.contains("close"), "{detail}");
            }
            other => panic!("expected close, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(end.class(), FailureClass::Transport);
        assert_eq!(end.stop().stats.hello, 1);
        assert_eq!(end.stop().stats.ping, 1);
        assert!(end.stop().stats.since_last_frame.is_some());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pump_reports_an_auth_error_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            let error =
                r#"{"msg_type":"data:error","error_type":"client_error","value":"invalid token"}"#;
            socket.send(Message::Text(error.into())).await.unwrap();
        });
        let (tx, mut rx) = mpsc::channel(8);
        let end = pump_stream(
            &format!("ws://{addr}/streaming/"),
            &SecretString::from("test-token".to_owned()),
            7,
            tx,
        )
        .await;
        assert_eq!(rx.recv().await.unwrap(), StreamEvent::Subscribed);
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Error {
                error_type: "client_error".to_owned(),
                value: "invalid token".to_owned(),
            })
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(end.class(), FailureClass::Auth);
        assert_eq!(end.stop().stats.errors, 1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pump_reports_frames_that_fail_to_parse() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(
                    r#"{"msg_type":"data:update","value":1}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"msg_type":"data:update","value":"1,2,3,4,5,6,7,8,9,10,11,12,13,14"}"#
                        .into(),
                ))
                .await
                .unwrap();
            socket.send(Message::Text("not-json".into())).await.unwrap();
            socket.send(Message::Close(None)).await.unwrap();
        });
        let (tx, mut rx) = mpsc::channel(8);
        let end = pump_stream(
            &format!("ws://{addr}/streaming/"),
            &SecretString::from("test-token".to_owned()),
            7,
            tx,
        )
        .await;
        assert_eq!(rx.recv().await.unwrap(), StreamEvent::Subscribed);
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::ParseFailed {
                reason: "value is not a string",
                msg_type: Some("data:update".to_owned()),
            })
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::ParseFailed {
                reason: "too many columns",
                msg_type: Some("data:update".to_owned()),
            })
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::ParseFailed {
                reason: "not json",
                msg_type: None,
            })
        );
        match rx.recv().await.unwrap() {
            StreamEvent::Trace(StreamTrace::Closed { .. }) => {}
            other => panic!("expected close, got {other:?}"),
        }
        assert_eq!(end.class(), FailureClass::Transport);
        assert_eq!(end.stop().stats.parse_failures, 3);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pump_records_the_close_code_reason_and_frame_counts() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(r#"{"msg_type":"control:hello"}"#.into()))
                .await
                .unwrap();
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: 1000.into(),
                    reason: "bye".into(),
                })))
                .await
                .unwrap();
        });
        let (tx, mut rx) = mpsc::channel(8);
        let end = pump_stream(
            &format!("ws://{addr}/streaming/"),
            &SecretString::from("test-token".to_owned()),
            7,
            tx,
        )
        .await;
        assert_eq!(rx.recv().await.unwrap(), StreamEvent::Subscribed);
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Hello)
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            StreamEvent::Trace(StreamTrace::Closed {
                detail: "close code=1000 reason=bye".to_owned(),
            })
        );
        let stop = end.stop();
        assert_eq!(stop.class, FailureClass::Transport);
        assert_eq!(stop.detail, "close code=1000 reason=bye");
        assert_eq!(stop.stats.hello, 1);
        assert_eq!(stop.stats.ping, 0);
        assert_eq!(stop.stats.updates, 0);
        assert_eq!(stop.stats.errors, 0);
        assert_eq!(stop.stats.parse_failures, 0);
        assert!(stop.stats.connected > Duration::ZERO);
        assert!(stop.stats.since_last_frame.is_some());
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
        assert_eq!(end.class(), FailureClass::Unavailable);
        assert_eq!(end.stop().stats.connected, Duration::ZERO);
    }
}
