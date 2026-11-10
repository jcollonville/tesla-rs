# tesla-owner-api

Unofficial client for Tesla's legacy Owner API and the vehicle streaming websocket.

This crate is not affiliated with Tesla, Inc. The Owner API (`ownerapi` client, `https://owner-api.teslamotors.com`) and `wss://streaming.vn.teslamotors.com/streaming/` are not the Fleet API. Tesla can change or remove them without notice.

The streaming socket is what TeslaMate uses for about one position per second while driving. Fleet API `vehicle_data` polling is a separate product and stays the source for commands, configuration, and the rest of the vehicle state.

## What it does

* Authorization-code login with PKCE for the public `ownerapi` client. The registered redirect is `tesla://auth/callback` on both `auth.tesla.com` and `auth.tesla.cn`. A normal browser cannot open that scheme. The operator copies the full `tesla://auth/callback?code=…&state=…` URL from the address bar (or the failed navigation in the Network tab) and pastes it back. A bare query string is also accepted.
* Token exchange and refresh are `application/x-www-form-urlencoded`. The request sends `client_id=ownerapi` and does not send a client secret. The HTTP client is `reqwest` with rustls and HTTP/2. Keep HTTP/2 enabled: refreshing over HTTP/1.1 can return an access token that the Owner API rejects.
* `GET /api/1/products` to resolve the streaming `vehicle_id` (the `tag`). This is not the REST `id`. `GET /api/1/vehicles` now returns HTTP 412. Products without `vehicle_id` (Powerwall, solar) are skipped.
* Subscribe frame `data:subscribe_oauth` and parsers for `data:update`, `data:error`, and `control:hello`.
* Reconnect delays for timeout, disconnect, vehicle errors, auth failures, and an endpoint that has gone away.

Access tokens and refresh tokens are `secrecy::SecretString`. `Debug` redacts them. The subscribe frame contains the access token; do not log that string.

Streaming fields `speed`, `odometer`, `range`, and `est_range` are miles. `elevation` is feet. `power` is kilowatts. Callers convert.

## License

MIT OR Apache-2.0. Copyright 2026 Jean Collonvillé.
