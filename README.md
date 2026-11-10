# tesla-rs

Unofficial Rust libraries for Tesla's [Fleet API](https://developer.tesla.com/docs/fleet-api), plus a self-hosted logger. This project is not affiliated with, endorsed by, or sponsored by Tesla, Inc.

The library crates are written to be reused by other programs. They do not depend on the logger. The example binaries call those crates and read the process environment. `tesla-logger` (`server/`) is a separate program: it stores drives, charges, and vehicle state in Postgres, polls the Fleet API, and can optionally receive Fleet Telemetry, publish MQTT, and open the unofficial Owner API stream.

## Crates

| Path | Crate | Status | License |
| --- | --- | --- | --- |
| `crates/tesla-auth` | OAuth 2.0 authorization code with PKCE, refresh, partner tokens, token storage | implemented | MIT OR Apache-2.0 |
| `crates/tesla-owner-api` | Unofficial Owner API login and the legacy streaming websocket | implemented | MIT OR Apache-2.0 |
| `crates/tesla-fleet-api` | Async Fleet API client (vehicles, live and cached data, wake, fleet status, telemetry config, charging history, partner registration) | implemented | MIT OR Apache-2.0 |
| `crates/tesla-vehicle-command` | Virtual key, session handshake, and signed vehicle commands | implemented | MIT OR Apache-2.0 |
| `crates/tesla-fleet-telemetry` | Fleet Telemetry protobuf decoding and mutual-TLS websocket receiver | implemented | MIT OR Apache-2.0 |
| `server/` | `tesla-logger`, a self-hosted drive, charge, and state archive | implemented | AGPL-3.0-only |
| `examples/list-vehicles` | Loopback Fleet login, then a vehicle list | example | MIT OR Apache-2.0 |
| `examples/lock-unlock` | Virtual-key `keygen`, `lock`, and `unlock` | example | MIT OR Apache-2.0 |
| `examples/telemetry-receiver` | Print decoded Fleet Telemetry frames | example | MIT OR Apache-2.0 |

`tesla-logger` is licensed separately from the libraries. See `server/README.md` and `server/LICENSE`.

## Run locally

Run every command from the repository root. Relative paths below are relative to that directory.

### Prerequisites

* Rust. `rust-toolchain.toml` pins the toolchain CI uses (1.99.1, with rustfmt and clippy). The workspace MSRV is 1.88 (`rust-version` in the root `Cargo.toml`). With rustup, the first `cargo` command in this directory installs the pinned toolchain.
* A system linker (`cc` on Linux). The Fleet Telemetry and vehicle-command builds compile vendored protobuf with a bundled `protoc`; a system protobuf compiler is not required.
* For `tesla-logger` only: Postgres 17 (the version in `server/docker-compose.yml` and in CI). Docker is optional and is only needed for Compose, or if you want Postgres in a container.
* The examples need outbound HTTPS to Tesla. They do not need Postgres or Docker.

### Clone

```bash
git clone https://github.com/jcollonville/tesla-rs.git
cd tesla-rs
```

| Path | What it is |
| --- | --- |
| `crates/` | Library crates |
| `examples/` | `list-vehicles`, `lock-unlock`, `telemetry-receiver` |
| `server/` | `tesla-logger` binary, SQL migrations, `Dockerfile`, `docker-compose.yml`, `.env.example` |
| `server/migrations/` | Applied by the `tesla-logger` binary at startup. There is no separate migrate command |

### Environment files

Cargo does not load a `.env` file. The example binaries call `std::env::var` and never read a dotenv file. `tesla-logger` does read one, with its own parser (see below).

`.env` is listed in `.gitignore`, so a file with that name is not committed. `*.pem` and `*.key` are ignored too. Token JSON files are not ignored; do not commit them.

Shell rules when you load a file yourself:

* No spaces around `=`.
* Single-quote any value that contains `$`, so the shell does not expand it.

```bash
set -a
source .env
set +a
cargo run -p list-vehicles
```

`source .env` reads the file in the current directory. From the repository root, a file saved at `server/.env` is loaded with `source server/.env`.

`tesla-logger` also reads a dotenv file on its own. The path is `TESLA_LOGGER_ENV_FILE`, or `.env` in the current directory when that variable is unset. A missing file is ignored. The parser:

* skips blank lines and lines starting with `#`
* accepts an `export ` prefix
* trims whitespace around the key and the value
* strips one matching pair of `'` or `"` quotes
* ignores keys whose value is empty
* does not expand `$` or backticks
* lets a real process environment variable replace the same key from the file

`server/.env.example` is written for Docker Compose: the database host is `postgres`, and the key and token paths are `/keys` and `/data` inside the container. Those values do not work for `cargo run` on the host. Use the local block under [Run tesla-logger](#run-tesla-logger) for a process you start yourself, and keep the example file's container paths when you use Compose.

### Tesla application

Create an application in the Tesla developer portal for the account's region.

* North America and Europe share `auth.tesla.com` and use different Fleet API audiences.
* China is a separate account, authorization server, and application on `developer.tesla.cn`.

Tesla has rejected application names that contain the word "tesla". Use a name that does not.

Register every redirect URI you will actually use. The examples and the logger use different URIs:

| Use | Redirect URI |
| --- | --- |
| `list-vehicles` and `lock-unlock`, default port | `http://127.0.0.1:8734/callback` |
| Same examples with `TESLA_CALLBACK_PORT` or `TESLA_REDIRECT_URI` set | That exact URI. The default shape is `http://127.0.0.1:{port}/callback` |
| `tesla-logger` Fleet login | `https://<TESLA_DOMAIN>/auth/callback` |

The logger always builds that Fleet redirect as `https://` plus `TESLA_DOMAIN` plus `/auth/callback`. `TESLA_PUBLIC_BASE_URL` does not change it.

Owner API streaming does not use this application. It uses Tesla's public `ownerapi` client, whose redirect is `tesla://auth/callback` on both `auth.tesla.com` and `auth.tesla.cn`. You do not register that URI on your Fleet application.

Copy the application's client id and client secret into the environment variables below. Use your own values. Do not commit them, and do not paste them into chat or tickets.

### Run `list-vehicles`

Required:

| Variable | Default | Purpose |
| --- | --- | --- |
| `TESLA_CLIENT_ID` | | Fleet application client id |
| `TESLA_CLIENT_SECRET` | | Fleet application client secret |

Optional:

| Variable | Default | Purpose |
| --- | --- | --- |
| `TESLA_REGION` | `na` | `na`, `eu`, or `cn`. Aliases: `north-america`, `north_america`, `europe`, `china` |
| `TESLA_CALLBACK_PORT` | `8734` | Loopback port. `0` asks the OS for a free port |
| `TESLA_REDIRECT_URI` | `http://127.0.0.1:{bound port}/callback` | Must match the application registration |
| `TESLA_SCOPES` | see below | Space-separated scope override |
| `TESLA_TOKEN_FILE` | unset | When set, the token set is written there, mode `0600` on Unix |

The default scopes are `openid`, `offline_access`, `user_data`, `vehicle_device_data`, `vehicle_location`, `vehicle_cmds`, and `vehicle_charging_cmds`.

`na` is North America and Asia-Pacific excluding China (`https://fleet-api.prd.na.vn.cloud.tesla.com`). `eu` is Europe, the Middle East, and Africa (`https://fleet-api.prd.eu.vn.cloud.tesla.com`). `cn` is China (`https://fleet-api.prd.cn.vn.cloud.tesla.cn`). An EU account should set `TESLA_REGION=eu` before login. The example default is `na`, and a token minted for the wrong audience can be rejected. Port `0` binds a free port and puts that port in the default redirect URI; register that URI, or set `TESLA_REDIRECT_URI` to one that is already registered.

```bash
set -a
source .env
set +a
cargo run -p list-vehicles
```

A `.env` for this example:

```bash
TESLA_CLIENT_ID=your-client-id
TESLA_CLIENT_SECRET='your-client-secret'
TESLA_REGION=eu
```

The program prints an authorize URL, waits up to 300 seconds for the browser to hit the loopback callback, and prints each vehicle's name, VIN, and state. It does not print tokens. The callback page does not echo the authorization code.

After login it prints `account region ...`. If the token audience and the account region differ, it lists vehicles from the account's Fleet API base and prints `token audience is ...`. If that host rejects the token, it prints `sign in again with TESLA_REGION set to the account region`.

### Run `lock-unlock`

`lock` and `unlock` call Fleet API `wake_up` once, then a signed command. `wake_up` is billable. BLE is not implemented.

`keygen` only needs `TESLA_KEY_FILE`. It writes a new P-256 SEC1 PEM (`BEGIN EC PRIVATE KEY`) and prints the public key. It refuses to replace an existing file unless `TESLA_KEY_FORCE=1`. On Unix the file mode is `0600`.

```bash
TESLA_KEY_FILE=server/private-key.pem cargo run -p lock-unlock -- keygen
```

`lock` and `unlock` need:

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `TESLA_VIN` | yes | | Vehicle VIN |
| `TESLA_KEY_FILE` | yes | | P-256 SEC1 or unencrypted PKCS#8 PEM |
| `TESLA_TOKEN_FILE` | yes | | Token file from `list-vehicles`, or a file this program will create |
| `TESLA_CLIENT_ID` | yes | | Needed to refresh, and to log in when the token file is empty |
| `TESLA_CLIENT_SECRET` | yes | | Same |
| `TESLA_REGION` | no | `na` | Same values as `list-vehicles` |
| `TESLA_CALLBACK_PORT` | no | `8734` | Used when the token file is empty and a loopback login runs |
| `TESLA_REDIRECT_URI` | no | `http://127.0.0.1:{port}/callback` | Must match the application registration |
| `TESLA_SCOPES` | no | same default as `list-vehicles` | Space-separated override |
| `TESLA_COMMAND` | no | `lock` | `lock` or `unlock`, used when no argument is passed |
| `TESLA_KEY_FORCE` | no | unset | Set to `1` to let `keygen` replace an existing key file |

```bash
set -a
source .env
set +a
cargo run -p lock-unlock -- lock
cargo run -p lock-unlock -- unlock
```

Usage is `lock-unlock [keygen|lock|unlock]`. When the token file has no tokens, the program runs the same loopback login as `list-vehicles` and saves the token set. The private key and tokens are not printed. The command prints the public key hex and `wake state ...`.

### Run `telemetry-receiver`

This example is separate from `tesla-logger`'s `TELEMETRY_*` settings. It prints decoded frames and does not store them.

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `TESLA_TELEMETRY_CERT` | yes | | PEM server certificate chain, leaf first |
| `TESLA_TELEMETRY_KEY` | yes | | PEM private key |
| `TESLA_TELEMETRY_BIND` | no | `0.0.0.0:4443` | Listen address |
| `TESLA_TELEMETRY_CA` | no | `prod` | `prod` or `eng` |
| `TESLA_TELEMETRY_CLIENT_CA` | no | unset | Extra PEM appended to the Tesla CA bundle |

```bash
export TESLA_TELEMETRY_CERT=server.crt
export TESLA_TELEMETRY_KEY=server.key
cargo run -p telemetry-receiver
```

The Tesla CA bundles are the ones shipped with the reference server. This example does not prove that a live vehicle certificate chains to them.

### Run tesla-logger

`tesla-logger` listens for HTTP, migrates Postgres, polls the Fleet API, and serves the status UI. It does not terminate TLS. Fleet login's redirect is still `https://<TESLA_DOMAIN>/auth/callback`, so a browser can finish that login only when an HTTPS front end for `TESLA_DOMAIN` forwards to this process (including `/.well-known/appspecific/com.tesla.3p.public-key.pem`). The UI, `/healthz`, `/logs`, and that public-key path are served on `TESLA_LISTEN_ADDR` over HTTP.

#### Logger environment

Required:

| Variable | Purpose |
| --- | --- |
| `TESLA_CLIENT_ID` | Fleet application client id |
| `TESLA_CLIENT_SECRET` | Fleet application client secret |
| `TESLA_FLEET_AUDIENCE` | Fleet API base URL. One of `https://fleet-api.prd.na.vn.cloud.tesla.com`, `https://fleet-api.prd.eu.vn.cloud.tesla.com`, `https://fleet-api.prd.cn.vn.cloud.tesla.cn`. A trailing slash is accepted. This chooses the region |
| `TESLA_DOMAIN` | Bare hostname used in the Fleet redirect. No scheme, path, or spaces. `https://<TESLA_DOMAIN>/auth/callback` must be registered on the application |
| `DATABASE_URL` | Postgres URL the process can open |
| `TESLA_PRIVATE_KEY_PATH` | P-256 PEM, SEC1 (`BEGIN EC PRIVATE KEY`) or unencrypted PKCS#8 (`BEGIN PRIVATE KEY`). The public key is derived from this file |
| `TESLA_TOKEN_STORE_PATH` | Fleet OAuth token file. Mode `0600` on Unix. The file contains secrets |
| `TESLA_LOGGER_UI_PASSWORD` | Password for the status UI. This is not the Tesla account password |

Optional:

| Variable | Default | Purpose |
| --- | --- | --- |
| `TESLA_PUBLIC_BASE_URL` | `https://<TESLA_DOMAIN>` | `https://` origin, or `http://` on `localhost`, `127.0.0.1`, or `::1`. The UI session cookie is marked `Secure` when this starts with `https://` |
| `TESLA_LISTEN_ADDR` | `0.0.0.0:4000` | HTTP listen socket |
| `TESLA_TOKEN_ENCRYPTION_KEY` | unset | Encrypts the Fleet token file and the Owner token file with AES-256-GCM. 64 hex characters are the raw key. Any other value is hashed with SHA-256 |
| `TESLA_LOGGER_ENV_FILE` | `.env` | Dotenv path. Relative to the current directory |
| `TESLA_OWNER_TOKEN_STORE_PATH` | `owner-tokens.json` next to `TESLA_TOKEN_STORE_PATH` | Owner API token file. Same encryption key |
| `OWNER_STREAM_ENABLED` | `true` | `true`/`false`, `1`/`0`, `yes`/`no`, or `on`/`off`. `false` keeps a saved Owner token and does not connect |
| `POLL_DRIVE_SECS` | `15` | Poll interval while driving |
| `POLL_CHARGE_SECS` | `60` | Poll interval while charging |
| `POLL_PARKED_SECS` | `180` | Poll interval while parked |
| `POLL_ASLEEP_SECS` | `300` | Poll interval while asleep |
| `POLL_BACKOFF_SECS` | `720` | Poll interval after a backoff |
| `POLL_PARKED_LIVE_LIMIT` | `2` | Live `vehicle_data` reads while parked |
| `NOMINATIM_ENABLED` | `true` | Reverse geocoding. The URL must be `https` when enabled |
| `NOMINATIM_URL` | `https://nominatim.openstreetmap.org` | Nominatim base URL |
| `NOMINATIM_CONTACT` | unset | Included in the User-Agent: `tesla-logger/0.1 (self-hosted; …)` |
| `NOMINATIM_MIN_INTERVAL_MS` | `1100` | Minimum delay between Nominatim requests |
| `MQTT_ENABLED` | `false` | Home Assistant discovery over plain MQTT |
| `MQTT_HOST` | `127.0.0.1` | Broker host |
| `MQTT_PORT` | `1883` | Broker port |
| `MQTT_USERNAME` | unset | Sent only when `MQTT_PASSWORD` is set too |
| `MQTT_PASSWORD` | unset | Sent only when `MQTT_USERNAME` is set too |
| `MQTT_TOPIC_PREFIX` | `tesla_logger` | Topic prefix |
| `TELEMETRY_ENABLED` | `false` | Mutual-TLS Fleet Telemetry receiver inside the logger |
| `TELEMETRY_BIND` | `0.0.0.0:4443` | Telemetry listen socket |
| `TELEMETRY_CERT_PATH` | unset | PEM certificate chain. Required for telemetry to start |
| `TELEMETRY_KEY_PATH` | unset | PEM private key. Required for telemetry to start |
| `RUST_LOG` | `info,sqlx=warn,hyper=warn,h2=warn,rumqttc=warn,reqwest=warn` | stderr filter. Leave `sqlx` off `trace`; query logs can include bound values |

`POSTGRES_PASSWORD` is read by Docker Compose, not by `tesla-logger`. `TESLAMATE_DATABASE_URL` is read only by the `import-teslamate` subcommand.

Booleans and integers that are unset use the defaults. An empty value in the dotenv file is treated as unset.

#### Private key

Generate a key with the example above (`TESLA_KEY_FILE=server/private-key.pem`). The logger does not generate one. On startup it reads the PEM and serves the public key at `/.well-known/appspecific/com.tesla.3p.public-key.pem` with no authentication. It does not call Fleet API partner registration.

#### Postgres for `cargo run`

Compose does not publish Postgres to the host. For a process on the host, start a database yourself. This matches the Compose image, user, and database name:

```bash
docker run -d --name tesla-logger-postgres \
  -e POSTGRES_USER=tesla_logger \
  -e POSTGRES_PASSWORD=change-me \
  -e POSTGRES_DB=tesla_logger \
  -p 127.0.0.1:5432:5432 \
  postgres:17
```

Pick your own password and use that same password in `DATABASE_URL`. `change-me` is only a placeholder.

#### Local env file and process

Save this as `server/.env` and replace the placeholders. `TESLA_REGION=eu` in the examples corresponds to the EU audience below; use the NA or CN audience for those accounts.

```bash
TESLA_CLIENT_ID=your-client-id
TESLA_CLIENT_SECRET='your-client-secret'
TESLA_FLEET_AUDIENCE=https://fleet-api.prd.eu.vn.cloud.tesla.com
TESLA_DOMAIN=logger.example.com
TESLA_PUBLIC_BASE_URL=http://127.0.0.1:4000
DATABASE_URL=postgres://tesla_logger:change-me@127.0.0.1:5432/tesla_logger
TESLA_PRIVATE_KEY_PATH=server/private-key.pem
TESLA_TOKEN_STORE_PATH=server/tokens.json
TESLA_LOGGER_UI_PASSWORD='choose-a-password'
TESLA_LISTEN_ADDR=127.0.0.1:4000
OWNER_STREAM_ENABLED=true
NOMINATIM_ENABLED=true
MQTT_ENABLED=false
TELEMETRY_ENABLED=false
```

`TESLA_DOMAIN=logger.example.com` makes the Fleet redirect `https://logger.example.com/auth/callback`. Register that URI. A value such as `127.0.0.1:4000` is accepted and becomes `https://127.0.0.1:4000/auth/callback`; the HTTP listener still does not serve that HTTPS URL. Point DNS and an HTTPS reverse proxy for `TESLA_DOMAIN` at `127.0.0.1:4000` before you use **Connect Tesla**. `TESLA_PUBLIC_BASE_URL=http://127.0.0.1:4000` keeps the UI cookie usable when you open the HTTP listener directly.

Migrations are compiled into the binary (`server/migrations/`) and run when the server starts, before it accepts connections. `import-teslamate` migrates the destination database as well.

```bash
TESLA_LOGGER_ENV_FILE=server/.env cargo run -p tesla-logger
```

That is the `serve` command. `cargo run -p tesla-logger -- serve` is the same. The process logs `listening` with the socket address. Stop it with Ctrl-C.

A missing required variable exits with `error: missing environment variable <NAME>`. That message includes the name. The examples do not.

#### Fleet login

1. Open `http://127.0.0.1:4000/` and sign in with `TESLA_LOGGER_UI_PASSWORD`.
2. Choose **Connect Tesla**. The browser goes to Tesla and, after approval, to `https://<TESLA_DOMAIN>/auth/callback`.
3. That request has to hit this process, and the browser still has to be signed in to the logger. The pending login is kept in memory. Restarting the process drops it.

The status page can also create an API token. It is shown once and has the form `tl_...`. Send it as `Authorization: Bearer tl_...`. The WebSocket also accepts `access_token` in the query string; prefer the header so proxy logs do not store it.

#### Owner API streaming

Fleet polling does not use the Owner API token. The stream is optional and unofficial. The socket is `wss://streaming.vn.teslamotors.com/streaming/`, or `wss://streaming.vn.cloud.tesla.cn/streaming/` when `TESLA_FLEET_AUDIENCE` is the China Fleet URL. Tesla can change or remove it. When it is disconnected, the logger keeps polling.

The public Owner API client redirects to `tesla://auth/callback`. It does not use `https://auth.tesla.com/void/callback`. A normal browser cannot open `tesla://`.

1. Sign in to the logger.
2. Open **Set up streaming**.
3. Choose **Prepare Tesla sign-in** once. Open the Tesla link and sign in with the account that owns the car. China accounts (`TESLA_FLEET_AUDIENCE` on `fleet-api.prd.cn.vn.cloud.tesla.cn`) use `auth.tesla.cn`. Every other audience uses `auth.tesla.com`.
4. The browser may say the address is not understood, the site cannot be reached, or offer to open the Tesla app. Do not open the Tesla app.
5. Copy the full URL from the address bar, the browser console, or the failed navigation in the Network tab. It starts with `tesla://auth/callback?code=…&state=…`. A bare `code=…&state=…` query string is also accepted.
6. Paste that URL into **Redirect URL** on the logger page and choose **Save sign-in**. The field is a password field so the authorization code is not displayed. The code in that URL is a secret: paste it only into this form.

**Prepare Tesla sign-in** stores a single pending login for 10 minutes. Choosing it again replaces that login. Pasting the URL from the first attempt then fails with "Owner API sign-in failed. Prepare it again and paste the full redirect URL." Prepare once, finish that sign-in, and prepare again only after the page tells you to.

A refresh token created for this logger can be pasted into **Owner API refresh token** instead. Do not reuse a TeslaMate refresh token. Tesla rotates refresh tokens, and using TeslaMate's token here can sign TeslaMate out. The Fleet API login is a separate token.

Cars for the stream come from `GET /api/1/products`. Products without a vehicle id (Powerwall, solar) are skipped. `GET /api/1/vehicles` returns HTTP 412 for this client and is not used. If the products call fails, cars already stored with a legacy vehicle id are used.

The stream chip, and `stream_status` on `GET /api/v1/vehicles/{id}/state`, become `connected` once the subscribe frame has been written, including while the car is parked and Tesla sends only hello or ping frames. They are `error` while a car is still connecting or reconnecting, and `disabled` when `OWNER_STREAM_ENABLED` is false or no Owner token is stored. `stream_error` is a short class such as `timeout` or `auth` and does not contain a token.

No restart is required after saving or disconnecting. `OWNER_STREAM_ENABLED=false` stops connecting and keeps the token file.

`/logs` shows recent lines from this process after you are signed in. Passwords, tokens, authorization headers, and subscribe frames are redacted. The list is in memory (400 lines) and is empty after a restart.

More of the archive behavior (polling, import, API fields) is in `server/README.md`.

### Docker Compose

From the repository root, with `server/.env` filled in from `server/.env.example` and `server/private-key.pem` present:

```bash
docker compose -f server/docker-compose.yml up -d --build
```

Compose reads `server/.env` (the `env_file` path is relative to the compose file). For the `tesla-logger` service it overrides:

| Variable | Value inside the container |
| --- | --- |
| `DATABASE_URL` | `postgres://tesla_logger:<POSTGRES_PASSWORD>@postgres:5432/tesla_logger` |
| `TESLA_LISTEN_ADDR` | `0.0.0.0:4000` |
| `TESLA_TOKEN_STORE_PATH` | `/data/tokens.json` |
| `TESLA_OWNER_TOKEN_STORE_PATH` | `/data/owner-tokens.json` |
| `TESLA_PRIVATE_KEY_PATH` | `/keys/private-key.pem` |

`POSTGRES_PASSWORD` in `server/.env` is the Postgres role password. It defaults to `change-me` when unset. The `DATABASE_URL` line in that file is not the URL the container uses; the override above is. The host port is `4000`. Postgres is not published to the host.

The image runs as uid `10001`. `keygen` writes the PEM with mode `0600`, so a file owned by another user is not readable in the container. Make `server/private-key.pem` readable by uid `10001` without making it world-readable. The compose file mounts it read-only at `/keys/private-key.pem`.

Named volumes `postgres` and `logger-data` hold the database and the token files. Stop the stack without deleting them:

```bash
docker compose -f server/docker-compose.yml down
```

`down -v` deletes those volumes, including the archived drives and the token files. The private key is a bind mount and stays on the host either way.

To publish a different host port, change the left-hand side of `4000:4000` in the compose file. The process inside the container still listens on `4000` unless you also change the `TESLA_LISTEN_ADDR` override.

The image build is `server/Dockerfile` (Rust 1.99.1, then a Debian slim runtime). Published images are `ghcr.io/jcollonville/tesla-logger` on `main` and on version tags.

Import from an existing TeslaMate database (read-only role) is documented in `server/README.md`. `import-tesla-mate` is an alias of `import-teslamate`.

### Check the setup

These checks assume the listener is on port 4000, as in the example file and Compose.

Logger health (no login):

```bash
curl -fsS http://127.0.0.1:4000/healthz
```

A healthy process prints `ok`. If Postgres is down the status is `503` and the body is `database unavailable`.

The same check as a one-shot command, against the default URL `http://127.0.0.1:4000/healthz`:

```bash
cargo run -p tesla-logger -- healthcheck
```

`healthcheck` does not load the rest of the configuration.

Public key (no login):

```bash
curl -fsS http://127.0.0.1:4000/.well-known/appspecific/com.tesla.3p.public-key.pem
```

UI:

1. Open `http://127.0.0.1:4000/`. The heading is **Sign in** (the document title is `tesla-logger`). The password is `TESLA_LOGGER_UI_PASSWORD`.
2. After login the dashboard lists vehicles once Fleet polling has stored some. **Tesla connected** means a Fleet token file is present.
3. Open `http://127.0.0.1:4000/logs` in a private window, or after signing out. You are sent back to `/`, which is the sign-in page. After signing in, `/logs` shows the in-memory lines.

Examples:

```bash
set -a
source .env
set +a
cargo run -p list-vehicles
```

A successful run prints one line per vehicle (`name`, VIN, state) or `no vehicles`.

### Failures

| What you see | What it means |
| --- | --- |
| `error: environment variable not found` from `list-vehicles`, `lock-unlock`, or `telemetry-receiver` | A required variable is unset. `std::env::var` does not include the name. For `list-vehicles` the required names are `TESLA_CLIENT_ID` and `TESLA_CLIENT_SECRET`. Loading `.env` with `source` is required; `cargo run` does not do it. |
| `error: missing environment variable TESLA_…` from `tesla-logger` | That variable is absent or empty after dotenv parsing. The name is in the message. |
| `error: unknown fleet api region` | `TESLA_REGION` is not `na`, `eu`, `cn`, or one of the aliases above. |
| `sign in again with TESLA_REGION set to the account region` | The token was minted for the default or configured region and that Fleet host rejected it. Set `TESLA_REGION` to the account region and sign in again. |
| Browser never hits the loopback callback, then `error: timed out waiting for the oauth callback` | The redirect URI is not the one registered on the application, or nothing reached `http://127.0.0.1:{port}/callback` within 300 seconds. |
| `error: oauth callback error: …` as soon as the example starts | The callback port is not available. The default is `8734` on `127.0.0.1`. |
| `TESLA_FLEET_AUDIENCE must be a Fleet API base URL …` | The value is not one of the three production Fleet API bases. |
| `TESLA_DOMAIN must be a bare hostname` | The value contains `://`, `/`, or a space. |
| `TESLA_PUBLIC_BASE_URL must be https, or http on localhost` | The origin is not HTTPS and not HTTP on loopback. |
| UI password submits and the sign-in form comes back, with no "Incorrect password" | `TESLA_PUBLIC_BASE_URL` is `https://…`, so the session cookie is `Secure`, and you opened the plain HTTP listener. For `http://127.0.0.1:4000` set `TESLA_PUBLIC_BASE_URL=http://127.0.0.1:4000`. |
| `sign in to the logger first` on `/auth/callback` | The Fleet callback arrived without a logger UI session. |
| `unknown oauth state` on `/auth/callback` | This process did not start that login, or it has restarted since. Use **Connect Tesla** again. |
| Owner page: "Prepare the Tesla sign-in again, then paste the redirect URL." | **Save sign-in** ran with no pending login. |
| Owner page: "That sign-in expired. Prepare it again." | The pending login is older than 10 minutes. |
| Owner page: "Owner API sign-in failed. Prepare it again and paste the full redirect URL." | The pasted URL's `state` does not match the pending login (including a second click of **Prepare Tesla sign-in**), or the URL is not the full redirect. Prepare once more and paste the URL from that attempt. |
| `token file is encrypted; set TESLA_TOKEN_ENCRYPTION_KEY` | The token file was written with an encryption key and the current process has none. |
| `/logs` returns you to the sign-in page | There is no UI session. Sign in on `/` first. |
| Telemetry log: `telemetry is enabled but TELEMETRY_CERT_PATH or TELEMETRY_KEY_PATH is missing` | `TELEMETRY_ENABLED` is true and one path is unset. The HTTP server still runs; the telemetry listener does not. |

## Security

* Access tokens, refresh tokens, ID tokens, client secrets, authorization codes, and PKCE verifiers are `secrecy::SecretString`. `Debug` redacts them.
* The HTTP clients do not log headers or bodies. `Authorization` is marked as a sensitive header. Redirects are disabled.
* Token and API URLs must be HTTPS, except HTTP on loopback for tests and the local login callback.
* `FileTokenStore` writes the token file atomically and, on Unix, with mode `0600`. The file contains secrets.
* Refresh tokens are single-use (Tesla keeps the previous one valid for up to 24 hours if it was not saved). `Session` serializes refresh inside one process.
* `vehicle_data` and `wake_up` are billable. The Fleet client never calls `wake_up` for you. A sleeping vehicle comes back as `FleetError::DeviceUnavailable`. Prefer `fleet_status` or `vehicle_data_cached`, and Fleet Telemetry for ongoing data.
* ID tokens are stored and redacted. This crate does not validate their signatures.
* `tesla-vehicle-command` keeps the P-256 scalar and the session key in `secrecy` containers and zeroizes them on drop. MAC checks are constant-time. SHA-1 is used only for the protocol's `K = SHA-1(x)[:16]` derivation. Private keys are not logged.
* Fleet API `wake_up` remains explicit. `Vehicle::wake` calls it once and does not hide the billing.

## Development

* Edition 2024
* MSRV 1.88 (`rust-version` in the workspace)
* `rust-toolchain.toml` pins the current stable toolchain used by CI (1.99.1)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo doc --workspace --no-deps
cargo deny check
cargo audit
cargo +1.88.0 check --workspace --all-targets --locked
```

Tests use a local mock server and do not call Tesla. `tesla-logger` database tests create throwaway databases and skip when `DATABASE_URL` is unset outside CI. CI sets `DATABASE_URL=postgres://tesla:tesla@localhost:5432/tesla_logger` against Postgres 17. That role is the test fixture; the Compose file uses a different role, `tesla_logger`.

## License

Library crates are dual-licensed under `MIT OR Apache-2.0`. Copyright 2026 Jean Collonvillé.

`tesla-logger` is `AGPL-3.0-only`.
