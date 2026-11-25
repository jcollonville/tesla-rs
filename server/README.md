# tesla-logger

Self-hosted archive of Tesla drives, charges, and vehicle state. It is a clean-room program: TeslaMate is a behavioural reference only, and this project is not affiliated with Tesla, Inc.

The server is licensed `AGPL-3.0-only`. The library crates in `crates/` stay `MIT OR Apache-2.0`.

Local setup — toolchain, Postgres, environment variables, `cargo run`, Docker Compose, and the checks that show it is working — is in the repository [README](../README.md#run-locally). The variable list there is the one to follow. This file describes logger behavior that the root guide only summarizes.

## What works

* Tesla OAuth (authorization code + PKCE) through `tesla-auth`, with the redirect `https://<TESLA_DOMAIN>/auth/callback`.
* Public key at `/.well-known/appspecific/com.tesla.3p.public-key.pem`, derived from the mounted P-256 private key.
* Sleep-aware Fleet API polling. The poller never calls `wake_up`. Presence comes from the vehicle list and, only before a live read, `GET /api/1/vehicles/{id}`. A live `vehicle_data` call resets Tesla's sleep timer, so a parked car gets at most `POLL_PARKED_LIVE_LIMIT` snapshots (default 1) and then only the vehicle list, at `POLL_BACKOFF_SECS` (default 1800, minimum 1800). `vehicle_data` runs again when a drive or a charge is detected, or when the car comes back online. A healthy Owner stream does not poll `vehicle_data` during a drive, except one call at drive start and one at drive end. A down or stale stream uses `POLL_DRIVE_FALLBACK_SECS` (default 120, minimum 60) and those calls count toward the caps. `Can't validate token` refreshes the Owner token before reconnecting. Disconnects and transport errors reconnect with backoff. Charging defaults to every 900 seconds in polling mode and is not polled in telemetry mode. The daily cap defaults to 160 and cannot be set higher. `FLEET_DATA_DYNAMIC_BUDGET` (default true) further limits the day to the remaining monthly credit divided by the days left. `FLEET_DATA_MONTH_SEED` fills usage the `fleet_usage` table does not have, and only for `FLEET_DATA_MONTH_SEED_MONTH`. At a cap, `vehicle_data` stops; the list and the Owner stream continue, and `charge_status` is `unknown` instead of `unplugged`. HTTP 429 backs off from 60 seconds to 3600 seconds. HTTP 402, and HTTP 403 whose body contains `EXCEEDED_LIMIT`, enter a billing block. A single probe checks whether Tesla has lifted it, every `FLEET_BLOCKED_PROBE_SECS` (default 3600, 300–86400), or sooner from the manual resume on Système. If the vehicle list returned the block, the probe is that list; otherwise it is one `vehicle_data` call while a car is already online. A successful probe resumes normal polling. The probe counts toward the daily and monthly caps. Any other HTTP 403 uses the 429 backoff.
* Drive and charge detection, positions partitioned by month, Nominatim reverse geocoding (cached, rate limited, disable-able), and geofences.
* Read API for Home Assistant: REST plus a WebSocket, described by `openapi.yaml`.
* Optional MQTT Home Assistant discovery (`MQTT_ENABLED=false` by default). The client speaks plain MQTT to `MQTT_HOST` and `MQTT_PORT`.
* Optional Fleet Telemetry (`TELEMETRY_ENABLED=false` by default). With the certificate and key set, it is the data source: the logger subscribes the needed signals and does not poll `vehicle_data` except a stale fallback (`POLL_TELEMETRY_FALLBACK_SECS`, default 3600) while the car is online. Startup logs `data_source`, and the state API exposes the same field. See the repository README section "Fleet Telemetry (optional)".
* Optional Owner API streaming for dense drive traces. Fleet API polling stays the primary source. The legacy streaming socket is unofficial and Tesla may remove it.
* TeslaMate 4.x importer (`import-teslamate`), read-only, batched, idempotent, and resumable.

Vehicle commands are not part of this server. Use `tesla-vehicle-command` separately.

## Configure

Copy `server/.env.example` to `server/.env` for Docker Compose. For `cargo run` on the host, the container hostnames and `/data` paths in that example do not apply; use the local file in the repository README. `tesla-logger` reads `TESLA_LOGGER_ENV_FILE` or `./.env`, and a process environment variable overrides the file. The Fleet redirect is always `https://<TESLA_DOMAIN>/auth/callback`.

The UI password protects the status page, the logs page, and the OAuth start button. API calls use a bearer token created on that page (`Authorization: Bearer tl_...`). The WebSocket also accepts `access_token` in the query string; prefer the header so proxy logs do not store it. Recent application logs are kept in memory and shown at `/logs` after signing in. A request with no session is redirected to `/`, which is the sign-in page. Passwords, tokens, authorization headers, and websocket subscribe frames are redacted. The list resets when the process restarts.

## Owner API streaming

Fleet API polling records climate, charge, and the other fields the Owner stream does not carry. While that stream is connected and a `data:update` is younger than 90 seconds, drive GPS comes from the stream (`source=stream`) and `vehicle_data` is not polled, except one call at drive start and one at drive end for temperatures, tyres, and odometer. The optional Owner API stream is the same legacy websocket TeslaMate uses (`wss://streaming.vn.teslamotors.com/streaming/`). It is not part of the Fleet API. Tesla can change or remove it without notice. When it is disconnected, rate-limited, rejected, or connected without a `data:update` for 90 seconds during a drive with movement evidence younger than 60 seconds, `vehicle_data` runs every `POLL_DRIVE_FALLBACK_SECS` (default 120 seconds, minimum 60). A `source=poll` position is stored at most every `POLL_GPS_FALLBACK_SECS`. Drive-end grace and Hold use the parked cadence. The poller still does not call `wake_up`. The debug console and `position_mode` on the state API report `hybrid` or `fallback`. Mode changes are logged as `position mode` with the database id and no VIN. A switch pulls the next poll forward at most once every 120 seconds.

Enable it from the status page:

1. Open the logger and sign in with `TESLA_LOGGER_UI_PASSWORD`.
2. Connect the Fleet API account if it is not connected yet. Polling does not use the Owner API token.
3. Open **Set up streaming**.
4. Choose **Prepare Tesla sign-in** once, open the Tesla link, and sign in with the Tesla account that owns the car. The authorize host is `auth.tesla.com`. China accounts (`TESLA_FLEET_AUDIENCE` on the China Fleet API host) use `auth.tesla.cn`. Choosing Prepare again replaces the pending login; the URL from the first attempt then fails the state check.
5. After login, the browser cannot open `tesla://auth/callback`. It may say the address is not understood, the site cannot be reached, or offer to open the Tesla app. Do not open the Tesla app.
6. Copy the full URL from the address bar, the console, or the failed navigation in the Network tab. It starts with `tesla://auth/callback?`. Paste that URL only into this logger. It contains the authorization code.
7. Paste it into **Redirect URL** and choose **Save sign-in**. The page returns to the vehicle table. The password field is only so the authorization code is not displayed. The pending login lasts 10 minutes. Pasting with nothing pending, or after a newer Prepare, asks you to prepare again.
8. The stream chip and `GET /api/v1/vehicles/{id}/state` field `stream_status` become `connected` once the subscribe frame is written, including while the car is parked and Tesla sends only hello or ping frames. They are `error` while it is reconnecting, and `disabled` when no Owner token is stored or `OWNER_STREAM_ENABLED` is false. `stream_error` is a short class such as `timeout` or `auth` and never contains a token. The log line is `owner stream status`. Cars are listed with `GET /api/1/products`; `GET /api/1/vehicles` returns HTTP 412 and is not used.

A refresh token generated for this logger can be pasted into **Owner API refresh token** instead of the redirect URL. Do not paste the refresh token TeslaMate is using. Tesla rotates refresh tokens, and using TeslaMate's token here can sign TeslaMate out. Each program should have its own Owner API login. The Fleet API login is a third one and is unchanged.

`TESLA_TOKEN_ENCRYPTION_KEY` encrypts `owner-tokens.json` with the same AES-256-GCM format as the Fleet API token file. With Docker Compose the file is `/data/owner-tokens.json` on the `logger-data` volume. No restart is required after saving or disconnecting. Set `OWNER_STREAM_ENABLED=false` to stop connecting without deleting the token.

Streamed rows are stored in `positions` with `source = 'stream'` and linked to the open drive. Those rows may leave temperature null. A poll within two seconds replaces the streamed point at that moment. Shift changes open and close the drive; an empty streaming gear does not close it. If the socket fails during a drive, `vehicle_data` runs on `POLL_DRIVE_FALLBACK_SECS` and does not call `wake_up`. Usage, the estimated cost, and a billing-limit resume are on Système and at `GET /api/v1/fleet/usage`.

Do not enable `sqlx=trace` in production. Query logs can include bound values. Tokens, the client secret, and the private key are not written to logs.

## Docker Compose

The command sequence, published port, volume warning, and uid `10001` key requirement are in the repository [README](../README.md#docker-compose). Short form, from the repository root, with `server/.env` filled in and `server/private-key.pem` readable by uid `10001`:

```bash
docker compose -f server/docker-compose.yml up -d --build
```

The image runs as uid 10001, uses `postgres:17`, and healthchecks `/healthz`. Publish host port 4000. Terminate TLS in front of this process and forward `/auth/callback` and `/.well-known/appspecific/com.tesla.3p.public-key.pem` to it. `docker compose down` keeps the `postgres` and `logger-data` volumes; `down -v` deletes them.

Import an existing TeslaMate database (read-only role). `import-tesla-mate` is accepted as an alias.

```bash
docker compose -f server/docker-compose.yml run --rm tesla-logger \
  import-teslamate --source 'postgres://readonly:secret@teslamate-host:5432/teslamate'
```

Positions are copied in pages of 5000 rows. Each page orders by `positions.id` and compares the cursor as `integer`, so Postgres uses the primary key. The page commits a cursor, so stopping and rerunning resumes, including a cursor saved by an older build. Only one page is kept in memory. The log line `teslamate import progress` reports rows/s and percent complete about every 5 seconds. Charge samples are multi-row inserts too, so a few hundred thousand of them stay in the same order of time as the drives (a few thousand), not the positions.

`repair-drive-ends` recomputes `end_at` with the same rule as a live close. The end becomes the first exact 0 km/h sample of the final zero run. Attached positions strictly after that instant are detached (`drive_id` set to null), not deleted. A missing speed inside the run stays non-moving and cannot start the run. A tail with no 0 km/h sample, or a 0 km/h sample followed by movement, is left unchanged. Rows that already have a null `drive_id` are never attached. The command is not part of startup. The default is a dry-run that prints `mode`, `closed`, `changed`, `unchanged`, and `drive_id` lines only. `--apply` writes, one transaction per drive, and refreshes the summary. A second run reports `changed=0`.

```bash
docker compose -f server/docker-compose.yml run --rm tesla-logger repair-drive-ends
docker compose -f server/docker-compose.yml run --rm tesla-logger repair-drive-ends --apply
```

Images are published to `ghcr.io/jcollonville/tesla-logger` on `main` and on version tags. The workflow builds the runner architecture (amd64). The Dockerfile itself is not architecture-specific.

## API

`GET /api/openapi.yaml` is the contract. Authenticated routes:

* `GET /api/v1/vehicles` and `GET /api/v1/vehicles/{id}` include `car_render`
* `GET /api/v1/vehicles/{id}/car.png` serves the cached official render (`image/png`, strong `ETag`, `Cache-Control: private, max-age=86400`, `304` on `If-None-Match`). `{id}` is the numeric vehicle id. `404` with `{"error":"not found"}` when no picture is available
* `GET /api/v1/vehicles/{id}/state`
* `GET /api/v1/drives?vehicle_id&from&to&page&per_page`
* `GET /api/v1/drives/{id}?max_points=300` returns an RDP route and a stride time series
* `GET /api/v1/charges` and `GET /api/v1/charges/{id}`
* `GET /api/v1/stats?vehicle_id&from&to`
* `GET /api/v1/geofences`
* `GET /api/v1/fleet/usage` returns today and month-to-date `vehicle_data` calls, the estimated cost, the caps, and whether `vehicle_data` is paused
* `GET /api/v1/ws` pushes `{"type":"state",...}` frames

Distances are kilometres. Fleet API miles are converted on ingest. Temperatures are Celsius, power is kilowatts, tire pressures are bar, charger voltage is volts, and charger current is amperes. `GET /api/v1/vehicles/{id}/state` and each WebSocket `state` frame return every `vehicle_state` column. Door, window, and trunk keys use the Fleet API names (`df`, `dr`, `pf`, `pr`, `fd_window`, `fp_window`, `rd_window`, `rp_window`, `ft`, `rt`). `0` is closed. `charging_state` is `charge_state.charging_state`, or Fleet Telemetry `DetailedChargeState` when the car sends that instead of the deprecated `ChargeState` field. `plugged_in` is derived from the charge cable, that charging state, and the charge-port latch. `charge_status` is `charging`, `plugged`, `unplugged`, or `unknown`. `unknown` and `charge_status_stale` are used when `vehicle_data` is paused and telemetry is not fresh. `data_source` is `polling` or `telemetry`. `scheduled_charging_start_time`, `scheduled_departure_time`, and `off_peak_hours_end_time` are minutes after midnight on the car's local clock. `scheduled_charging_mode` is `Off`, `StartAt`, or `DepartBy`. No extra Tesla endpoint is called for the schedule.

TeslaMate rows are already metric and are copied as stored. TeslaMate does not store drive battery levels on `drives`. The importer reads them from the start and end positions (`battery_level`, then `usable_battery_level`). Every `import-teslamate` run fills any still-null drive levels from the stored positions after the position copy finishes, including a resume that only imports the remaining positions. Migration `0003_backfill_drive_battery` does the same fill once at startup. Live drives keep the state of charge from the moment the drive opens (or the last stored level) and the last level seen before it closes.

## Maps

Opening a drive or a charge draws that track, or that charge's location, on an OpenFreeMap basemap: Positron when the console is light, Dark when it is dark. MapLibre GL JS 5.24.0 (BSD-3-Clause) is served from `/static/` with the other console files. The browser does not load the library from a CDN.

Tiles, fonts, and sprites are fetched by this server from `https://tiles.openfreemap.org` and cached next to the token store, in `map-tiles/`. The browser only talks to this process, so `img-src` and `connect-src` stay `'self'`. OpenFreeMap sees the logger's address and the tiles for the area on screen, not the visitor's address and not a VIN. Cached tiles are kept for seven days; style documents are refreshed after six hours. `TILE_UPSTREAM=off` keeps the drawn track and makes no tile request. Set `TILE_UPSTREAM` to another HTTPS origin when you host a compatible tile set yourself.

That proxy is the privacy choice. A direct browser request would send the visitor's address and the viewed area to the tile host, and this server's `Referrer-Policy: same-origin` would strip the `Referer` those hosts expect from web pages. The cost of proxying is bandwidth on the logger and that the operator's address is what the tile host records. The GPS is already in this database. Tile paths are not written to the request log, because a tile coordinate is a place.

MapLibre positions its canvas with style attributes, so the page policy allows `style-src-attr 'unsafe-inline'`. Stylesheets and scripts stay on this origin. The worker is the CSP build (`worker-src 'self'`), not a `blob:` worker. Attribution (OpenFreeMap, OpenMapTiles, OpenStreetMap) is on the map. The overview mini-map stays a local drawing and does not request tiles, so opening the home page does not publish where the car is parked.

## Develop

```bash
export DATABASE_URL=postgres://tesla:tesla@127.0.0.1:5432/tesla_logger
cargo test -p tesla-logger
```

Database tests create throwaway databases and skip when `DATABASE_URL` is unset outside CI.

`demo/seed.sql` fills a migrated database with an invented archive (two cars, six weeks of drives and charges, places in the ocean near 0°N 0°E) for UI work and screenshots. Each run replaces only its own demo rows:

```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f server/demo/seed.sql
```

`demo/bench-seed.sql` builds a synthetic archive of a few years (5000 drives, about 3.2 million positions by default, invented places near 0°N 0°E) on an empty migrated database, to time the console pages. Never run it against real data:

```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -v drives=5000 -v pts_div=3 -f server/demo/bench-seed.sql
```

## Car pictures

The overview and fleet cards show Tesla's official render of the car when one is known for its model, paint, and wheels (Model 3, Y, S, and X in the paints Tesla's image service still draws). The server asks `static-assets.tesla.com/v1/compositor/` once per combination with option codes only (never a VIN or vehicle id), keeps the PNG in memory and in `car-renders/` next to `TESLA_TOKEN_STORE_PATH`, and serves it from `/v/{id}/car.png` to signed-in users. The same bytes are available to a read API token at `GET /api/v1/vehicles/{id}/car.png` (`image/png`, strong `ETag`, `Cache-Control: private, max-age=86400`, `304` when `If-None-Match` matches). `{id}` is the numeric vehicle id, never a VIN. `car_render` on the vehicle JSON is true when that route may have a picture. The browser therefore loads images from its own origin and the CSP stays `img-src 'self' data:`. When the paint is not available, the car is a Cybertruck, the fetch fails, or `CAR_RENDERS_ENABLED=false`, the page keeps the drawn silhouette and the API returns `404` with `{"error":"not found"}`. Failures are remembered for a few minutes (a day for a refusal) so a missing picture does not trigger a request per page view.
