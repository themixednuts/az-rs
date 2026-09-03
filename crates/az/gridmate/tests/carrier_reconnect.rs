//! Abrupt peer death must release the exact responder generation so the same
//! UDP endpoint can establish a fresh carrier session.

#![cfg(all(feature = "client", feature = "server"))]

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use gridmate::carrier::{
    DataReliability, DatagramHeader, MessageData, MessageFlags, SYSTEM_CHANNEL, SequenceNumber,
    system_message,
};
use gridmate::driver::{
    Established, SecureConnection, SecureConnectionBuilder, generate_self_signed_cert,
};
use gridmate::serialize::{CARRIER_ENDIAN, WriteBuffer};
use gridmate::{CarrierDesc, Event, ServerListenerHandle, Spawner};

const GRIDMATE_VERSION: u32 = 1;

struct ThreadSpawner;

impl Spawner for ThreadSpawner {
    fn spawn(&self, future: gridmate::BoxedFuture) {
        std::thread::spawn(move || futures_lite::future::block_on(future));
    }
}

fn install_spawner() {
    let _ = gridmate::set_spawner(Arc::new(ThreadSpawner));
}

#[test]
fn silent_peer_times_out_and_same_udp_endpoint_gets_fresh_session() {
    install_spawner();
    let (cert_pem, key_pem) = generate_self_signed_cert("gridmate.test").unwrap();

    let mut desc = CarrierDesc::default();
    desc.with_timeout(100).with_disconnect_detection(true);
    let listener = futures_lite::future::block_on(ServerListenerHandle::bind_configured(
        "127.0.0.1:0",
        &cert_pem,
        &key_pem,
        desc,
    ))
    .expect("server bind");
    let server_addr = listener.local_addr().to_string();

    let (client_a, local_addr) =
        futures_lite::future::block_on(connect_carrier(&server_addr, None));
    let session_a = futures_lite::future::block_on(next_ready(&listener));

    let disconnected = futures_lite::future::block_on(next_disconnected(&listener));
    assert_eq!(disconnected.0, session_a);
    assert_eq!(disconnected.1, "BadConnection");
    wait_until(Duration::from_secs(1), || listener.peer_count() == 0);

    drop(client_a);

    let (_client_b, rebound_addr) =
        futures_lite::future::block_on(connect_carrier(&server_addr, Some(local_addr)));
    assert_eq!(rebound_addr, local_addr);
    let session_b = futures_lite::future::block_on(next_ready(&listener));

    assert_ne!(session_a, session_b);
    assert_eq!(listener.peer_count(), 1);
}

async fn connect_carrier(
    server_addr: &str,
    local_addr: Option<std::net::SocketAddr>,
) -> (SecureConnection<Established>, std::net::SocketAddr) {
    let mut builder = SecureConnectionBuilder::new(server_addr);
    if let Some(local_addr) = local_addr {
        builder = builder.with_local_addr(local_addr);
    }
    let mut connection = builder.connect().await.expect("client connect");
    let local_addr = connection.local_addr().expect("client local addr");
    connection
        .write(&connect_request())
        .await
        .expect("write SM_CONNECT_REQUEST");
    with_timeout(connection.read(), Duration::from_secs(1))
        .await
        .expect("SM_CONNECT_ACK timeout")
        .expect("SM_CONNECT_ACK read");
    (connection, local_addr)
}

async fn next_ready(listener: &ServerListenerHandle) -> gridmate::SessionId {
    loop {
        match with_timeout(listener.next_event(), Duration::from_secs(2))
            .await
            .expect("listener timeout")
            .expect("listener closed")
        {
            Event::Ready { session } => return session,
            Event::Error { description } => panic!("listener error: {description}"),
            _ => {}
        }
    }
}

async fn next_disconnected(listener: &ServerListenerHandle) -> (gridmate::SessionId, String) {
    loop {
        match with_timeout(listener.next_event(), Duration::from_secs(2))
            .await
            .expect("listener timeout")
            .expect("listener closed")
        {
            Event::Disconnected { session, reason } => return (session, reason),
            Event::Error { description } => panic!("listener error: {description}"),
            _ => {}
        }
    }
}

async fn with_timeout<F, T>(future: F, timeout: Duration) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    futures_lite::future::or(async { Some(future.await) }, async {
        async_io::Timer::after(timeout).await;
        None
    })
    .await
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + timeout;
    while !predicate() {
        assert!(std::time::Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn connect_request() -> Bytes {
    let mut body = Vec::with_capacity(5);
    body.extend_from_slice(&GRIDMATE_VERSION.to_be_bytes());
    body.push(system_message::SM_CONNECT_REQUEST);

    let message = MessageData {
        reliability: DataReliability::Unreliable,
        channel: SYSTEM_CHANNEL,
        num_chunks: SequenceNumber::from(1),
        sequence_number: SequenceNumber::ZERO,
        send_reliable_seq_num: SequenceNumber::ZERO,
        data: Bytes::from(body),
        #[cfg(debug_assertions)]
        wire_spans: gridmate::carrier::message::MessageWireSpans::default(),
        is_connecting: true,
        ack_callback: None,
    };

    let mut payload = WriteBuffer::new(CARRIER_ENDIAN);
    let flags = MessageFlags::DataChannel as u8 | MessageFlags::Connecting as u8;
    payload.write_u8(flags);
    payload.write_u16(u16::try_from(message.data.len()).expect("test body fits a u16 length"));
    payload.write_u8(message.channel);
    payload.write_u16(message.sequence_number.get());
    payload.write_u16(message.send_reliable_seq_num.get());
    payload.write_bytes(message.data.as_ref());
    let payload = payload.into_vec();

    let mut datagram = Vec::with_capacity(DatagramHeader::SEQUENCE_SIZE + payload.len());
    datagram.extend_from_slice(&1_u16.to_be_bytes());
    datagram.extend_from_slice(&payload);
    Bytes::from(datagram)
}
