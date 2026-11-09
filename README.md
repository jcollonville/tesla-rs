# tesla-rs

Unofficial Rust libraries for Tesla's [Fleet API](https://developer.tesla.com/docs/fleet-api). This project is not affiliated with, endorsed by, or sponsored by Tesla, Inc.

The crates are written to be reused by other programs. They do not depend on the future logger server.

## Crates

| Path | Crate | Status | License |
| --- | --- | --- | --- |
| `crates/tesla-auth` | OAuth 2.0 authorization code with PKCE, refresh, partner tokens, token storage | implemented | MIT OR Apache-2.0 |
| `crates/tesla-fleet-api` | Async Fleet API client (vehicles, live and cached data, wake, fleet status, telemetry config, charging history, partner registration) | implemented | MIT OR Apache-2.0 |
| `crates/tesla-vehicle-command` | Virtual key, session handshake, and signed vehicle commands | implemented | MIT OR Apache-2.0 |
| `crates/tesla-fleet-telemetry` | Fleet Telemetry protobuf decoding and mutual-TLS websocket receiver | implemented | MIT OR Apache-2.0 |
| `server/` | `tesla-logger`, a self-hosted drive, charge, and state archive (Docker and Home Assistant) | not started | AGPL-3.0-only |

`tesla-logger` will be a separate license from the libraries. The AGPL text will be added with the server. Empty crates are not published.

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

## Try the login example

Register an application at the Tesla developer portal and allow `http://127.0.0.1:8734/callback`.

```bash
export TESLA_CLIENT_ID="..."
export TESLA_CLIENT_SECRET="..."
export TESLA_REGION="na"   # na, eu, or cn
cargo run -p list-vehicles
```

The program prints an authorize URL, waits for the browser callback, and lists vehicle name, VIN, and state. It does not print tokens. Set `TESLA_TOKEN_FILE` to persist them.

`examples/lock-unlock` reads that token file, a P-256 PEM from `TESLA_KEY_FILE`, and `TESLA_VIN`. `lock-unlock keygen` writes a new key and prints only the public key. `lock` and `unlock` call `wake_up` once, then the signed command. BLE is not implemented.

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

Tests use a local mock server and do not call Tesla.

## License

Library crates are dual-licensed under `MIT OR Apache-2.0`. Copyright 2026 Jean Collonvillé.

The future `tesla-logger` server will be `AGPL-3.0-only`.
