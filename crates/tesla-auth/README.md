# tesla-auth

OAuth 2.0 client for the Tesla Fleet API.

* Authorization-code login with PKCE `S256`
* Refresh-token rotation
* Partner tokens via client credentials
* `TokenStore`, with memory and file implementations
* North America, Europe, and China hosts

```rust
use secrecy::SecretString;
use tesla_auth::{AuthClient, AuthorizeOptions, ClientConfig, Region, Scope};

let client = AuthClient::new(ClientConfig {
    client_id: "client-id".into(),
    client_secret: SecretString::from("client-secret".to_owned()),
    redirect_uri: "http://127.0.0.1:8734/callback".into(),
    region: Region::NorthAmerica,
    scopes: Scope::third_party_vehicle(),
})
.expect("client");
let pending = client.begin_login(&AuthorizeOptions::default()).expect("login");
println!("{}", pending.url());
```

This crate is unofficial and is not affiliated with Tesla. See the [repository README](https://github.com/jcollonville/tesla-rs) for security notes.

## License

MIT OR Apache-2.0
