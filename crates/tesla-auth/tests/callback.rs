//! Loopback callback listener tests.

use std::time::Duration;

use secrecy::ExposeSecret;
use tesla_auth::{bind_callback_listener, listen_for_callback};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn callback_ignores_favicon_and_does_not_echo_the_code() {
    let listener = bind_callback_listener(0).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let wait = tokio::spawn(async move {
        listen_for_callback(listener, "/callback", Duration::from_secs(5)).await
    });

    let mut noise = TcpStream::connect(addr).await.unwrap();
    noise
        .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut ignored = Vec::new();
    let _ = noise.read_to_end(&mut ignored).await;

    let mut login = TcpStream::connect(addr).await.unwrap();
    login
        .write_all(b"GET /callback?code=auth-code&state=state-value HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut page = Vec::new();
    login.read_to_end(&mut page).await.unwrap();
    let page = String::from_utf8_lossy(&page);
    assert!(page.contains("Login complete"));
    assert!(!page.contains("auth-code"));

    let params = wait.await.unwrap().unwrap();
    assert_eq!(params.code().unwrap().expose_secret(), "auth-code");
    assert_eq!(params.state.as_deref(), Some("state-value"));
}
