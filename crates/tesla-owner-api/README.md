# tesla-owner-api

Unofficial client for Tesla's legacy Owner API and the vehicle streaming websocket.

This crate is not affiliated with Tesla, Inc. The Owner API (`ownerapi` client, `https://owner-api.teslamotors.com`) and `wss://streaming.vn.teslamotors.com/streaming/` are not the Fleet API. Tesla can change or remove them without notice.

The streaming socket is what TeslaMate uses for about one position per second while driving. Fleet API `vehicle_data` polling is a separate product and stays the source for commands, configuration, and the rest of the vehicle state.

## What it does

* Authorization-code login with PKCE for the public `ownerapi` client, including the `https://auth.tesla.com/void/callback` redirect TeslaMate uses.
* Refresh-token grant. The request sends `client_id=ownerapi` and does not send a client secret.
* `GET /api/1/vehicles` to resolve the streaming `vehicle_id` (the `tag`). This is not the REST `id`.
* Subscribe frame `data:subscribe_oauth` and parsers for `data:update`, `data:error`, and `control:hello`.
* Reconnect delays for timeout, disconnect, vehicle errors, auth failures, and an endpoint that has gone away.

Access tokens and refresh tokens are `secrecy::SecretString`. `Debug` redacts them. The subscribe frame contains the access token; do not log that string.

Streaming fields `speed`, `odometer`, `range`, and `est_range` are miles. `elevation` is feet. `power` is kilowatts. Callers convert.

## License

MIT OR Apache-2.0. Copyright 2026 Jean Collonvillé.
