# tesla-logger

Self-hosted archive of Tesla drives, charges, and vehicle state. It is a clean-room program: TeslaMate is a behavioural reference only, and this project is not affiliated with Tesla, Inc.

The server is licensed `AGPL-3.0-only`. The library crates in `crates/` stay `MIT OR Apache-2.0`.

## What works

* Tesla OAuth (authorization code + PKCE) through `tesla-auth`, with the redirect `https://<TESLA_DOMAIN>/auth/callback`.
* Public key at `/.well-known/appspecific/com.tesla.3p.public-key.pem`, derived from the mounted P-256 private key.
* Sleep-aware Fleet API polling. The poller never calls `wake_up`. It uses the vehicle list to see `online` / `asleep` / `offline`, calls `vehicle_data` only while the car is already online, polls faster while driving or charging, and backs off when parked.
* Drive and charge detection, positions partitioned by month, Nominatim reverse geocoding (cached, rate limited, disable-able), and geofences.
* Read API for Home Assistant: REST plus a WebSocket, described by `openapi.yaml`.
* Optional MQTT Home Assistant discovery (`MQTT_ENABLED=false` by default). The client speaks plain MQTT, which is the usual setup for a broker on the same NAS.
* Optional Fleet Telemetry ingest (`TELEMETRY_ENABLED=false` by default) through `tesla-fleet-telemetry`. A fresh stream suppresses billable live polls for that car.
* TeslaMate 4.x importer (`import-teslamate`), read-only, batched, idempotent, and resumable.

Vehicle commands are not part of this server. Use `tesla-vehicle-command` separately.

## Configure

Copy `server/.env.example` to `server/.env`.

| Variable | Purpose |
| --- | --- |
| `TESLA_CLIENT_ID` / `TESLA_CLIENT_SECRET` | Fleet API application |
| `TESLA_FLEET_AUDIENCE` | `https://fleet-api.prd.eu.vn.cloud.tesla.com` for Europe |
| `TESLA_DOMAIN` | Hostname behind the reverse proxy, for example `tsl.b-612.fr` |
| `TESLA_PUBLIC_BASE_URL` | Public `https://` origin. Defaults to `https://<TESLA_DOMAIN>` |
| `DATABASE_URL` | Postgres URL |
| `TESLA_PRIVATE_KEY_PATH` | P-256 PEM, mounted read-only |
| `TESLA_TOKEN_STORE_PATH` | OAuth token file. Mode `0600` |
| `TESLA_TOKEN_ENCRYPTION_KEY` | Optional. 64 hex chars, or a passphrase hashed with SHA-256, encrypts the token file with AES-256-GCM |
| `TESLA_LISTEN_ADDR` | Default `0.0.0.0:4000` |
| `TESLA_LOGGER_UI_PASSWORD` | Password for the status page |
| `NOMINATIM_*` | Reverse geocoding. Set `NOMINATIM_ENABLED=false` to disable |
| `MQTT_*` | Home Assistant discovery. Off by default |
| `TELEMETRY_*` | Mutual-TLS Fleet Telemetry receiver. Off by default |
| `POLL_*` | Poll intervals |

The UI password protects the status page and the OAuth start button. API calls use a bearer token created on that page (`Authorization: Bearer tl_...`). The WebSocket also accepts `access_token` in the query string; prefer the header so proxy logs do not store it.

Do not enable `sqlx=trace` in production. Query logs can include bound values. Tokens, the client secret, and the private key are not written to logs.

## Docker on a Synology NAS (amd64)

From the repository root, with `server/.env` filled in and `server/private-key.pem` readable by uid `10001`:

```bash
docker compose -f server/docker-compose.yml up -d --build
```

The image runs as uid 10001, uses `postgres:17`, and healthchecks `/healthz`. Point the existing reverse proxy at port 4000 and serve the public key from this process instead of nginx.

Import an existing TeslaMate database (read-only role). `import-tesla-mate` is accepted as an alias.

```bash
docker compose -f server/docker-compose.yml run --rm tesla-logger \
  import-teslamate --source 'postgres://readonly:secret@teslamate-host:5432/teslamate'
```

Positions are copied in pages of at most 2000 rows, and each page commits a cursor, so stopping and rerunning resumes. Only one page is kept in memory. A same-host Postgres 16 imported about 24,000 position rows per second, so 9.3 million positions is about 7 minutes. A NAS disk or a remote database is slower; half that rate is a fair planning figure. The log line `teslamate import progress` reports rows/s and percent complete about every 5 seconds. Charge samples are multi-row inserts too, so a few hundred thousand of them stay in the same order of time as the drives (a few thousand), not the positions.

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

TeslaMate rows are already metric and are copied as stored. TeslaMate does not store drive battery levels on `drives`. The importer reads them from the start and end positions (`battery_level`, then `usable_battery_level`). Migration `0003_backfill_drive_battery` fills drives imported before that, from the positions already in this database, and only where the level is still null. Re-running the importer does the same fill after positions are copied. Live drives keep the state of charge from the moment the drive opens (or the last stored level) and the last level seen before it closes.

## Develop

```bash
export DATABASE_URL=postgres://tesla:tesla@127.0.0.1:5432/tesla_logger
cargo test -p tesla-logger
```

Database tests create throwaway databases and skip when `DATABASE_URL` is unset outside CI.
