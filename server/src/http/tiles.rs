// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Same-origin proxy for the drive and charge basemap.
//!
//! The browser asks only this process for tiles (`/tiles/p/…`). The process
//! fetches OpenFreeMap, caches the body, and rewrites style JSON so every
//! later URL stays on this origin. Paths that are not part of the two styles
//! we ship are rejected, so the route is not an open proxy. The request log
//! skips these paths: a tile coordinate is a place.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::sync::{Mutex, Semaphore};

use crate::app::AppState;
use crate::http::ui_session;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MEMORY_CAP: usize = 128;
const DISK_CAP: usize = 4_000;
const TILE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const STYLE_TTL: Duration = Duration::from_secs(6 * 3600);
const MISS_TTL: Duration = Duration::from_secs(600);
const USER_AGENT: &str = concat!(
    "tesla-logger/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/jcollonville/tesla-rs)"
);

enum Slot {
    Ready {
        bytes: Arc<Vec<u8>>,
        content_type: &'static str,
        until: Instant,
    },
    Missing(Instant),
}

/// Cached basemap client. An empty upstream disables fetches.
pub(crate) struct Basemap {
    base: String,
    client: reqwest::Client,
    dir: Option<PathBuf>,
    memory: Mutex<Memory>,
    slots: Semaphore,
}

struct Memory {
    slots: HashMap<String, Slot>,
    order: VecDeque<String>,
}

impl Basemap {
    pub(crate) fn from_config(config: &crate::config::Config) -> Self {
        let Some(base) = config.tile_upstream.clone() else {
            return Self::disabled();
        };
        let dir = config
            .token_store_path
            .parent()
            .map(|parent| parent.join("map-tiles"));
        Self::new(base, dir)
    }

    pub(crate) fn disabled() -> Self {
        Self::new(String::new(), None)
    }

    pub(crate) fn new(mut base: String, dir: Option<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_default();
        while base.ends_with('/') {
            base.pop();
        }
        Self {
            base,
            client,
            dir,
            memory: Mutex::new(Memory {
                slots: HashMap::new(),
                order: VecDeque::new(),
            }),
            slots: Semaphore::new(4),
        }
    }

    async fn fetch(&self, raw_path: &str) -> Result<Tile, TileError> {
        let Some(path) = normalize(raw_path) else {
            return Err(TileError::Denied);
        };
        if !allowed(&path) || self.base.is_empty() {
            return Err(TileError::Denied);
        }
        if let Some(tile) = self.remembered(&path).await {
            return tile;
        }
        if let Some(tile) = self.read_disk(&path).await {
            self.remember_ready(&path, tile.clone()).await;
            return Ok(tile);
        }
        let _permit = self
            .slots
            .acquire()
            .await
            .map_err(|_| TileError::Upstream)?;
        if let Some(tile) = self.remembered(&path).await {
            return tile;
        }
        match self.fetch_upstream(&path).await {
            Ok(tile) => {
                self.write_disk(&path, &tile).await;
                self.remember_ready(&path, tile.clone()).await;
                Ok(tile)
            }
            Err(TileError::Missing) => {
                self.remember_missing(&path).await;
                Err(TileError::Missing)
            }
            Err(err) => Err(err),
        }
    }

    async fn remembered(&self, path: &str) -> Option<Result<Tile, TileError>> {
        let mut memory = self.memory.lock().await;
        let now = Instant::now();
        match memory.slots.get(path) {
            Some(Slot::Ready {
                bytes,
                content_type,
                until,
            }) if *until > now => Some(Ok(Tile {
                bytes: Arc::clone(bytes),
                content_type,
            })),
            Some(Slot::Missing(until)) if *until > now => Some(Err(TileError::Missing)),
            Some(_) => {
                memory.slots.remove(path);
                None
            }
            None => None,
        }
    }

    async fn remember_ready(&self, path: &str, tile: Tile) {
        let mut memory = self.memory.lock().await;
        if memory.slots.len() >= MEMORY_CAP
            && let Some(old) = memory.order.pop_front()
        {
            memory.slots.remove(&old);
        }
        memory.order.push_back(path.to_owned());
        memory.slots.insert(
            path.to_owned(),
            Slot::Ready {
                bytes: tile.bytes,
                content_type: tile.content_type,
                until: Instant::now() + ttl(path),
            },
        );
    }

    async fn remember_missing(&self, path: &str) {
        let mut memory = self.memory.lock().await;
        memory
            .slots
            .insert(path.to_owned(), Slot::Missing(Instant::now() + MISS_TTL));
    }

    async fn fetch_upstream(&self, path: &str) -> Result<Tile, TileError> {
        let url = upstream_url(&self.base, path).ok_or(TileError::Denied)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| TileError::Upstream)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(TileError::Missing);
        }
        if !response.status().is_success() {
            return Err(TileError::Upstream);
        }
        let kind = kind_of(path);
        let announced = response.content_length().unwrap_or(0);
        if announced > MAX_BYTES as u64 {
            return Err(TileError::Upstream);
        }
        let bytes = response.bytes().await.map_err(|_| TileError::Upstream)?;
        if bytes.len() > MAX_BYTES {
            return Err(TileError::Upstream);
        }
        let body = match kind {
            Kind::Json => rewrite(&bytes, &self.base).ok_or(TileError::Upstream)?,
            Kind::Png if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => bytes.to_vec(),
            Kind::Pbf if !bytes.is_empty() => bytes.to_vec(),
            Kind::Png | Kind::Pbf => return Err(TileError::Upstream),
        };
        Ok(Tile {
            bytes: Arc::new(body),
            content_type: kind.content_type(),
        })
    }

    async fn read_disk(&self, path: &str) -> Option<Tile> {
        let dir = self.dir.as_ref()?;
        let hash = crate::crypto::sha256_hex(path);
        let file = dir.join(&hash);
        let meta = tokio::fs::read_to_string(dir.join(format!("{hash}.meta")))
            .await
            .ok()?;
        let mut lines = meta.lines();
        let content_type = lines.next()?.trim();
        let expiry: u64 = lines.next()?.trim().parse().ok()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        if expiry < now {
            let _ = tokio::fs::remove_file(&file).await;
            let _ = tokio::fs::remove_file(dir.join(format!("{hash}.meta"))).await;
            return None;
        }
        let bytes = tokio::fs::read(&file).await.ok()?;
        let content_type = match content_type {
            "application/json" => "application/json",
            "image/png" => "image/png",
            "application/vnd.mapbox-vector-tile" => "application/vnd.mapbox-vector-tile",
            _ => return None,
        };
        Some(Tile {
            bytes: Arc::new(bytes),
            content_type,
        })
    }

    async fn write_disk(&self, path: &str, tile: &Tile) {
        let Some(dir) = &self.dir else {
            return;
        };
        if tokio::fs::create_dir_all(dir).await.is_err() {
            return;
        }
        let hash = crate::crypto::sha256_hex(path);
        let file = dir.join(&hash);
        let tmp = dir.join(format!("{hash}.tmp"));
        if tokio::fs::write(&tmp, tile.bytes.as_slice()).await.is_err() {
            return;
        }
        if tokio::fs::rename(&tmp, &file).await.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
            return;
        }
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                since.as_secs().saturating_add(ttl(path).as_secs())
            });
        let meta = format!("{}\n{expiry}\n", tile.content_type);
        let _ = tokio::fs::write(dir.join(format!("{hash}.meta")), meta).await;
        trim_disk(dir).await;
    }
}

#[derive(Clone)]
pub(crate) struct Tile {
    bytes: Arc<Vec<u8>>,
    content_type: &'static str,
}

#[derive(Debug)]
enum TileError {
    Denied,
    Missing,
    Upstream,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Json,
    Png,
    Pbf,
}

impl Kind {
    fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Png => "image/png",
            Self::Pbf => "application/vnd.mapbox-vector-tile",
        }
    }
}

#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn kind_of(path: &str) -> Kind {
    if path.ends_with(".png") {
        Kind::Png
    } else if path.ends_with(".pbf") {
        Kind::Pbf
    } else {
        Kind::Json
    }
}

fn ttl(path: &str) -> Duration {
    if path.starts_with("styles/") || path == "planet" {
        STYLE_TTL
    } else {
        TILE_TTL
    }
}

/// Tile paths name a place. The access log must not record them.
pub(crate) fn is_quiet(path: &str) -> bool {
    path.starts_with("/tiles/")
}

pub(super) async fn proxy(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(path): AxumPath<String>,
) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match app.inner.basemap.fetch(&path).await {
        Ok(tile) => {
            let cache = if kind_of(&path) == Kind::Json {
                "private, max-age=3600"
            } else {
                "private, max-age=604800"
            };
            (
                [
                    (header::CONTENT_TYPE, tile.content_type),
                    (header::CACHE_CONTROL, cache),
                ],
                tile.bytes.as_slice().to_vec(),
            )
                .into_response()
        }
        Err(TileError::Missing | TileError::Denied) => StatusCode::NOT_FOUND.into_response(),
        Err(TileError::Upstream) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

fn normalize(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 240 || raw.contains('\\') || raw.contains('\0') {
        return None;
    }
    let mut out = String::new();
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if !byte.is_ascii() || byte < 0x20 || byte == b'/' || byte == b'\\' {
                return None;
            }
            out.push(char::from(byte));
            index += 3;
        } else if bytes[index].is_ascii() && bytes[index] >= 0x20 {
            out.push(char::from(bytes[index]));
            index += 1;
        } else {
            return None;
        }
    }
    if out
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(out)
}

fn allowed(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["styles", style] => *style == "positron" || *style == "dark",
        ["planet"] => true,
        ["fonts", stack, range] => font_stack(stack) && font_range(range),
        ["sprites", id, file] => sprite_id(id) && sprite_file(file),
        ["planet", stamp, z, x, file] => planet_stamp(stamp) && tile_file(z, x, file, 14, "pbf"),
        ["natural_earth", "ne2sr", z, x, file] => tile_file(z, x, file, 6, "png"),
        _ => false,
    }
}

fn font_stack(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | ',' | '_' | '-'))
}

fn font_range(value: &str) -> bool {
    let Some(body) = value.strip_suffix(".pbf") else {
        return false;
    };
    let Some((start, end)) = body.split_once('-') else {
        return false;
    };
    let (Ok(start), Ok(end)) = (start.parse::<u32>(), end.parse::<u32>()) else {
        return false;
    };
    start < end && end <= 65_536 && start.is_multiple_of(256)
}

fn sprite_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn sprite_file(value: &str) -> bool {
    matches!(value, "ofm.json" | "ofm.png" | "ofm@2x.json" | "ofm@2x.png")
}

fn planet_stamp(value: &str) -> bool {
    let Some((day, rest)) = value.split_once('_') else {
        return false;
    };
    let Some((clock, tag)) = rest.split_once('_') else {
        return false;
    };
    tag == "pt"
        && day.len() == 8
        && clock.len() == 6
        && day.chars().all(|c| c.is_ascii_digit())
        && clock.chars().all(|c| c.is_ascii_digit())
}

fn tile_file(z: &str, x: &str, file: &str, max_z: u32, ext: &str) -> bool {
    let Some(y) = file.strip_suffix(&format!(".{ext}")) else {
        return false;
    };
    let (Ok(z), Ok(x), Ok(y)) = (z.parse::<u32>(), x.parse::<u32>(), y.parse::<u32>()) else {
        return false;
    };
    if z > max_z {
        return false;
    }
    let span = 1_u32 << z;
    x < span && y < span
}

fn upstream_url(base: &str, path: &str) -> Option<url::Url> {
    let mut url = url::Url::parse(&format!("{base}/")).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        for part in path.split('/') {
            segments.push(part);
        }
    }
    Some(url)
}

fn rewrite(bytes: &[u8], base: &str) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    let from = format!("{}/", base.trim_end_matches('/'));
    Some(text.replace(&from, "/tiles/p/").into_bytes())
}

#[allow(clippy::case_sensitive_file_extension_comparisons)]
async fn trim_disk(dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let mut files = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.ends_with(".tmp") || name.ends_with(".meta") {
            continue;
        }
        let modified = meta.modified().ok();
        files.push((modified, entry.path()));
    }
    if files.len() <= DISK_CAP {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let extra = files.len() - DISK_CAP;
    for (_, path) in files.into_iter().take(extra) {
        let _ = tokio::fs::remove_file(&path).await;
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            let _ = tokio::fs::remove_file(path.with_file_name(format!("{name}.meta"))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Basemap, allowed, is_quiet, normalize, rewrite};

    #[test]
    fn only_the_two_styles_and_their_assets_are_allowed() {
        assert!(allowed("styles/positron"));
        assert!(allowed("styles/dark"));
        assert!(!allowed("styles/liberty"));
        assert!(allowed("planet"));
        assert!(allowed("planet/20261115_080022_pt/3/2/1.pbf"));
        assert!(!allowed("planet/20261115_080022_pt/15/0/0.pbf"));
        assert!(!allowed("planet/not-a-stamp/0/0/0.pbf"));
        assert!(allowed("fonts/Noto Sans Regular/0-255.pbf"));
        assert!(!allowed("fonts/Noto Sans Regular/1-255.pbf"));
        assert!(allowed("sprites/ofm_f384/ofm.json"));
        assert!(allowed("sprites/ofm_f384/ofm@2x.png"));
        assert!(!allowed("sprites/ofm_f384/other.png"));
        assert!(allowed("natural_earth/ne2sr/2/1/1.png"));
        assert!(!allowed("natural_earth/ne2sr/7/0/0.png"));
        assert!(!allowed("planet/20261115_080022_pt/3/9/1.pbf"));
    }

    #[test]
    fn normalization_rejects_traversal() {
        assert_eq!(
            normalize("fonts/Noto%20Sans/0-255.pbf").as_deref(),
            Some("fonts/Noto Sans/0-255.pbf")
        );
        assert!(normalize("../tokens.json").is_none());
        assert!(normalize("styles/%2e%2e/secret").is_none());
        assert!(normalize("styles//positron").is_none());
        assert!(is_quiet(
            "/tiles/p/planet/20261115_080022_pt/12/2048/1360.pbf"
        ));
        assert!(!is_quiet("/v/1/drives/2"));
    }

    #[test]
    fn style_json_points_back_at_this_server() {
        let body = br#"{"tiles":["http://127.0.0.1:9/planet/20260101_000000_pt/{z}/{x}/{y}.pbf"],"glyphs":"http://127.0.0.1:9/fonts/{fontstack}/{range}.pbf"}"#;
        let rewritten = rewrite(body, "http://127.0.0.1:9").unwrap();
        let text = String::from_utf8(rewritten).unwrap();
        assert!(!text.contains("127.0.0.1"), "{text}");
        assert!(text.contains("\"/tiles/p/planet/20260101_000000_pt/{z}/{x}/{y}.pbf\""));
        assert!(rewrite(b"<html>", "http://127.0.0.1:9").is_none());
    }

    #[tokio::test]
    async fn a_tile_is_fetched_once_and_then_read_from_disk() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let png = b"\x89PNG\r\n\x1a\nsea";
        Mock::given(method("GET"))
            .and(path("/natural_earth/ne2sr/2/1/1.png"))
            .and(header("user-agent", super::USER_AGENT))
            .respond_with(ResponseTemplate::new(200).set_body_raw(png.as_slice(), "image/png"))
            .expect(1)
            .mount(&server)
            .await;
        let dir = std::env::temp_dir().join(format!(
            "tl-tiles-{}",
            crate::crypto::random_token().unwrap()
        ));
        let map = Basemap::new(server.uri(), Some(dir.clone()));
        assert!(map.fetch("../etc/passwd").await.is_err());
        let first = map.fetch("natural_earth/ne2sr/2/1/1.png").await.unwrap();
        assert_eq!(first.bytes.as_slice(), png);
        let second = map.fetch("natural_earth/ne2sr/2/1/1.png").await.unwrap();
        assert_eq!(second.bytes.as_slice(), png);
        let restarted = Basemap::new(server.uri(), Some(dir));
        let third = restarted
            .fetch("natural_earth/ne2sr/2/1/1.png")
            .await
            .unwrap();
        assert_eq!(third.bytes.as_slice(), png);
    }

    #[tokio::test]
    async fn style_urls_are_rewritten_before_they_are_cached() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = format!("{{\"sprite\":\"{}/sprites/ofm_f384/ofm\"}}", server.uri());
        Mock::given(method("GET"))
            .and(path("/styles/positron"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let map = Basemap::new(server.uri(), None);
        let tile = map.fetch("styles/positron").await.unwrap();
        let text = std::str::from_utf8(&tile.bytes).unwrap();
        assert!(text.contains("/tiles/p/sprites/ofm_f384/ofm"), "{text}");
        assert!(!text.contains(&server.uri()), "{text}");
        let _ = map.fetch("styles/positron").await.unwrap();
    }
}
