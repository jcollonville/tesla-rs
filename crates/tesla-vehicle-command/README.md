# tesla-vehicle-command

Rust implementation of Tesla's vehicle-command protocol: the P-256 virtual key, the VCSEC and infotainment handshakes, and signed commands sent through Fleet API.

```rust
use tesla_fleet_api::{BearerToken, FleetClient, Region};
use tesla_vehicle_command::{FleetTransport, PrivateKey, Vehicle};

async fn demo() -> Result<(), tesla_vehicle_command::CommandError> {
    let fleet = FleetClient::for_region(Region::NorthAmerica, BearerToken::new("access-token"))
        .map_err(tesla_vehicle_command::CommandError::from)?;
    let transport = FleetTransport::new(fleet, "5YJ00000000000000")?;
    let key = PrivateKey::read_file(std::path::Path::new("tesla-key.pem"))?;
    let mut vehicle = Vehicle::new(transport, key)?;
    vehicle.wake().await?;
    vehicle.lock().await?;
    Ok(())
}
```

`wake` calls Fleet API `wake_up` once. That call is billable. The client does not wake before other commands.

## What it speaks

* P-256 private keys in SEC1 PEM (`BEGIN EC PRIVATE KEY`) and unencrypted PKCS#8 PEM (`BEGIN PRIVATE KEY`). The public key is the 65-byte uncompressed point.
* Session info for `DOMAIN_VEHICLE_SECURITY` and `DOMAIN_INFOTAINMENT`: ECDH, `K = SHA-1(x)[:16]`, HMAC-SHA256 session authentication, epoch, counter, and domain clock.
* Internet commands use HMAC-SHA256. If the handshake sets the detached-encryption flag, command parameters are AES-GCM encrypted and the command type stays in the authenticated prefix. That is the reference client's Fleet API behavior.
* Responses with `AES_GCM_Response_data` are decrypted and checked for replay. Plaintext responses from firmware older than 2024.38 are accepted and are not authenticated.
* `POST /api/1/vehicles/{vin}/signed_command` through `tesla-fleet-api`.

`CommandTransport` is the extension point for a future BLE transport. BLE itself is not implemented. The advertisement name and GATT layout are described in Tesla's protocol document.

## Build

The build compiles the vendored `.proto` files with `prost` and a bundled `protoc`, so a system protobuf compiler is not required. Those files come from `teslamotors/vehicle-command` and stay under the Apache License 2.0. See `NOTICE`.

## License

MIT OR Apache-2.0
