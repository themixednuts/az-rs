//! `SecureConnection` - DTLS connection implementation
//!
//! `GridMate`: `SecureSocketDriver::Connection`
//! Internal connection state for `SecureSocketDriver`

use crate::carrier::connection_security;
#[cfg(feature = "client")]
use crate::carrier::state::{
    ConnectionEvent, ConnectionState as StateMachineState, StateMachine, StateResult,
};
use crate::driver::error::DriverError;
use async_io::Async;
#[cfg(feature = "client")]
use async_io::Timer;
use bytes::Bytes;
use foreign_types::ForeignType;
use futures_util::Stream;
use openssl::ssl::Ssl;
// `SslContext` backs both the client's `SecureConnectionInner::ssl_ctx` and the
// server's `SecureSocketListener::ssl_ctx`; the builder-side names are used by
// `connect_bound` (client) and `build_server_ssl_ctx` (server). A
// `transport`-only build compiles neither, so gate the imports to match rather
// than leave them unused.
#[cfg(any(feature = "client", feature = "server"))]
use openssl::ssl::{SslContext, SslOptions, SslVerifyMode};
#[cfg(any(feature = "client", feature = "server"))]
use openssl::x509::X509;
#[cfg(feature = "client")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "client")]
use std::io::Write as StdWrite;
use std::net::SocketAddr;
use std::net::UdpSocket;
#[cfg(feature = "client")]
use std::path::Path;
use std::pin::Pin;
use std::ptr;
use std::sync::Arc;
use std::task::{Context, Poll};
#[cfg(feature = "client")]
use std::time::Duration;
use tracing::{debug, trace};

// SSL error constants
const SSL_ERROR_WANT_READ: i32 = 2;
const SSL_ERROR_WANT_WRITE: i32 = 3;
const BIO_CTRL_PENDING: i32 = 10;

/// Encrypted-datagram scratch buffer size — one maximum-size UDP payload.
///
/// Heap-allocated as a boxed slice rather than an inline `[u8; N]`: a 64 KiB
/// array is materialised on the stack before it can be boxed, which overflows
/// small stacks in unoptimised builds.
const RECV_BUFFER_LEN: usize = 65536;

// Manual FFI declaration for DTLSv1_2_method (not exposed by openssl-sys by default)
// This function creates an SSL_METHOD for DTLS 1.2 which sets the record layer version to 0xFEFD
//
// Only the client `connect_bound` and the server `build_server_ssl_ctx` build a
// context; a `transport`-only build compiles the connection type without either
// and has no caller for the binding.
#[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
unsafe extern "C" {
    fn DTLSv1_2_method() -> *const openssl_sys::SSL_METHOD;
}

/// Global SSL keylog file (for Wireshark decryption)
///
/// Using `std::sync::Mutex` is acceptable here because:
/// 1. It's only accessed briefly during initialization and SSL callbacks
/// 2. We use `try_lock()` to avoid blocking in async contexts
/// 3. If lock fails, we just skip writing that keylog line (acceptable for debugging)
#[cfg(feature = "client")]
static SSL_KEYLOG_FILE: std::sync::Mutex<Option<File>> = std::sync::Mutex::new(None);

// ============================================================================
// BIO Wrapper - Safe abstraction over OpenSSL memory BIOs
// ============================================================================

/// Safe wrapper for OpenSSL memory BIO
///
/// Note: BIOs attached to SSL via `SSL_set_bio` are owned by the SSL object
/// and freed when SSL is dropped. This wrapper does NOT implement Drop
/// because the BIO is typically owned by SSL, not by this struct.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MemBio(*mut openssl_sys::BIO);

// SAFETY: OpenSSL 1.1.1+ is thread-safe. BIOs are managed by the SSL object
// which is also thread-safe. Access is synchronized via the owning SecureConnection.
unsafe impl Send for MemBio {}
unsafe impl Sync for MemBio {}

impl MemBio {
    /// Create a new memory BIO
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Ssl`] if `BIO_new(BIO_s_mem())` returns null,
    /// which OpenSSL does only on allocation failure.
    // Only `bio_new_mem_pair` constructs one, and that is reachable only from
    // the client/server handshake paths.
    #[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
    pub(crate) fn new() -> Result<Self, DriverError> {
        // Set BIO to return -1 on EOF (GridMate pattern)
        const BIO_C_SET_BUF_MEM_EOF_RETURN: i32 = 130;

        let bio = unsafe { openssl_sys::BIO_new(openssl_sys::BIO_s_mem()) };
        if bio.is_null() {
            return Err(DriverError::Ssl("Failed to create BIO".to_string()));
        }

        unsafe {
            openssl_sys::BIO_ctrl(bio, BIO_C_SET_BUF_MEM_EOF_RETURN, -1, ptr::null_mut());
        }

        Ok(Self(bio))
    }

    /// Feed data into the BIO (for read BIO - data going into SSL)
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Ssl`] if `data` is longer than `i32::MAX` (the
    /// width `BIO_write` accepts), or if `BIO_write` accepted fewer bytes than
    /// were offered.
    pub(crate) fn feed(&self, data: &[u8]) -> Result<(), DriverError> {
        let len = i32::try_from(data.len()).map_err(|_| {
            DriverError::Ssl(format!("BIO_write length {} exceeds i32", data.len()))
        })?;
        unsafe {
            let written = openssl_sys::BIO_write(self.0, data.as_ptr().cast(), len);
            if written != len {
                return Err(DriverError::Ssl(format!(
                    "BIO_write incomplete: {}/{} bytes",
                    written,
                    data.len()
                )));
            }
        }
        Ok(())
    }

    /// Drain data from the BIO (for write BIO - data coming from SSL)
    pub(crate) fn drain(&self) -> Bytes {
        use bytes::BytesMut;
        let mut result = BytesMut::with_capacity(16384);
        let mut buf = [0u8; 16384];
        // Fixed 16 KiB scratch buffer, so its length is exactly the C `int`
        // `BIO_read` wants; the clamp is unreachable and only avoids a cast.
        let capacity = i32::try_from(buf.len()).unwrap_or(i32::MAX);

        unsafe {
            loop {
                let read = openssl_sys::BIO_read(self.0, buf.as_mut_ptr().cast(), capacity);
                trace!("BIO_read returned: {}", read);
                // A negative return is EOF / retry / error, and zero means the
                // BIO is drained — the conversion doubles as the old `read <= 0`
                // guard.
                let Ok(read) = usize::try_from(read) else {
                    break;
                };
                if read == 0 {
                    break;
                }
                result.extend_from_slice(&buf[..read]);
            }
        }

        trace!("Drained total {} bytes from BIO", result.len());
        result.freeze()
    }

    /// Get the raw pointer (for `SSL_set_bio` and other OpenSSL functions)
    // Only the client/server `SSL_set_bio` calls need the raw handle.
    #[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
    pub(crate) const fn as_ptr(&self) -> *mut openssl_sys::BIO {
        self.0
    }

    /// Check pending bytes in BIO
    pub(crate) fn pending(&self) -> usize {
        let pending =
            unsafe { openssl_sys::BIO_ctrl(self.0, BIO_CTRL_PENDING, 0, ptr::null_mut()) };
        // `BIO_ctrl` returns C `long`, which is 32-bit on Windows and 64-bit
        // on Linux. `BIO_CTRL_PENDING` is a byte count; a negative OpenSSL
        // error therefore maps to no readable bytes.
        usize::try_from(pending).unwrap_or_default()
    }
}

/// Create a pair of memory BIOs for DTLS (`read_bio`, `write_bio`)
///
/// # Errors
///
/// Returns any error [`MemBio::new`] returns.
// Called only from the client `connect_bound` / `create_ssl` and the server
// `accept` paths, so a `transport`-only build has no caller.
#[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
pub(crate) fn bio_new_mem_pair() -> Result<(MemBio, MemBio), DriverError> {
    Ok((MemBio::new()?, MemBio::new()?))
}

/// Feed data into read BIO (convenience wrapper)
pub(crate) fn bio_feed(bio: MemBio, data: &[u8]) -> Result<(), DriverError> {
    bio.feed(data)
}

/// Drain data from write BIO (convenience wrapper)
pub(crate) fn bio_drain(bio: MemBio) -> Bytes {
    bio.drain()
}

/// Type-level state markers for `SecureConnection`
/// State is encoded in the type system, preventing invalid operations at compile time
/// Idle state - not initialized
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Idle;

/// `CookieExchange` state - DTLS cookie exchange phase
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookieExchange;

/// Connect state - DTLS handshake in progress
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connect;

/// Established state - DTLS connection ready for encrypted data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Established;

/// Disconnected state - connection closed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disconnected;

/// Connection state (kept for runtime queries only - type-level state is preferred)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureConnectionState {
    Idle,
    CookieExchange,
    Connect,
    Established,
    Disconnected,
}

/// Inner state shared across all connection states (enables safe state transitions)
struct SecureConnectionInner {
    socket: Arc<Async<UdpSocket>>,
    #[cfg(feature = "client")]
    ssl_ctx: SslContext,
    ssl: Ssl,
    read_bio: MemBio,
    write_bio: MemBio,
    addr: SocketAddr,
    #[cfg(feature = "client")]
    state_machine: StateMachine,
    /// Encrypted UDP recv destination — fed straight to `bio_feed`,
    /// never escapes as `Bytes`. Boxed to avoid the stack overflow
    /// when the inner is moved into async futures.
    recv_buffer: Box<[u8]>,
    /// Decrypted `SSL_read` output — produces `Arc`-shared `Bytes`
    /// without `copy_from_slice` (see
    /// [`super::recv_ring::RecvRing`]). Amortises one allocation
    /// across many SSL records on the client side.
    decrypt_ring: super::recv_ring::RecvRing,
}

/// `SecureConnection` - DTLS connection (`GridMate`: `SecureSocketDriver::Connection`)
///
/// Manages DTLS handshake, encryption, and connection state using manual BIO operations.
/// This struct is Send-safe: OpenSSL 1.1.1+ is thread-safe, and we use Arc for shared state.
/// Type-level state machine: State is encoded in the type parameter
pub struct SecureConnection<State> {
    /// Option enables moving inner out during state transitions
    inner: Option<SecureConnectionInner>,
    _state: std::marker::PhantomData<State>,
}

impl<State> SecureConnection<State> {
    /// Take the inner state for state transitions
    const fn take_inner(&mut self) -> SecureConnectionInner {
        self.inner
            .take()
            .expect("SecureConnectionInner already taken")
    }

    /// Get a reference to the inner state
    const fn inner(&self) -> &SecureConnectionInner {
        self.inner
            .as_ref()
            .expect("SecureConnectionInner not present")
    }

    /// Get a mutable reference to the inner state
    const fn inner_mut(&mut self) -> &mut SecureConnectionInner {
        self.inner
            .as_mut()
            .expect("SecureConnectionInner not present")
    }

    /// Local UDP endpoint owned by this DTLS connection.
    ///
    /// # Errors
    ///
    /// Returns the `getsockname` failure the OS reports for the bound socket.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner().socket.get_ref().local_addr()
    }
}

impl<State> core::fmt::Debug for SecureConnection<State> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecureConnection")
            .field("addr", &self.inner().addr)
            .finish()
    }
}

/// Builder for creating DTLS connections with optional configuration
///
/// # Examples
///
/// ```no_run
/// # use std::path::Path;
/// # use gridmate::driver::secure_connection::SecureConnectionBuilder;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // The CA cert PEM lives wherever your driver wants it — pass any
/// // `&str` containing the PEM-encoded chain.
/// const CA_PEM: &str = "-----BEGIN CERTIFICATE-----\n...";
/// let connection = SecureConnectionBuilder::new("127.0.0.1:8080")
///     .with_cert_pem(CA_PEM)
///     .with_keylog(Path::new("ssl_keylog.txt"))
///     .connect()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "client")]
pub struct SecureConnectionBuilder<'a> {
    addr: &'a str,
    cert_pem: Option<&'a str>,
    keylog_path: Option<&'a Path>,
    local_addr: Option<SocketAddr>,
}

#[cfg(feature = "client")]
impl<'a> SecureConnectionBuilder<'a> {
    /// Create a new builder for the given server address
    #[must_use]
    pub const fn new(addr: &'a str) -> Self {
        Self {
            addr,
            cert_pem: None,
            keylog_path: None,
            local_addr: None,
        }
    }

    /// Set the CA certificate PEM for server verification
    #[must_use]
    pub const fn with_cert_pem(mut self, cert: &'a str) -> Self {
        self.cert_pem = Some(cert);
        self
    }

    /// Enable SSL keylog file for Wireshark decryption
    #[must_use]
    pub const fn with_keylog(mut self, path: &'a Path) -> Self {
        self.keylog_path = Some(path);
        self
    }

    /// Bind the client transport to an explicit local UDP endpoint.
    #[must_use]
    pub const fn with_local_addr(mut self, local_addr: SocketAddr) -> Self {
        self.local_addr = Some(local_addr);
        self
    }

    /// Establish the DTLS connection
    ///
    /// # Errors
    ///
    /// Returns any error [`SecureConnection::connect`] returns.
    pub async fn connect(self) -> Result<SecureConnection<Established>, DriverError> {
        SecureConnection::connect_bound(self.addr, self.cert_pem, self.keylog_path, self.local_addr)
            .await
    }
}

#[cfg(feature = "client")]
impl SecureConnection<Idle> {
    /// Create and establish DTLS connection
    ///
    /// For a more ergonomic API, consider using [`SecureConnectionBuilder`] instead.
    /// Returns `SecureConnection<Established>` after handshake completes.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Address`] if `addr` is not a `host:port` socket
    /// address, [`DriverError::Io`] if the UDP bind, the keylog file open, or a
    /// socket send/recv fails, and [`DriverError::Ssl`] if the `SSL_CTX` cannot
    /// be built (bad cipher list, unparseable CA cert), the handshake times
    /// out, or `SSL_connect` reports a fatal error.
    pub async fn connect(
        addr: &str,
        cert: Option<&str>,
        ssl_keylog_path: Option<&Path>,
    ) -> Result<SecureConnection<Established>, DriverError> {
        Self::connect_bound(addr, cert, ssl_keylog_path, None).await
    }

    async fn connect_bound(
        addr: &str,
        cert: Option<&str>,
        ssl_keylog_path: Option<&Path>,
        local_addr: Option<SocketAddr>,
    ) -> Result<SecureConnection<Established>, DriverError> {
        let server_addr: SocketAddr = addr
            .parse()
            .map_err(|e| DriverError::Address(format!("Invalid address: {e}")))?;

        let local_addr = local_addr.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));
        let std_socket = std::net::UdpSocket::bind(local_addr)?;
        let socket = Arc::new(Async::new(std_socket)?);

        // Setup SSL keylog
        if let Some(keylog_path) = ssl_keylog_path {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(keylog_path)?;

            // Use try_lock to avoid blocking in async context
            if let Ok(mut guard) = SSL_KEYLOG_FILE.try_lock() {
                *guard = Some(file);
            }
        }

        // Create SSL_CTX using DTLSv1_2_method() for DTLS 1.2 record layer version (0xFEFD)
        // Required for compatibility with the original server implementation.
        // Using DTLSv1_2_method() instead of DTLS_method() ensures:
        // - Record layer version is 0xFEFD (DTLS 1.2) instead of 0xFEFF (DTLS 1.0)
        // - Protocol is locked to DTLS 1.2 only
        let mut ctx_builder = unsafe {
            let method = DTLSv1_2_method();
            debug!("Using DTLSv1_2_method() for DTLS 1.2 record layer (0xFEFD)");
            let ssl_ctx_ptr = openssl_sys::SSL_CTX_new(method);
            if ssl_ctx_ptr.is_null() {
                return Err(DriverError::Ssl(
                    "Failed to create SSL_CTX with DTLSv1_2_method".to_string(),
                ));
            }
            // Wrap the raw pointer in SslContextBuilder for safe Rust API using foreign_types
            // SAFETY: ssl_ctx_ptr is a valid, non-null pointer from SSL_CTX_new
            openssl::ssl::SslContextBuilder::from_ptr(ssl_ctx_ptr)
        };
        debug!("SSL_CTX created successfully with DTLS 1.2 method");

        // GridMate: SSL_CTX_set_options(m_sslContext, SSL_OP_NO_QUERY_MTU)
        ctx_builder.set_options(SslOptions::NO_QUERY_MTU);

        // GridMate: SSL_CTX_set_cipher_list(m_sslContext, "ECDHE-RSA-AES256-GCM-SHA384")
        ctx_builder
            .set_cipher_list("ECDHE-RSA-AES256-GCM-SHA384")
            .map_err(|e| DriverError::Ssl(format!("Failed to set cipher list: {e}")))?;

        // GridMate: SSL_CTX_set_verify and load CA certificates
        if let Some(cert_pem) = cert.filter(|s| !s.is_empty()) {
            // Load CA certificate using openssl crate
            let cert = X509::from_pem(cert_pem.as_bytes())
                .map_err(|e| DriverError::Ssl(format!("Failed to parse certificate: {e}")))?;

            let mut store_builder = openssl::x509::store::X509StoreBuilder::new().map_err(|e| {
                DriverError::Ssl(format!("Failed to create certificate store: {e}"))
            })?;
            store_builder
                .add_cert(cert)
                .map_err(|e| DriverError::Ssl(format!("Failed to add certificate: {e}")))?;

            let store = store_builder.build();
            let _ = ctx_builder.set_verify_cert_store(store);
            ctx_builder.set_verify(SslVerifyMode::PEER);
        } else {
            ctx_builder.set_verify(SslVerifyMode::NONE);
        }

        // If keylog enabled, attach callback
        // Use try_lock to check if keylog is enabled (non-blocking)
        let keylog_enabled = SSL_KEYLOG_FILE
            .try_lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|_| true))
            .unwrap_or(false);

        if keylog_enabled {
            ctx_builder.set_keylog_callback(|_ssl, line| {
                // Use try_lock in callback to avoid blocking
                // If lock is contended, we just skip this keylog line (acceptable for debugging)
                if let Ok(mut guard) = SSL_KEYLOG_FILE.try_lock()
                    && let Some(ref mut file) = *guard
                {
                    let _ = writeln!(file, "{line}");
                    let _ = file.flush();
                }
            });
        }

        let ssl_ctx = ctx_builder.build();

        // Note: keylog callback can be set via openssl crate on builder in newer versions; skip if unavailable

        // Create SSL using openssl crate
        let mut ssl = Ssl::new(&ssl_ctx)
            .map_err(|e| DriverError::Ssl(format!("Failed to create SSL: {e}")))?;

        // GridMate: SSL_set_mtu(ssl, 1200)
        ssl.set_mtu(1200)
            .map_err(|e| DriverError::Ssl(format!("Failed to set MTU: {e}")))?;

        // Create BIOs (GridMate: m_inDTLSBuffer = BIO_new(BIO_s_mem()))
        let (read_bio, write_bio) = bio_new_mem_pair()?;

        // Attach BIOs to SSL (GridMate: SSL_set_bio(m_ssl, m_inDTLSBuffer, m_outDTLSBuffer))
        // SAFETY: BIOs are valid pointers from bio_new_mem_pair, SSL takes ownership
        unsafe {
            openssl_sys::SSL_set_bio(ssl.as_ptr(), read_bio.as_ptr(), write_bio.as_ptr());
        }

        // Set SSL to client mode (GridMate pattern)
        ssl.set_connect_state();

        // Remove SNI for clients whose compatibility profile omits it.
        unsafe {
            const SSL_CTRL_SET_TLSEXT_HOSTNAME: i32 = 55;
            openssl_sys::SSL_ctrl(
                ssl.as_ptr(),
                SSL_CTRL_SET_TLSEXT_HOSTNAME,
                0,
                ptr::null_mut(),
            );
        }

        debug!("SSL_CTX and SSL created with DTLSv1_2_method and manual BIOs");

        let inner = SecureConnectionInner {
            socket,
            ssl_ctx,
            ssl,
            read_bio,
            write_bio,
            addr: server_addr,
            state_machine: StateMachine::new_client(5_000),
            recv_buffer: vec![0u8; RECV_BUFFER_LEN].into_boxed_slice(),
            // 32 × 64 KB = 2 MB ring per connection; amortises one
            // allocation across ~32 SSL records.
            decrypt_ring: super::recv_ring::RecvRing::new(32 * 65536, 65536),
        };

        let connection = Self {
            inner: Some(inner),
            _state: std::marker::PhantomData,
        };

        // Establish connection - transitions through handshake states internally
        let established = connection.process_connection().await?;

        Ok(established)
    }

    /// Process connection using `GridMate` state machine
    /// Internal method - transitions through handshake states
    async fn process_connection(mut self) -> Result<SecureConnection<Established>, DriverError> {
        use futures_util::FutureExt as _;

        let result = self
            .inner_mut()
            .state_machine
            .dispatch(ConnectionEvent::Enter);
        self.handle_state_result_internal(result).await?;

        loop {
            if self.inner().state_machine.current_state() == StateMachineState::Established {
                debug!("DTLS handshake complete: {}", self.inner().addr);
                // Safe state transition: take inner and move to new type
                let inner = self.take_inner();
                return Ok(SecureConnection {
                    inner: Some(inner),
                    _state: std::marker::PhantomData,
                });
            }
            if self.inner().state_machine.current_state() == StateMachineState::Disconnected {
                return Err(DriverError::Ssl(
                    "Connection closed during handshake".to_string(),
                ));
            }

            if self.inner().state_machine.check_timeout() {
                trace!("Connection timeout");
                self.inner_mut()
                    .state_machine
                    .dispatch(ConnectionEvent::Update);
                return Err(DriverError::Ssl("Connection timeout".to_string()));
            }

            // Only set timeout for retries if we're not in Established state
            let timeout = if self.inner().state_machine.current_state()
                != StateMachineState::Established
                && self.inner().state_machine.should_retry_handshake()
            {
                Duration::from_millis(0)
            } else {
                Duration::from_millis(100)
            };

            // Get mutable reference to inner for recv operation
            let inner = self.inner_mut();
            futures_util::select! {
                result = inner.socket.recv_from(&mut inner.recv_buffer).fuse() => {
                    match result {
                        Ok((len, from_addr)) => {
                            trace!("Recv {} bytes from {}", len, from_addr);

                            // Extract packet information first to avoid borrow conflicts
                            let inner = self.inner();
                            let packet_slice = &inner.recv_buffer[..len];
                            let packet_type = connection_security::type_to_string(packet_slice);
                            trace!("Packet type: {}", packet_type);

                            // Check if this is a special packet that needs state machine handling
                            let is_hello_verify = connection_security::is_hello_verify_request(packet_slice);
                            let is_hello_request = connection_security::is_hello_request_handshake(packet_slice);
                            let current_state = inner.state_machine.current_state();

                            // Drop the borrow of packet_slice before mutating self
                            let (keep_packet, special_result) = {
                                if is_hello_verify && current_state == StateMachineState::CookieExchange {
                                    // Keep packet for OpenSSL to extract cookie
                                    let result = self.inner_mut().state_machine.dispatch(ConnectionEvent::CookieExchangeCompleted);
                                    (true, Some(result))
                                } else if is_hello_request {
                                    match current_state {
                                        StateMachineState::CookieExchange => {
                                            let result = self.inner_mut().state_machine.dispatch(ConnectionEvent::CookieExchangeCompleted);
                                            (false, Some(result))
                                        }
                                        StateMachineState::Connect => {
                                            (false, Some(StateResult::RecreateSSL))
                                        }
                                        _ => (true, None),
                                    }
                                } else {
                                    (true, None)
                                }
                            };

                            if let Some(result) = special_result {
                                self.handle_state_result_internal(result).await?;
                            }

                            if keep_packet {
                                // Feed to SSL BIO (GridMate: BIO_write(m_inDTLSBuffer, data, dataSize))
                                // Re-borrow recv_buffer here since we've dropped the previous borrow
                                let read_bio = self.inner().read_bio;
                                let recv_buf = &self.inner().recv_buffer[..len];
                                bio_feed(read_bio, recv_buf)?;

                                // Dispatch NewIncomingDgram event
                                let result = self
                                    .inner_mut()
                                    .state_machine
                                    .dispatch(ConnectionEvent::NewIncomingDgram);
                                self.handle_state_result_internal(result).await?;
                            } else {
                                trace!("Packet discarded (GridMate pattern)");
                            }
                        }
                        Err(e) => {
                            trace!("Socket recv_from error: {:?}", e);
                            return Err(DriverError::Io(e));
                        }
                    }
                }
                _ = <Timer as futures_util::FutureExt>::fuse(Timer::after(timeout)) => {
                    // Timeout - only dispatch Update if we're not in Established state (to avoid retries after handshake)
                    if self.inner().state_machine.current_state() != StateMachineState::Established {
                        let result = self.inner_mut().state_machine.dispatch(ConnectionEvent::Update);
                        self.handle_state_result_internal(result).await?;
                    }
                    // If Established, just continue waiting for packets
                }
            }
        }
    }
}

/// State-specific implementations for Established state
impl SecureConnection<Established> {
    /// Write plaintext data to the connection
    ///
    /// Encrypts and sends data over DTLS.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Ssl`] if `data` is longer than `i32::MAX`, or if
    /// `SSL_write` fails with anything other than `WANT_READ` / `WANT_WRITE`
    /// (those return `Ok(0)`), and [`DriverError::Io`] if sending the resulting
    /// encrypted records on the UDP socket fails.
    pub async fn write(&mut self, data: &[u8]) -> Result<usize, DriverError> {
        let len = i32::try_from(data.len()).map_err(|_| {
            DriverError::Ssl(format!("SSL_write length {} exceeds i32", data.len()))
        })?;
        // GridMate pattern: SSL_write to encrypt application data
        let written =
            unsafe { openssl_sys::SSL_write(self.inner().ssl.as_ptr(), data.as_ptr().cast(), len) };

        if written <= 0 {
            let ssl_error =
                unsafe { openssl_sys::SSL_get_error(self.inner().ssl.as_ptr(), written) };
            if !Self::is_retryable_ssl_error(ssl_error) {
                trace!("❌ Fatal SSL error: {}", ssl_error);
                return Err(DriverError::Ssl(format!("SSL_write error: {ssl_error}")));
            }
            return Ok(0);
        }

        // GridMate pattern: QueueDatagrams()
        self.flush_outgoing().await?;
        trace!("Sent {} encrypted bytes", written);
        // `written > 0` was established above, so this never falls back.
        Ok(usize::try_from(written).unwrap_or_default())
    }

    /// Read decrypted data from the connection
    ///
    /// Waits for network I/O and DTLS decryption, returns plaintext when ready.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::ConnectionClosed`] if the datagram stream ends,
    /// [`DriverError::Io`] if the UDP `recv_from` fails, or
    /// [`DriverError::Ssl`] if `SSL_read` reports a non-retryable error.
    pub async fn read(&mut self) -> Result<Bytes, DriverError> {
        use futures_util::StreamExt as _;
        self.read_stream()
            .next()
            .await
            .transpose()?
            .ok_or(DriverError::ConnectionClosed)
    }

    /// Get async stream of decrypted datagrams
    /// Yields plaintext datagrams as they arrive (zero-copy `Bytes`)
    pub fn read_stream(&mut self) -> impl Stream<Item = Result<Bytes, DriverError>> + '_ {
        SecureReceiveStream::new(self)
    }

    /// Check if there's pending decrypted data available
    #[must_use]
    pub fn has_pending_data(&self) -> bool {
        self.bio_pending() > 0
    }

    /// Try to read decrypted data without blocking
    ///
    /// Returns `Ok(Some(data))` if data is available, `Ok(None)` if no data.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Ssl`] if `SSL_read` reports a non-retryable
    /// error; `WANT_READ` / `WANT_WRITE` surface as `Ok(None)`.
    pub fn try_read(&mut self) -> Result<Option<Bytes>, DriverError> {
        if self.bio_pending() == 0 {
            return Ok(None);
        }

        self.read_ssl_data()
    }

    /// Try to receive and decrypt a UDP packet without blocking.
    ///
    /// Uses the underlying non-blocking `std::net::UdpSocket` (already set to
    /// non-blocking by `Async::new()`) via `get_ref()`. Returns `WouldBlock`
    /// as `Ok(None)`.
    ///
    /// Used for batch-draining: after the async select fires on the first
    /// packet, call this in a loop to drain all queued UDP packets before
    /// sending a combined ACK — matching Lumberyard's `UpdateReceive` pattern.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Io`] for any `recv_from` failure other than
    /// `WouldBlock` (which is `Ok(None)`), and [`DriverError::Ssl`] if the
    /// datagram cannot be fed into the read BIO or `SSL_read` reports a
    /// non-retryable error.
    pub fn try_recv_decrypt(&mut self) -> Result<Option<Bytes>, DriverError> {
        let inner = self.inner_mut();

        // Non-blocking recv via the raw std socket (already non-blocking)
        match inner.socket.get_ref().recv_from(&mut inner.recv_buffer) {
            Ok((len, _from_addr)) => {
                trace!("try_recv_decrypt: got {} bytes from socket", len);
                let data = &inner.recv_buffer[..len];
                bio_feed(inner.read_bio, data)?;
                Self::read_ssl_data_internal(&inner.ssl, &mut inner.decrypt_ring)
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(DriverError::Io(e)),
        }
    }

    /// Read decrypted data from SSL (internal helper)
    /// Returns Ok(Some(data)) on success, Ok(None) if `WANT_READ/WANT_WRITE`, Err on fatal error
    fn read_ssl_data(&mut self) -> Result<Option<Bytes>, DriverError> {
        let inner = self.inner_mut();
        Self::read_ssl_data_internal(&inner.ssl, &mut inner.decrypt_ring)
    }

    /// Internal helper to read SSL data (DRY — shared between methods).
    ///
    /// Writes plaintext directly into the `decrypt_ring`'s uninit
    /// tail and splits the result off as an `Arc`-shared `Bytes` —
    /// no `copy_from_slice`. Same zero-copy contract as the
    /// server-side per-peer `SSL_read` path in `multi_peer`.
    fn read_ssl_data_internal(
        ssl: &Ssl,
        decrypt_ring: &mut super::recv_ring::RecvRing,
    ) -> Result<Option<Bytes>, DriverError> {
        let read_result;
        let plaintext = {
            let slot = decrypt_ring.recv_slot();
            // The ring hands out `max_datagram`-sized slots (64 KiB here), so
            // the clamp is unreachable; it only avoids an unchecked narrowing
            // to the C `int` `SSL_read` takes.
            let capacity = i32::try_from(slot.len()).unwrap_or(i32::MAX);
            read_result =
                unsafe { openssl_sys::SSL_read(ssl.as_ptr(), slot.as_mut_ptr().cast(), capacity) };
            if let Ok(read) = usize::try_from(read_result)
                && read > 0
            {
                decrypt_ring.commit(read)
            } else {
                Bytes::new()
            }
        };

        if read_result > 0 {
            trace!("Received {} decrypted bytes", read_result);
            return Ok(Some(plaintext));
        }

        // Check for SSL errors
        let ssl_error = unsafe { openssl_sys::SSL_get_error(ssl.as_ptr(), read_result) };
        if !Self::is_retryable_ssl_error(ssl_error) {
            trace!("❌ Fatal SSL error: {}", ssl_error);
            return Err(DriverError::Ssl(format!("SSL_read error: {ssl_error}")));
        }
        Ok(None)
    }

    /// Close connection gracefully
    ///
    /// Consumes self and returns `SecureConnection`<Disconnected>.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Io`] if sending the final `close_notify` records
    /// on the UDP socket fails. A failing `SSL_shutdown` is traced, not
    /// propagated — the connection is being torn down either way.
    pub async fn close(mut self) -> Result<SecureConnection<Disconnected>, DriverError> {
        // Perform proper SSL shutdown (GridMate: SSL_shutdown)
        debug!("Performing SSL shutdown for {}", self.inner().addr);

        // SSL_shutdown() can return 0 (shutdown not complete) or 1 (shutdown complete)
        // We call it once for unidirectional shutdown
        let shutdown_result = unsafe { openssl_sys::SSL_shutdown(self.inner().ssl.as_ptr()) };

        if shutdown_result < 0 {
            // Check for SSL errors
            let ssl_error = unsafe {
                openssl_sys::SSL_get_error(self.inner().ssl.as_ptr(), shutdown_result as i32)
            };
            if ssl_error != openssl_sys::SSL_ERROR_ZERO_RETURN
                && ssl_error != openssl_sys::SSL_ERROR_SYSCALL
            {
                trace!(
                    "SSL shutdown error for {}: SSL error {}",
                    self.inner().addr,
                    ssl_error
                );
            }
        } else if shutdown_result == 1 {
            debug!("SSL shutdown complete for {}", self.inner().addr);
        }

        // Drain any remaining data from write BIO and send it
        self.flush_encrypted_data().await?;

        debug!("DTLS connection closed for {}", self.inner().addr);

        // Safe state transition: take inner and move to new type
        let inner = self.take_inner();
        Ok(SecureConnection {
            inner: Some(inner),
            _state: std::marker::PhantomData,
        })
    }
}

/// Stream of decrypted datagrams from DTLS connection
///
/// The recv scratch buffer is a heap `Vec` rather than an inline
/// `[u8; 65536]`: the future owns it for the duration of the `recv_from`, and
/// materialising a 64 KiB array on the stack before moving it into the boxed
/// future is a stack-overflow hazard in debug builds.
type SocketRecvFuture<'a> = Pin<
    Box<
        dyn std::future::Future<Output = Result<(usize, SocketAddr, Vec<u8>), std::io::Error>>
            + Send
            + 'a,
    >,
>;

struct SecureReceiveStream<'a> {
    connection: &'a mut SecureConnection<Established>,
    socket_fut: Option<SocketRecvFuture<'a>>,
}

impl<'a> SecureReceiveStream<'a> {
    fn new(connection: &'a mut SecureConnection<Established>) -> Self {
        Self {
            connection,
            socket_fut: None,
        }
    }
}

impl Stream for SecureReceiveStream<'_> {
    type Item = Result<Bytes, DriverError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = unsafe { self.as_mut().get_unchecked_mut() };

        // Check if there's already decrypted data in the BIO
        let pending = this.connection.inner().read_bio.pending();

        trace!("[SecureReceiveStream] BIO_pending: {} bytes", pending);

        if pending > 0 {
            trace!("[SecureReceiveStream] Attempting to read decrypted data from BIO");

            // Use shared SSL read helper (DRY)
            let inner = this.connection.inner_mut();
            match SecureConnection::<Established>::read_ssl_data_internal(
                &inner.ssl,
                &mut inner.decrypt_ring,
            ) {
                Ok(Some(data)) => {
                    trace!(
                        "[SecureReceiveStream] Successfully read {} bytes, returning Ready",
                        data.len()
                    );
                    return Poll::Ready(Some(Ok(data)));
                }
                Ok(None) => {
                    trace!(
                        "[SecureReceiveStream] SSL_read returned None (WANT_READ/WANT_WRITE), need more encrypted data"
                    );
                    // WANT_READ/WANT_WRITE - need more encrypted data, fall through to socket read
                }
                Err(e) => {
                    trace!(
                        "[SecureReceiveStream] SSL_read error: {:?}, returning Ready(Err)",
                        e
                    );
                    return Poll::Ready(Some(Err(e)));
                }
            }
        }

        // Need more encrypted DTLS data from socket. If we don't have a
        // pending socket future yet, build one. Either way, drive it through
        // a single `poll(cx)` so the inner future registers its waker with
        // the current task — without that, returning `Pending` from a path
        // that never polled the future leaves the executor parked with no
        // wake source. (Production hides this with periodic timers in
        // `carrier_thread::run`'s `select!`; tests don't have that.)
        let fut = if let Some(fut) = this.socket_fut.as_mut() {
            trace!("[SecureReceiveStream] Polling existing socket future");
            fut
        } else {
            let socket = this.connection.inner().socket.clone();
            let mut local_buf = vec![0u8; RECV_BUFFER_LEN];
            let new_fut = async move {
                socket
                    .recv_from(&mut local_buf)
                    .await
                    .map(move |(len, addr)| (len, addr, local_buf))
            };
            this.socket_fut = Some(Box::pin(new_fut));
            this.socket_fut.as_mut().expect("just stored")
        };

        match fut.as_mut().poll(cx) {
            Poll::Ready(Ok((len, _from_addr, buf))) => {
                this.socket_fut = None;
                // Copy received data to connection's buffer
                let inner = this.connection.inner_mut();
                (*inner.recv_buffer)[..len].copy_from_slice(&buf[..len]);
                trace!("← Recv {} bytes (encrypted DTLS)", len);

                let packet_type = connection_security::type_to_string(&(*inner.recv_buffer)[..len]);
                trace!("   Packet type: {}", packet_type);

                // Feed encrypted data into BIO and immediately try to extract
                // a decrypted record — without this, returning `Pending` leaves
                // the caller waiting on a wake that nothing schedules.
                if let Err(e) = bio_feed(inner.read_bio, &inner.recv_buffer[..len]) {
                    return Poll::Ready(Some(Err(e)));
                }

                match SecureConnection::<Established>::read_ssl_data_internal(
                    &inner.ssl,
                    &mut inner.decrypt_ring,
                ) {
                    Ok(Some(data)) => Poll::Ready(Some(Ok(data))),
                    // SSL_read needs more bytes (e.g. handshake fragment).
                    // Re-arm by waking ourselves so the executor re-polls and
                    // we issue another `recv_from`.
                    Ok(None) => {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Err(e) => Poll::Ready(Some(Err(e))),
                }
            }
            Poll::Ready(Err(e)) => {
                this.socket_fut = None;
                Poll::Ready(Some(Err(DriverError::Io(e))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl SecureConnection<Established> {
    /// Flush outgoing encrypted data to socket (`GridMate`: `QueueDatagrams`)
    /// Only available in Established state
    async fn flush_outgoing(&mut self) -> Result<(), DriverError> {
        self.flush_encrypted_data().await
    }

    /// Address of the peer this connection is talking to. For client
    /// connections this is the server we connected to; for
    /// server-accepted connections this is the client's source.
    #[must_use]
    pub const fn peer_addr(&self) -> SocketAddr {
        self.inner().addr
    }
}

impl<State> SecureConnection<State> {
    /// Get socket reference
    #[must_use]
    pub const fn socket(&self) -> &Arc<Async<UdpSocket>> {
        &self.inner().socket
    }

    /// Get number of pending bytes in read BIO
    fn bio_pending(&self) -> usize {
        self.inner().read_bio.pending()
    }

    /// Check if SSL error is retryable (`WANT_READ` or `WANT_WRITE`)
    const fn is_retryable_ssl_error(ssl_error: i32) -> bool {
        ssl_error == SSL_ERROR_WANT_READ || ssl_error == SSL_ERROR_WANT_WRITE
    }

    /// Flush encrypted data from write BIO to socket (DRY helper)
    async fn flush_encrypted_data(&mut self) -> Result<(), DriverError> {
        let inner = self.inner_mut();
        let pending = bio_drain(inner.write_bio);
        if !pending.is_empty() {
            trace!("→ Sending {} bytes (encrypted)", pending.len());
            inner.socket.send_to(pending.as_ref(), inner.addr).await?;
        }
        Ok(())
    }

    /// Handle state machine results (internal method during handshake)
    #[cfg(feature = "client")]
    async fn handle_state_result_internal(
        &mut self,
        result: StateResult,
    ) -> Result<(), DriverError> {
        let mut current_result = result;
        loop {
            match current_result {
                StateResult::Handled | StateResult::Unhandled => return Ok(()),
                StateResult::Transition => {
                    // After transition, dispatch Enter to trigger actions in new state
                    current_result = self
                        .inner_mut()
                        .state_machine
                        .dispatch(ConnectionEvent::Enter);
                    // Fall through to the next loop iteration to process the
                    // Enter result.
                }
                current => {
                    // Handle all other results directly
                    return self.handle_state_result_direct_internal(current).await;
                }
            }
        }
    }

    /// Handle a single state result (non-recursive, internal during handshake)
    #[cfg(feature = "client")]
    async fn handle_state_result_direct_internal(
        &mut self,
        result: StateResult,
    ) -> Result<(), DriverError> {
        match result {
            StateResult::SslConnect => {
                debug!("→ SSL_connect()");
                let ret = unsafe { openssl_sys::SSL_connect(self.inner().ssl.as_ptr()) };
                if ret == 1 {
                    debug!("✅ SSL_connect succeeded!");
                    self.inner_mut()
                        .state_machine
                        .transition(StateMachineState::Established);
                } else {
                    self.handle_ssl_error(ret)?;
                }
                // Flush during handshake
                self.flush_encrypted_data().await?;
                Ok(())
            }
            StateResult::RecreateSSL => {
                debug!("→ RecreateSSL()");
                // GridMate: DestroySSL() + CreateSSL(m_sslContext) + sm.Transition(CS_CONNECT)
                Self::destroy_ssl();
                self.create_ssl()?;
                // GridMate's CS_CONNECT EnterEventId: SSL_connect(m_ssl)
                debug!("→ SSL_connect() (Connect state Enter)");
                let ret = unsafe { openssl_sys::SSL_connect(self.inner().ssl.as_ptr()) };
                if ret == 1 {
                    debug!("✅ SSL_connect succeeded immediately!");
                    self.inner_mut()
                        .state_machine
                        .transition(StateMachineState::Established);
                } else {
                    self.handle_ssl_error(ret)?;
                }
                // Flush during handshake
                self.flush_encrypted_data().await?;
                Ok(())
            }
            StateResult::SendHelloRequest => {
                debug!("→ SendHelloRequest()");
                // Server-side only, not implemented for client
                Ok(())
            }
            StateResult::SslRead | StateResult::SslWrite => {
                // Handled in send/recv methods
                Ok(())
            }
            #[cfg(feature = "server")]
            StateResult::SslAccept => {
                // Server-side only - not used in client connections
                Ok(())
            }
            #[cfg(not(feature = "server"))]
            StateResult::SslAccept => {
                // Not used in client mode
                Ok(())
            }
            StateResult::ForceDtlsTimeout => {
                debug!("→ ForceDTLSTimeout()");
                unsafe {
                    const DTLS_CTRL_HANDLE_TIMEOUT: i32 = 74;
                    openssl_sys::SSL_ctrl(
                        self.inner().ssl.as_ptr(),
                        DTLS_CTRL_HANDLE_TIMEOUT,
                        0,
                        ptr::null_mut(),
                    );
                }
                // Flush during handshake
                self.flush_encrypted_data().await?;
                Ok(())
            }
            StateResult::Handled | StateResult::Unhandled | StateResult::Transition => {
                // These should have been handled by the loop above
                Ok(())
            }
        }
    }

    /// Handle SSL errors (`GridMate`'s `HandleSSLError` pattern)
    #[cfg(feature = "client")]
    fn handle_ssl_error(&mut self, result: i32) -> Result<(), DriverError> {
        let ssl_error = unsafe { openssl_sys::SSL_get_error(self.inner().ssl.as_ptr(), result) };

        if !Self::is_retryable_ssl_error(ssl_error) {
            // Get detailed error code (GridMate pattern)
            let err_code = unsafe { openssl_sys::ERR_get_error() };

            trace!(
                "❌ Fatal SSL error: {} (OpenSSL error code: 0x{:x})",
                ssl_error, err_code
            );
            self.inner_mut()
                .state_machine
                .transition(StateMachineState::Disconnected);
            return Err(DriverError::Ssl(format!(
                "SSL error {ssl_error} (code: 0x{err_code:x})"
            )));
        }
        Ok(())
    }

    /// Destroy SSL context (`GridMate`'s `DestroySSL` pattern)
    ///
    /// Takes no receiver: the `Ssl` object is freed by its own `Drop` when the
    /// replacement is stored in `create_ssl`, so this is only the trace point
    /// that marks the teardown.
    #[cfg(feature = "client")]
    fn destroy_ssl() {
        // Note: BIOs are owned by SSL and will be freed when SSL is freed
        // Just need to drop the SSL object
        // The Ssl struct will handle cleanup via Drop
        debug!("Destroying SSL");
    }

    /// Create new SSL context (`GridMate`'s `CreateSSL` pattern)
    #[cfg(feature = "client")]
    fn create_ssl(&mut self) -> Result<(), DriverError> {
        let inner = self.inner_mut();
        // GridMate pattern: m_ssl = SSL_new(sslContext)
        let mut ssl = Ssl::new(&inner.ssl_ctx)
            .map_err(|e| DriverError::Ssl(format!("Failed to create SSL: {e}")))?;

        ssl.set_mtu(1200).ok();

        // Create new BIOs
        let (read_bio, write_bio) = bio_new_mem_pair()?;

        // Attach BIOs to SSL
        unsafe {
            openssl_sys::SSL_set_bio(ssl.as_ptr(), read_bio.as_ptr(), write_bio.as_ptr());
        }

        // Set SSL to client mode
        ssl.set_connect_state();

        // Remove SNI extension
        unsafe {
            const SSL_CTRL_SET_TLSEXT_HOSTNAME: i32 = 55;
            openssl_sys::SSL_ctrl(
                ssl.as_ptr(),
                SSL_CTRL_SET_TLSEXT_HOSTNAME,
                0,
                ptr::null_mut(),
            );
        }

        inner.ssl = ssl;
        inner.read_bio = read_bio;
        inner.write_bio = write_bio;

        debug!("Created new SSL with fresh BIOs (GridMate pattern)");
        Ok(())
    }
}

// SAFETY: OpenSSL 1.1.1+ is thread-safe when used correctly.
// - SSL_CTX, SSL, and BIO are wrapped in safe types from openssl crate
// - SecureConnection is wrapped in Arc<async_lock::RwLock<...>> which provides synchronization
// - async_lock::RwLock allows holding lock across .await points safely
// - All mutable access goes through RwLock, ensuring exclusive access
// - OpenSSL's global state is internally synchronized (CRYPTO_THREAD_lock_new, etc.)
unsafe impl<State> Send for SecureConnection<State> {}
unsafe impl<State> Sync for SecureConnection<State> {}

// ============================================================================
// Server-side DTLS listener
// ============================================================================

/// DTLS server-side listener.
///
/// Owns the bound UDP socket and the shared `SSL_CTX` configured with the
/// server's RSA cert + key. Each [`accept`](Self::accept) call drives a
/// single peer's `SSL_accept` handshake to completion and returns the
/// established connection.
///
/// This is the minimum viable single-peer accept path. Multi-peer demux
/// (one socket fanning to many in-flight handshakes) is a follow-up.
#[cfg(feature = "server")]
pub struct SecureSocketListener {
    socket: Arc<Async<UdpSocket>>,
    ssl_ctx: SslContext,
}

#[cfg(feature = "server")]
impl SecureSocketListener {
    /// Bind a DTLS listener to `addr` (`host:port`, e.g. `127.0.0.1:24083`).
    ///
    /// `cert_pem` and `key_pem` are PEM-encoded RSA cert + private key; the
    /// cert must be RSA so the cipher `ECDHE-RSA-AES256-GCM-SHA384` (the
    /// only one our `SecureConnection` client offers) can be negotiated.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Address`] if `addr` is not a `host:port` socket
    /// address, [`DriverError::Io`] if the UDP bind fails (port in use, address
    /// not available), and [`DriverError::Ssl`] if the cert or key cannot be
    /// parsed or does not match the configured cipher.
    // `bind` has no await today, but it is half of the `bind` / `accept` pair
    // and every caller awaits both; dropping `async` is a public API break
    // rippling out to callers this task does not own.
    #[allow(clippy::unused_async)]
    pub async fn bind(addr: &str, cert_pem: &str, key_pem: &str) -> Result<Self, DriverError> {
        let bind_addr: SocketAddr = addr
            .parse()
            .map_err(|e| DriverError::Address(format!("Invalid address: {e}")))?;

        let std_socket = UdpSocket::bind(bind_addr)?;
        let socket = Arc::new(Async::new(std_socket)?);

        let ssl_ctx = build_server_ssl_ctx(cert_pem, key_pem)?;

        Ok(Self { socket, ssl_ctx })
    }

    /// Local socket address. Useful when binding to port 0 to discover the
    /// OS-chosen port for tests.
    ///
    /// # Errors
    ///
    /// Returns the `getsockname` failure the OS reports for the bound socket.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.get_ref().local_addr()
    }

    /// Accept a single DTLS handshake from the next peer that contacts this
    /// listener. Consumes the listener — the established connection takes
    /// over the bound socket.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::Io`] if a `recv_from` or `send_to` on the shared
    /// socket fails, and [`DriverError::Ssl`] if `Ssl::new` fails, a BIO cannot
    /// be allocated, or `SSL_accept` reports anything other than `WANT_READ` /
    /// `WANT_WRITE` — the OpenSSL error queue is drained into the message.
    pub async fn accept(self) -> Result<SecureConnection<Established>, DriverError> {
        let Self { socket, ssl_ctx } = self;
        let mut recv_buffer: Box<[u8]> = vec![0u8; RECV_BUFFER_LEN].into_boxed_slice();

        // Block until the first datagram arrives so we know the peer address.
        let (len, peer_addr) = socket.recv_from(&mut recv_buffer).await?;
        debug!("← Server: first datagram {} bytes from {}", len, peer_addr);

        let mut ssl = Ssl::new(&ssl_ctx).map_err(|e| DriverError::Ssl(format!("Ssl::new: {e}")))?;
        ssl.set_mtu(1200).ok();

        let (read_bio, write_bio) = bio_new_mem_pair()?;
        unsafe {
            openssl_sys::SSL_set_bio(ssl.as_ptr(), read_bio.as_ptr(), write_bio.as_ptr());
        }
        ssl.set_accept_state();

        // Seed SSL with the ClientHello we just received.
        bio_feed(read_bio, &recv_buffer[..len])?;

        loop {
            let ret = unsafe { openssl_sys::SSL_accept(ssl.as_ptr()) };

            // Always flush whatever response SSL produced.
            let outgoing = bio_drain(write_bio);
            if !outgoing.is_empty() {
                trace!(
                    "→ Server: send {} bytes to {} during handshake",
                    outgoing.len(),
                    peer_addr
                );
                socket.send_to(outgoing.as_ref(), peer_addr).await?;
            }

            if ret == 1 {
                debug!("✅ Server: SSL_accept complete for {}", peer_addr);
                break;
            }

            let ssl_error = unsafe { openssl_sys::SSL_get_error(ssl.as_ptr(), ret) };
            if ssl_error != SSL_ERROR_WANT_READ && ssl_error != SSL_ERROR_WANT_WRITE {
                // Drain the full ERR queue (see `multi_peer::accept_handshake`
                // for rationale — opaque hex codes turn into actionable
                // OpenSSL strings like `tlsv1 alert decrypt error`).
                let stack = openssl::error::ErrorStack::get();
                let joined = if stack.errors().is_empty() {
                    "<empty ERR queue>".to_string()
                } else {
                    stack
                        .errors()
                        .iter()
                        .map(|e| format!("{e}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                };
                return Err(DriverError::Ssl(format!(
                    "SSL_accept error: {ssl_error}: {joined}"
                )));
            }

            // Wait for the next datagram from any peer; for the single-peer
            // tracer we trust loopback ordering.
            let (len, _from) = socket.recv_from(&mut recv_buffer).await?;
            bio_feed(read_bio, &recv_buffer[..len])?;
        }

        let inner = SecureConnectionInner {
            socket,
            #[cfg(feature = "client")]
            ssl_ctx,
            ssl,
            read_bio,
            write_bio,
            addr: peer_addr,
            #[cfg(feature = "client")]
            state_machine: StateMachine::new_server(5_000),
            recv_buffer,
            decrypt_ring: super::recv_ring::RecvRing::new(32 * 65536, 65536),
        };
        Ok(SecureConnection {
            inner: Some(inner),
            _state: std::marker::PhantomData,
        })
    }
}

#[cfg(feature = "server")]
pub(crate) fn build_server_ssl_ctx(
    cert_pem: &str,
    key_pem: &str,
) -> Result<SslContext, DriverError> {
    let mut ctx_builder = unsafe {
        let method = DTLSv1_2_method();
        let ssl_ctx_ptr = openssl_sys::SSL_CTX_new(method);
        if ssl_ctx_ptr.is_null() {
            return Err(DriverError::Ssl(
                "SSL_CTX_new failed for server".to_string(),
            ));
        }
        openssl::ssl::SslContextBuilder::from_ptr(ssl_ctx_ptr)
    };
    ctx_builder.set_options(SslOptions::NO_QUERY_MTU);
    ctx_builder
        .set_cipher_list("ECDHE-RSA-AES256-GCM-SHA384")
        .map_err(|e| DriverError::Ssl(format!("set_cipher_list: {e}")))?;

    let cert = X509::from_pem(cert_pem.as_bytes())
        .map_err(|e| DriverError::Ssl(format!("X509::from_pem: {e}")))?;
    ctx_builder
        .set_certificate(&cert)
        .map_err(|e| DriverError::Ssl(format!("set_certificate: {e}")))?;

    let pkey = openssl::pkey::PKey::private_key_from_pem(key_pem.as_bytes())
        .map_err(|e| DriverError::Ssl(format!("private_key_from_pem: {e}")))?;
    ctx_builder
        .set_private_key(&pkey)
        .map_err(|e| DriverError::Ssl(format!("set_private_key: {e}")))?;

    ctx_builder.set_verify(SslVerifyMode::NONE);

    Ok(ctx_builder.build())
}
