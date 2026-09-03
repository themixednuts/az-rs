// GridMate SecureSocketDriver State Machine
// Implements stateless → stateful DTLS handshake pattern

use std::time::Instant;

/// Connection States
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Top-level state
    Top,
    /// Active parent state
    Active,

    // Server-side states
    #[cfg(feature = "server")]
    /// Server: Send `HelloRequest` and wait for stateful handshake
    SendHelloRequest,
    #[cfg(feature = "server")]
    /// Server: Accepting SSL connection
    Accept,

    // Client-side states
    #[cfg(feature = "client")]
    /// Client: Performing cookie exchange (stateless)
    CookieExchange,
    #[cfg(feature = "client")]
    /// Client: Connecting with SSL (stateful)
    Connect,

    /// Connection established and ready for data
    Established,
    /// Connection disconnected
    Disconnected,
}

/// Connection Events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// State machine entry event
    Enter,
    /// State machine exit event
    Exit,
    /// Update event (called periodically)
    Update,
    /// New incoming DTLS datagram received
    NewIncomingDgram,
    /// New outgoing DTLS datagram to send
    NewOutgoingDgram,
    /// Server: Received ClientHello(1) with valid cookie, start stateful handshake
    StatefulHandshake,
    /// Client: Cookie exchange completed (received `HelloRequest`)
    CookieExchangeCompleted,
}

/// State Machine Context
pub struct StateMachine {
    current_state: ConnectionState,
    previous_state: ConnectionState,

    // Client state timers (GridMate pattern)
    creation_time: Instant,
    #[allow(dead_code)]
    next_hello_request_resend: Instant,
    #[allow(dead_code)]
    hello_request_resend_interval: std::time::Duration,
    next_handshake_retry: Instant,
    /// Current handshake retry interval
    handshake_retry_interval: std::time::Duration,

    // Configuration
    timeout_ms: u64,
}

impl StateMachine {
    /// Create new state machine for client
    #[cfg(feature = "client")]
    #[must_use]
    pub fn new_client(timeout_ms: u64) -> Self {
        let now = Instant::now();
        let retry_interval_ms = timeout_ms / 10; // 10 retry attempts
        Self {
            current_state: ConnectionState::CookieExchange,
            previous_state: ConnectionState::Top,
            creation_time: now,
            next_hello_request_resend: now,
            hello_request_resend_interval: std::time::Duration::from_millis(100),
            handshake_retry_interval: std::time::Duration::from_millis(retry_interval_ms),
            next_handshake_retry: now + std::time::Duration::from_millis(retry_interval_ms),
            timeout_ms,
        }
    }

    /// Create new state machine for server
    #[cfg(feature = "server")]
    #[must_use]
    pub fn new_server(timeout_ms: u64) -> Self {
        let now = Instant::now();
        Self {
            current_state: ConnectionState::SendHelloRequest,
            previous_state: ConnectionState::Top,
            creation_time: now,
            next_hello_request_resend: now,
            hello_request_resend_interval: std::time::Duration::from_millis(100),
            handshake_retry_interval: std::time::Duration::from_millis(30),
            next_handshake_retry: now + std::time::Duration::from_millis(30),
            timeout_ms,
        }
    }

    /// Get current state
    #[must_use]
    pub const fn current_state(&self) -> ConnectionState {
        self.current_state
    }

    /// Transition to new state
    pub const fn transition(&mut self, new_state: ConnectionState) {
        self.previous_state = self.current_state;
        self.current_state = new_state;
    }

    /// Dispatch event to current state
    pub fn dispatch(&mut self, event: ConnectionEvent) -> StateResult {
        match self.current_state {
            ConnectionState::Top => StateResult::Unhandled,
            ConnectionState::Active => self.on_state_active(event),

            #[cfg(feature = "server")]
            ConnectionState::SendHelloRequest => self.on_state_send_hello_request(event),
            #[cfg(feature = "server")]
            ConnectionState::Accept => Self::on_state_accept(event),

            #[cfg(feature = "client")]
            ConnectionState::CookieExchange => self.on_state_cookie_exchange(event),
            #[cfg(feature = "client")]
            ConnectionState::Connect => self.on_state_connect(event),

            ConnectionState::Established => Self::on_state_established(event),
            ConnectionState::Disconnected => Self::on_state_disconnected(event),
        }
    }

    /// Check if connection has timed out
    #[must_use]
    pub fn check_timeout(&self) -> bool {
        // Compare in `u128` (what `as_millis` returns) rather than
        // truncating it: a long-lived connection can outrun `u64`
        // milliseconds only in theory, but the widening comparison
        // costs nothing and cannot wrap a timeout into "not expired".
        self.creation_time.elapsed().as_millis() > u128::from(self.timeout_ms)
    }

    /// Check if it's time to retry handshake
    #[must_use]
    pub fn should_retry_handshake(&self) -> bool {
        Instant::now() >= self.next_handshake_retry
    }

    /// Update retry timer
    pub fn update_retry_timer(&mut self) {
        // GridMate: timeout / kSSLHandshakeAttempts (where kSSLHandshakeAttempts = 10)
        let retry_interval_ms = self.timeout_ms / 10;
        self.handshake_retry_interval = std::time::Duration::from_millis(retry_interval_ms);
        self.next_handshake_retry = Instant::now() + self.handshake_retry_interval;
    }

    // ========================================================================
    // State Handlers
    // ========================================================================

    fn on_state_active(&mut self, event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter => {
                self.creation_time = Instant::now();
                StateResult::Handled
            }
            ConnectionEvent::Update => {
                // Check timeout (GridMate pattern)
                if self.check_timeout() {
                    self.transition(ConnectionState::Disconnected);
                    StateResult::Transition
                } else {
                    StateResult::Unhandled
                }
            }
            _ => StateResult::Unhandled,
        }
    }

    #[cfg(feature = "server")]
    fn on_state_send_hello_request(&mut self, event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter => {
                self.hello_request_resend_interval = std::time::Duration::from_millis(100);
                self.next_hello_request_resend =
                    Instant::now() + self.hello_request_resend_interval;
                StateResult::SendHelloRequest
            }
            ConnectionEvent::StatefulHandshake => {
                self.transition(ConnectionState::Accept);
                StateResult::Transition
            }
            ConnectionEvent::Update => {
                if Instant::now() > self.next_hello_request_resend {
                    self.next_hello_request_resend =
                        Instant::now() + self.hello_request_resend_interval;
                    self.hello_request_resend_interval *= 2;
                    self.hello_request_resend_interval = std::cmp::min(
                        self.hello_request_resend_interval,
                        std::time::Duration::from_secs(1),
                    );
                    StateResult::SendHelloRequest
                } else {
                    StateResult::Unhandled
                }
            }
            _ => StateResult::Unhandled,
        }
    }

    /// Every event that means "there is handshake work to do" maps to
    /// the same `SSL_accept` drive, so entry, inbound datagram and tick
    /// share one arm. Like the two terminal handlers below, this state
    /// mutates nothing and takes no receiver.
    #[cfg(feature = "server")]
    const fn on_state_accept(event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter
            | ConnectionEvent::NewIncomingDgram
            | ConnectionEvent::Update => StateResult::SslAccept,
            _ => StateResult::Unhandled,
        }
    }

    #[cfg(feature = "client")]
    fn on_state_cookie_exchange(&mut self, event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter => {
                // Send first ClientHello (without cookie)
                let retry_interval_ms = self.timeout_ms / 10;
                self.handshake_retry_interval = std::time::Duration::from_millis(retry_interval_ms);
                self.next_handshake_retry = Instant::now() + self.handshake_retry_interval;
                StateResult::SslConnect
            }
            ConnectionEvent::CookieExchangeCompleted => {
                // Received HelloVerifyRequest, transition to Connect
                self.transition(ConnectionState::Connect);
                StateResult::Transition
            }
            ConnectionEvent::NewIncomingDgram => StateResult::SslConnect,
            ConnectionEvent::Update => {
                if self.should_retry_handshake() {
                    self.update_retry_timer();
                    StateResult::ForceDtlsTimeout
                } else {
                    StateResult::Unhandled
                }
            }
            _ => StateResult::Unhandled,
        }
    }

    #[cfg(feature = "client")]
    fn on_state_connect(&mut self, event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter => {
                // Call SSL_connect to send ClientHello with cookie
                let retry_interval_ms = self.timeout_ms / 10;
                self.handshake_retry_interval = std::time::Duration::from_millis(retry_interval_ms);
                self.next_handshake_retry = Instant::now() + self.handshake_retry_interval;
                StateResult::SslConnect
            }
            ConnectionEvent::NewIncomingDgram => StateResult::SslConnect,
            ConnectionEvent::Update => {
                if self.should_retry_handshake() {
                    self.update_retry_timer();
                    StateResult::ForceDtlsTimeout
                } else {
                    StateResult::Unhandled
                }
            }
            _ => StateResult::Unhandled,
        }
    }

    /// `Established` and `Disconnected` are terminal for the state
    /// machine's own bookkeeping — they map events straight to a
    /// [`StateResult`] for the caller to act on and never mutate
    /// timers or transition, so they take no receiver.
    const fn on_state_established(event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter => StateResult::Handled,
            ConnectionEvent::NewIncomingDgram => StateResult::SslRead,
            ConnectionEvent::NewOutgoingDgram => StateResult::SslWrite,
            _ => StateResult::Unhandled,
        }
    }

    const fn on_state_disconnected(event: ConnectionEvent) -> StateResult {
        match event {
            ConnectionEvent::Enter => StateResult::Handled,
            _ => StateResult::Unhandled,
        }
    }
}

/// State handler result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateResult {
    /// Event was handled
    Handled,
    /// Event was not handled
    Unhandled,
    /// Transition occurred
    Transition,
    /// Send `HelloRequest` packet
    SendHelloRequest,
    /// Call `SSL_accept`
    SslAccept,
    /// Call `SSL_connect`
    SslConnect,
    /// Call `SSL_read`
    SslRead,
    /// Call `SSL_write`
    SslWrite,
    /// Force DTLS timeout
    ForceDtlsTimeout,
    /// Recreate SSL context
    RecreateSSL,
}
