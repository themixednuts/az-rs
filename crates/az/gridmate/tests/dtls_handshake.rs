//! Server-side DTLS handshake — the tracer bullet for the server transport.
//!
//! Boots a real `SecureSocketListener` on loopback and connects this crate's
//! `SecureConnectionBuilder` against it. Both sides reach `Established` only
//! if the cookie exchange + `SSL_accept` flow are wired correctly.

#![cfg(all(feature = "client", feature = "server"))]

use gridmate::driver::{SecureConnectionBuilder, SecureSocketListener, generate_self_signed_cert};

#[test]
fn server_accepts_handshake_from_client() {
    // Self-signed RSA cert + key — RSA so the client's cipher
    // (ECDHE-RSA-AES256-GCM-SHA384) negotiates.
    let (cert_pem, key_pem) = generate_self_signed_cert("gridmate.test").unwrap();

    // Bind on an OS-chosen loopback port so multiple test runs don't collide.
    let listener = futures_lite::future::block_on(SecureSocketListener::bind(
        "127.0.0.1:0",
        &cert_pem,
        &key_pem,
    ))
    .expect("server bind");

    let server_addr = listener.local_addr().expect("local_addr").to_string();

    // Server task on a separate thread so the client can drive its own
    // async-io reactor on the main thread without contention.
    let server_thread = std::thread::spawn(move || {
        futures_lite::future::block_on(
            async move { listener.accept().await.expect("server accept") },
        )
    });

    let client_conn =
        futures_lite::future::block_on(SecureConnectionBuilder::new(&server_addr).connect())
            .expect("client connect");

    let server_conn = server_thread.join().expect("server thread join");

    // Both being typed `SecureConnection<Established>` is the contract; the
    // explicit drops just keep the connections alive past the handshake check
    // so close-on-drop doesn't race with assertions added later.
    drop(server_conn);
    drop(client_conn);
}

/// After the handshake, plaintext written on either side must arrive
/// decrypted on the other. Proves the `Established`-state `read`/`write`
/// paths work over a server-accepted connection (not just client-initiated).
#[test]
fn established_connection_round_trips_app_data() {
    const CLIENT_TO_SERVER: &[u8] = b"hello from client";
    const SERVER_TO_CLIENT: &[u8] = b"hello from server";

    let (cert_pem, key_pem) = generate_self_signed_cert("gridmate.test").unwrap();

    let listener = futures_lite::future::block_on(SecureSocketListener::bind(
        "127.0.0.1:0",
        &cert_pem,
        &key_pem,
    ))
    .expect("server bind");
    let server_addr = listener.local_addr().expect("local_addr").to_string();

    let server_thread = std::thread::spawn(move || {
        futures_lite::future::block_on(async move {
            let mut server_conn = listener.accept().await.expect("server accept");
            let received = with_timeout(server_conn.read(), 3_000).await;
            assert_eq!(
                received.as_ref(),
                CLIENT_TO_SERVER,
                "server got wrong bytes"
            );
            with_timeout(server_conn.write(SERVER_TO_CLIENT), 3_000).await;
            server_conn
        })
    });

    let mut client_conn =
        futures_lite::future::block_on(SecureConnectionBuilder::new(&server_addr).connect())
            .expect("client connect");

    futures_lite::future::block_on(async {
        with_timeout(client_conn.write(CLIENT_TO_SERVER), 3_000).await;
        let received = with_timeout(client_conn.read(), 3_000).await;
        assert_eq!(
            received.as_ref(),
            SERVER_TO_CLIENT,
            "client got wrong bytes"
        );
    });

    let server_conn = server_thread.join().expect("server thread join");
    drop(server_conn);
    drop(client_conn);
}

/// Bound an awaitable so a regression cannot hang the cargo test runner.
/// The `poll_next` contract is fragile enough that defending against this is
/// worth the extra helper.
async fn with_timeout<F, T>(fut: F, ms: u64) -> T
where
    F: std::future::Future<Output = Result<T, gridmate::driver::DriverError>>,
{
    use futures_lite::future::FutureExt;
    let timeout = async {
        async_io::Timer::after(std::time::Duration::from_millis(ms)).await;
        Err(gridmate::driver::DriverError::Ssl(format!(
            "timeout after {ms}ms"
        )))
    };
    fut.or(timeout)
        .await
        .unwrap_or_else(|e| panic!("future returned error: {e:?}"))
}
