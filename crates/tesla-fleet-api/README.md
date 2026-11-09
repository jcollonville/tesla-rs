# tesla-fleet-api

Async client for the Tesla Fleet API.

```rust
use tesla_fleet_api::{BearerToken, FleetClient, Region};

let client = FleetClient::for_region(Region::Europe, BearerToken::new("access-token"))
    .expect("client");
let vehicles = client.list_vehicles(&Default::default()).await.expect("vehicles");
let _ = vehicles;
```

`vehicle_data` is billable and is never preceded by an automatic wake. Pass an explicit endpoint list. Use `fleet_status` and `vehicle_data_cached` when a live read is unnecessary.

The client accepts any [`AccessTokenProvider`](https://docs.rs/tesla-fleet-api), including `tesla_auth::Session`.

This crate is unofficial and is not affiliated with Tesla.

## License

MIT OR Apache-2.0
