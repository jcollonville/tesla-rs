# tesla-logger

Self-hosted archive of Tesla drives, charges, and vehicle state. It is a clean-room program: TeslaMate is a behavioural reference only, and this project is not affiliated with Tesla, Inc.

The server is licensed `AGPL-3.0-only`. The library crates in `crates/` stay `MIT OR Apache-2.0`.

Local setup — toolchain, Postgres, environment variables, `cargo run`, Docker Compose, and the checks that show it is working — is in the repository [README](../README.md#run-locally). The variable list there is the one to follow. This file describes logger behavior that the root guide only summarizes.

## What works

* Tesla OAuth (authorization code + PKCE) through `tesla-auth`, with the redirect `https://<TESLA_DOMAIN>/auth/callback`.
* Public key at `/.well-known/appspecific/com.tesla.3p.public-key.pem`, derived from the mounted P-256 private key.
* Sleep-aware Fleet API polling. The poller never calls `wake_up`. It uses the vehicle list to see `online` / `asleep` / `offline`, calls `vehicle_data` only while the car is already online, polls faster while driving or charging, and backs off when parked.
* Drive and charge detection, positions partitioned by month, Nominatim reverse geocoding (cached, rate limited, disable-able), and geofences.
* Read API for Home Assistant: REST plus a WebSocket, described by `openapi.yaml`.
* Optional MQTT Home Assistant discovery (`MQTT_ENABLED=false` by default). The client speaks plain MQTT to `MQTT_HOST` and `MQTT_PORT`.
* Optional Fleet Telemetry ingest (`TELEMETRY_ENABLED=false` by default) through `tesla-fleet-telemetry`. A fresh stream suppresses billable live polls for that car.
* Optional Owner API streaming for dense drive traces. Fleet API polling stays the primary source. The legacy streaming socket is unofficial and Tesla may remove it.
* TeslaMate 4.x importer (`import-teslamate`), read-only, batched, idempotent, and resumable.

Vehicle commands are not part of this server. Use `tesla-vehicle-command` separately.

## Configure

Copy `server/.env.example` to `server/.env` for Docker Compose. For `cargo run` on the host, the container hostnames and `/data` paths in that example do not apply; use the local file in the repository README. `tesla-logger` reads `TESLA_LOGGER_ENV_FILE` or `./.env`, and a process environment variable overrides the file. The Fleet redirect is always `https://<TESLA_DOMAIN>/auth/callback`.

The UI password protects the status page, the logs page, and the OAuth start button. API calls use a bearer token created on that page (`Authorization: Bearer tl_...`). The WebSocket also accepts `access_token` in the query string; prefer the header so proxy logs do not store it. Recent application logs are kept in memory and shown at `/logs` after signing in. A request with no session is redirected to `/`, which is the sign-in page. Passwords, tokens, authorization headers, and websocket subscribe frames are redacted. The list resets when the process restarts.

## Owner API streaming

Fleet API polling records a position every `POLL_DRIVE_SECS` (15 seconds by default, often slower when Tesla rate-limits `vehicle_data`). The optional Owner API stream is the same legacy websocket TeslaMate uses (`wss://streaming.vn.teslamotors.com/streaming/`). It is not part of the Fleet API. Tesla can change or remove it without notice. When it is disconnected, rate-limited, or rejected, the logger logs `owner stream status`, backs off, and keeps polling.

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

Streamed rows are stored in `positions` with `source = 'stream'` and linked to the open drive. A poll within two seconds replaces the streamed point at that moment. Shift changes open and close the drive; an empty streaming gear does not close it. If the socket fails, polling continues.

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

Images are published to `ghcr.io/jcollonville/tesla-logger` on `main` and on version tags. The workflow builds the runner architecture (amd64). The Dockerfile itself is not architecture-specific.

## API

`GET /api/openapi.yaml` is the contract. Authenticated routes:

* `GET /api/v1/vehicles` and `GET /api/v1/vehicles/{id}`
* `GET /api/v1/vehicles/{id}/state`
* `GET /api/v1/drives?vehicle_id&from&to&page&per_page`
* `GET /api/v1/drives/{id}?max_points=300` returns an RDP route and a stride time series
* `GET /api/v1/charges` and `GET /api/v1/charges/{id}`
* `GET /api/v1/stats?vehicle_id&from&to`
* `GET /api/v1/geofences`
* `GET /api/v1/ws` pushes `{"type":"state",...}` frames

Distances are kilometres. Fleet API miles are converted on ingest. Temperatures are Celsius, power is kilowatts, tire pressures are bar, charger voltage is volts, and charger current is amperes. `GET /api/v1/vehicles/{id}/state` and each WebSocket `state` frame return every `vehicle_state` column. Door, window, and trunk keys use the Fleet API names (`df`, `dr`, `pf`, `pr`, `fd_window`, `fp_window`, `rd_window`, `rp_window`, `ft`, `rt`). `0` is closed. `plugged_in` is derived from the charge-port latch, the charging state, and the charge cable. `scheduled_charging_start_time` is minutes after midnight.

TeslaMate rows are already metric and are copied as stored. TeslaMate does not store drive battery levels on `drives`. The importer reads them from the start and end positions (`battery_level`, then `usable_battery_level`). Every `import-teslamate` run fills any still-null drive levels from the stored positions after the position copy finishes, including a resume that only imports the remaining positions. Migration `0003_backfill_drive_battery` does the same fill once at startup. Live drives keep the state of charge from the moment the drive opens (or the last stored level) and the last level seen before it closes.

## Develop

```bash
export DATABASE_URL=postgres://tesla:tesla@127.0.0.1:5432/tesla_logger
cargo test -p tesla-logger
```

Database tests create throwaway databases and skip when `DATABASE_URL` is unset outside CI.
