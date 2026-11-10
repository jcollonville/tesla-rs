// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Bounded in-memory copy of recent tracing events for the status UI.
//!
//! Lines are redacted before they are stored. The buffer does not read Docker
//! or host log files.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use secrecy::ExposeSecret;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use crate::config::Config;
use crate::error::redact_secrets;

const CAPACITY: usize = 400;
const MAX_CHARS: usize = 1_500;

const JSON_KEYS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "code_verifier",
    "client_secret",
    "password",
    "token",
    "code",
];

const ASSIGNMENT_KEYS: &[&str] = &[
    "code_verifier",
    "access_token",
    "refresh_token",
    "id_token",
    "client_secret",
    "password",
    "token",
    "code",
];

/// One stored log line. The text has already been redacted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogLine {
    pub at: DateTime<Utc>,
    pub level: String,
    pub text: String,
}

#[derive(Clone)]
pub(crate) struct LogBuffer {
    inner: Arc<Mutex<BufferState>>,
}

struct BufferState {
    lines: VecDeque<LogLine>,
    secrets: Vec<String>,
    cap: usize,
}

impl LogBuffer {
    pub(crate) fn new(cap: usize) -> Self {
        let cap = cap.clamp(1, CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(BufferState {
                lines: VecDeque::with_capacity(cap),
                secrets: Vec::new(),
                cap,
            })),
        }
    }

    pub(crate) fn install_config(&self, config: &Config) {
        let mut secrets = Vec::new();
        push_secret(&mut secrets, config.ui_password.expose_secret());
        push_secret(&mut secrets, config.client_secret.expose_secret());
        if let Some(key) = &config.token_encryption_key {
            push_secret(&mut secrets, key);
        }
        if let Some(password) = &config.mqtt_password {
            push_secret(&mut secrets, password.expose_secret());
        }
        if let Some(password) = database_password(&config.database_url) {
            push_secret(&mut secrets, &password);
        }
        let mut guard = lock(&self.inner);
        guard.secrets = secrets;
        let secrets = guard.secrets.clone();
        for line in &mut guard.lines {
            line.text = redact_line(&line.text, &secrets);
        }
    }

    pub(crate) fn record(&self, level: &str, text: &str) {
        let mut guard = lock(&self.inner);
        let text = truncate(redact_line(text, &guard.secrets), MAX_CHARS);
        if guard.lines.len() >= guard.cap {
            guard.lines.pop_front();
        }
        guard.lines.push_back(LogLine {
            at: Utc::now(),
            level: level.to_owned(),
            text,
        });
    }

    pub(crate) fn lines(&self) -> Vec<LogLine> {
        let guard = lock(&self.inner);
        guard
            .lines
            .iter()
            .map(|line| LogLine {
                at: line.at,
                level: line.level.clone(),
                text: redact_line(&line.text, &guard.secrets),
            })
            .collect()
    }
}

pub(crate) struct CaptureLayer {
    buffer: LogBuffer,
}

impl CaptureLayer {
    pub(crate) fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.buffer
            .record(event.metadata().level().as_str(), &visitor.finish());
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: String,
}

impl FieldVisitor {
    fn push(&mut self, name: &str, value: &str) {
        if name == "message" {
            strip_debug_quotes(value).clone_into(&mut self.message);
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(name);
        self.fields.push('=');
        self.fields.push_str(value);
    }

    fn finish(self) -> String {
        match (self.message.is_empty(), self.fields.is_empty()) {
            (false, false) => format!("{} {}", self.message, self.fields),
            (false, true) => self.message,
            (true, false) => self.fields,
            (true, true) => String::new(),
        }
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.push(field.name(), &format!("{value:?}"));
    }
}

pub(crate) fn redact_line(text: &str, secrets: &[String]) -> String {
    let mut text = redact_secrets(text);
    text = redact_subscribe_frames(&text);
    text = redact_json_secrets(&text);
    text = redact_authorization(&text);
    text = redact_assignments(&text);
    let mut ordered: Vec<&String> = secrets.iter().filter(|secret| secret.len() >= 4).collect();
    ordered.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    for secret in ordered {
        text = replace_secret(&text, secret);
    }
    text
}

fn push_secret(secrets: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.len() >= 4 {
        secrets.push(value.to_owned());
    }
}

fn database_password(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let password = parsed.password()?.trim();
    (password.len() >= 4).then(|| password.to_owned())
}

fn lock(mutex: &Mutex<BufferState>) -> std::sync::MutexGuard<'_, BufferState> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn truncate(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn strip_debug_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn redact_subscribe_frames(text: &str) -> String {
    const NEEDLE: &str = "data:subscribe_oauth";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(NEEDLE) {
        let before = &rest[..at];
        if let Some(start) = before.rfind('{')
            && let Some(end) = matching_brace(rest, start)
        {
            out.push_str(&before[..start]);
            out.push_str("[subscribe frame redacted]");
            rest = &rest[end..];
            continue;
        }
        out.push_str(before);
        out.push_str("[subscribe frame redacted]");
        rest = &rest[at + NEEDLE.len()..];
    }
    out.push_str(rest);
    out
}

fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0_i32;
    let mut index = open;
    let mut in_string = false;
    let mut escape = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(index + 1);
            }
        }
        index += 1;
    }
    None
}

fn redact_json_secrets(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"'
            && let Some((value_quote, value_end)) = json_secret_at(text, index)
        {
            out.push_str(&text[index..value_quote]);
            out.push_str("\"***\"");
            index = value_end;
            continue;
        }
        let ch = text[index..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn json_secret_at(text: &str, quote: usize) -> Option<(usize, usize)> {
    let rest = &text[quote + 1..];
    let key = JSON_KEYS
        .iter()
        .find(|key| rest.starts_with(*key) && rest[key.len()..].starts_with('"'))?;
    let mut index = quote + 1 + key.len() + 1;
    let bytes = text.as_bytes();
    index = skip_ascii_space(bytes, index);
    if bytes.get(index) != Some(&b':') {
        return None;
    }
    index = skip_ascii_space(bytes, index + 1);
    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    let value_quote = index;
    Some((value_quote, end_of_json_string(text, value_quote)?))
}

fn redact_authorization(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if word_at(&lower, index, "authorization") {
            let after = index + "authorization".len();
            let mut cursor = skip_ascii_space(bytes, after);
            if bytes
                .get(cursor)
                .is_some_and(|byte| *byte == b':' || *byte == b'=')
            {
                cursor = skip_ascii_space(bytes, cursor + 1);
                let end = header_value_end(text, cursor);
                out.push_str(&text[index..after]);
                out.push_str(": ***");
                index = end;
                continue;
            }
        }
        if word_at(&lower, index, "bearer") {
            let after = index + "bearer".len();
            if bytes.get(after).is_some_and(u8::is_ascii_whitespace) {
                let token = skip_ascii_space(bytes, after);
                let end = skip_token(bytes, token);
                if end > token {
                    out.push_str("Bearer ***");
                    index = end;
                    continue;
                }
            }
        }
        let ch = text[index..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn redact_assignments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some((key_len, value_end)) = assignment_at(text, index) {
            out.push_str(&text[index..index + key_len]);
            out.push_str("=***");
            index = value_end;
            continue;
        }
        let ch = text[index..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn assignment_at(text: &str, index: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    if index > 0 && is_ident(bytes[index - 1]) {
        return None;
    }
    let rest = &text[index..];
    let key = ASSIGNMENT_KEYS.iter().find(|key| rest.starts_with(*key))?;
    let after = index + key.len();
    if bytes.get(after) != Some(&b'=') {
        return None;
    }
    Some((key.len(), assignment_value_end(text, after + 1)))
}

fn assignment_value_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    if bytes.get(start) == Some(&b'"') {
        return end_of_json_string(text, start).unwrap_or(start);
    }
    skip_token(bytes, start)
}

fn header_value_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    if bytes.get(start) == Some(&b'"') {
        return end_of_json_string(text, start).unwrap_or(start);
    }
    let word_end = skip_token(bytes, start);
    if text[start..word_end].eq_ignore_ascii_case("bearer") {
        return skip_token(bytes, skip_ascii_space(bytes, word_end));
    }
    word_end
}

fn end_of_json_string(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'"') {
        return None;
    }
    let mut index = open + 1;
    let mut escape = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escape {
            escape = false;
        } else if byte == b'\\' {
            escape = true;
        } else if byte == b'"' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn skip_ascii_space(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn skip_token(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len()
        && !bytes[index].is_ascii_whitespace()
        && bytes[index] != b'&'
        && bytes[index] != b';'
    {
        index += 1;
    }
    index
}

fn word_at(lower: &str, index: usize, word: &str) -> bool {
    let bytes = lower.as_bytes();
    if !lower[index..].starts_with(word) {
        return false;
    }
    if index > 0 && is_ident(bytes[index - 1]) {
        return false;
    }
    let after = index + word.len();
    after >= bytes.len() || !is_ident(bytes[after])
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn replace_secret(text: &str, secret: &str) -> String {
    if secret.len() < 4 {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(secret) {
        let after = index + secret.len();
        let before_ok = index == 0 || !is_ident(rest.as_bytes()[index - 1]);
        let after_ok = after >= rest.len() || !is_ident(rest.as_bytes()[after]);
        if secret.len() >= 16 || (before_ok && after_ok) {
            out.push_str(&rest[..index]);
            out.push_str("***");
            rest = &rest[after..];
        } else {
            out.push_str(&rest[..=index]);
            rest = &rest[index + 1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{LogBuffer, redact_line};
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const ACCESS: &str = "access-secret-value";
    const REFRESH: &str = "refresh-secret-value";
    const CODE: &str = "auth-code-value";
    const VERIFIER: &str = "pkce-verifier-value";
    const BEARER: &str = "bearer-secret-value";
    const PASSWORD: &str = "test-password";
    const KEY: &str = "test-key-value";
    const SUBSCRIBE: &str = "subscribe-token-value";

    fn sample() -> String {
        format!(
            "owner stream is up access_token={ACCESS} refresh_token={REFRESH} code={CODE} \
             code_verifier={VERIFIER} Authorization: Bearer {BEARER} \
             {{\"access_token\":\"{ACCESS}\",\"password\":\"{PASSWORD}\"}} \
             {{\"msg_type\":\"data:subscribe_oauth\",\"token\":\"{SUBSCRIBE}\",\"value\":\"speed\",\"tag\":\"1\"}} \
             postgres://tesla:s3cret@db:5432/tesla_logger status_code=412 class=timeout"
        )
    }

    #[test]
    fn redacts_tokens_passwords_and_subscribe_frames() {
        let secrets = [PASSWORD.to_owned(), KEY.to_owned()];
        let line = format!("{} key {KEY}", sample());
        let redacted = redact_line(&line, &secrets);
        for secret in [
            ACCESS, REFRESH, CODE, VERIFIER, BEARER, PASSWORD, KEY, SUBSCRIBE, "s3cret",
        ] {
            assert!(!redacted.contains(secret), "{secret} in {redacted}");
        }
        assert!(redacted.contains("owner stream is up"));
        assert!(redacted.contains("status_code=412"));
        assert!(redacted.contains("class=timeout"));
        assert!(redacted.contains("[subscribe frame redacted]"));
        assert!(redacted.contains("postgres://tesla:***@db:5432/tesla_logger"));
    }

    #[test]
    fn buffer_keeps_a_bounded_redacted_history() {
        let buffer = LogBuffer::new(2);
        buffer.record("INFO", "owner stream is up");
        buffer.record("WARN", &format!("access_token={ACCESS}"));
        buffer.record("ERROR", "third");
        let lines = buffer.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "third");
        assert!(!lines.iter().any(|line| line.text.contains(ACCESS)));
        assert!(lines.iter().any(|line| line.level == "WARN"));
    }

    #[test]
    fn tracing_fields_are_captured_without_the_secret() {
        use tracing_subscriber::layer::SubscriberExt;

        let buffer = LogBuffer::new(8);
        let layer = super::CaptureLayer::new(buffer.clone());
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(access_token = ACCESS, "signed in");
        });
        let text = buffer
            .lines()
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("signed in"), "{text}");
        assert!(!text.contains(ACCESS), "{text}");
    }

    #[tokio::test]
    async fn logs_page_requires_a_session_and_hides_secrets() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .expect("lazy pool");
        let state = crate::app::test_state(pool).await;
        let line = format!(
            "owner stream is up access_token={ACCESS} refresh_token={REFRESH} code={CODE} \
             code_verifier={VERIFIER} Authorization: Bearer {BEARER} \
             {{\"msg_type\":\"data:subscribe_oauth\",\"token\":\"{SUBSCRIBE}\",\"tag\":\"1\"}} \
             ui test-password and test-key <note>"
        );
        state.inner.logs.record("INFO", &line);

        let denied = call(&state, "GET", "/logs", None, None).await;
        assert_eq!(denied.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(denied.headers().get("location").unwrap(), "/");
        let denied_body = body_text(denied).await;
        assert!(!denied_body.contains("owner stream is up"));
        assert_secret_free(&denied_body);

        let login = call(&state, "GET", "/", None, None).await;
        let csrf = header_cookie(login.headers(), "tl_csrf").expect("csrf");
        let posted = call(
            &state,
            "POST",
            "/",
            Some(&format!("tl_csrf={csrf}")),
            Some(&format!("password=test-password&csrf={csrf}")),
        )
        .await;
        assert!(posted.status().is_redirection());
        let session = header_cookie(posted.headers(), "tl_session").expect("session");
        let page = call(
            &state,
            "GET",
            "/logs",
            Some(&format!("tl_session={session}")),
            None,
        )
        .await;
        assert_eq!(page.status(), axum::http::StatusCode::OK);
        assert_eq!(page.headers().get("cache-control").unwrap(), "no-store");
        let html = body_text(page).await;
        assert!(html.contains("owner stream is up"), "{html}");
        assert!(html.contains("href=\"/auth/owner\""));
        assert!(html.contains("href=\"/\""));
        assert!(html.contains("&lt;note&gt;"));
        assert!(!html.contains("<note>"));
        assert_secret_free(&html);

        let owner = call(
            &state,
            "GET",
            "/auth/owner",
            Some(&format!("tl_session={session}")),
            None,
        )
        .await;
        assert_eq!(owner.status(), axum::http::StatusCode::OK);
        let owner_html = body_text(owner).await;
        assert!(owner_html.contains("href=\"/logs\""));
        assert_secret_free(&owner_html);
    }

    fn assert_secret_free(text: &str) {
        for secret in [
            ACCESS,
            REFRESH,
            CODE,
            VERIFIER,
            BEARER,
            SUBSCRIBE,
            "test-password",
            "test-key",
        ] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }

    async fn call(
        state: &crate::app::AppState,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        form: Option<&str>,
    ) -> axum::response::Response {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header(axum::http::header::COOKIE, cookie);
        }
        if form.is_some() {
            builder = builder.header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            );
        }
        let request = builder
            .body(Body::from(form.unwrap_or("").to_owned()))
            .unwrap();
        crate::http::router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
    }

    fn header_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
        headers
            .get_all(axum::http::header::SET_COOKIE)
            .iter()
            .find_map(|value| {
                let text = value.to_str().ok()?;
                let pair = text.split(';').next()?;
                let (key, value) = pair.split_once('=')?;
                (key.trim() == name).then(|| value.trim().to_owned())
            })
    }

    async fn body_text(response: axum::response::Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
