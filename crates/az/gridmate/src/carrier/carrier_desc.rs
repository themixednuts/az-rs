// CarrierDesc - Configuration for Carrier creation
// Following GridMate CarrierDesc struct

#[cfg(feature = "transport")]
use std::time::Duration;

/// Carrier protocol settings selected by the embedding project or runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CarrierProtocolProfile {
    /// Handshake protocol version.
    pub version: u32,
    /// Whether post-handshake user datagrams should attempt LZ4 compression.
    pub send_compressed: bool,
}

impl CarrierProtocolProfile {
    /// Construct an explicit carrier protocol profile.
    #[must_use]
    pub const fn new(version: u32, send_compressed: bool) -> Self {
        Self {
            version,
            send_compressed,
        }
    }
}

impl Default for CarrierProtocolProfile {
    fn default() -> Self {
        Self::new(1, false)
    }
}

/// Carrier configuration descriptor (`GridMate`: `CarrierDesc`)
/// Rust idiom: Builder pattern with defaults via Default trait
#[derive(Clone)]
pub struct CarrierDesc {
    /// Server address to connect to
    pub server_address: String,

    /// SSL keylog path (for Wireshark)
    pub ssl_keylog_path: Option<std::path::PathBuf>,

    /// CA certificate (PEM string or path)
    pub ca_cert: Option<String>,

    /// Connection timeout in milliseconds (`GridMate`: `m_connectionTimeoutMS` = 5000)
    pub connection_timeout_ms: u32,

    /// Whether established peers are disconnected after the connection timeout.
    /// Mirrors `GridMate`'s default-on `m_enableDisconnectDetection`.
    pub enable_disconnect_detection: bool,

    /// Thread update interval in milliseconds (`GridMate`: `m_threadUpdateTimeMS` = 30)
    pub thread_update_time_ms: u32,

    /// Connection retry base interval (`GridMate`: `m_connectionRetryIntervalBase` = 10)
    pub connection_retry_interval_base: u64,

    /// Connection retry max interval (`GridMate`: `m_connectionRetryIntervalMax` = 1000)
    pub connection_retry_interval_max: u64,

    /// Clock sync interval in milliseconds (`GridMate`: disabled by default)
    pub clock_sync_interval: u32,

    /// Protocol version (`GridMate`: `m_version` = 1).
    pub version: u32,

    /// Whether post-handshake user datagrams should attempt LZ4 compression.
    pub send_compressed: bool,
}

impl Default for CarrierDesc {
    fn default() -> Self {
        let protocol = CarrierProtocolProfile::default();
        Self {
            server_address: String::new(),
            ssl_keylog_path: None,
            ca_cert: None,
            connection_timeout_ms: 5000,
            enable_disconnect_detection: true,
            thread_update_time_ms: 10,
            connection_retry_interval_base: 10,
            connection_retry_interval_max: 1000,
            clock_sync_interval: 0,
            version: protocol.version,
            send_compressed: protocol.send_compressed,
        }
    }
}

impl CarrierDesc {
    /// Create new descriptor with server address
    pub fn new(server_address: impl Into<String>) -> Self {
        Self {
            server_address: server_address.into(),
            ..Default::default()
        }
    }

    /// Builder pattern: Set SSL keylog path
    pub fn with_ssl_keylog(&mut self, path: impl Into<std::path::PathBuf>) -> &mut Self {
        self.ssl_keylog_path = Some(path.into());
        self
    }

    /// Builder pattern: Set CA certificate
    pub fn with_ca_cert(&mut self, cert: impl Into<String>) -> &mut Self {
        self.ca_cert = Some(cert.into());
        self
    }

    /// Builder pattern: Set connection timeout
    pub const fn with_timeout(&mut self, timeout_ms: u32) -> &mut Self {
        self.connection_timeout_ms = timeout_ms;
        self
    }

    /// Enable or disable source-compatible disconnect detection.
    pub const fn with_disconnect_detection(&mut self, enabled: bool) -> &mut Self {
        self.enable_disconnect_detection = enabled;
        self
    }

    /// Settings consumed by one carrier driver after transport setup.
    #[cfg(feature = "transport")]
    pub(crate) fn runtime_config(&self) -> CarrierRuntimeConfig {
        CarrierRuntimeConfig {
            protocol: self.protocol_profile(),
            disconnect_timeout: self
                .enable_disconnect_detection
                .then(|| Duration::from_millis(u64::from(self.connection_timeout_ms))),
        }
    }

    /// Builder pattern: select the carrier wire profile.
    pub const fn with_protocol_profile(&mut self, protocol: CarrierProtocolProfile) -> &mut Self {
        self.version = protocol.version;
        self.send_compressed = protocol.send_compressed;
        self
    }

    /// Return the selected carrier wire profile.
    #[must_use]
    pub const fn protocol_profile(&self) -> CarrierProtocolProfile {
        CarrierProtocolProfile::new(self.version, self.send_compressed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(feature = "transport")]
pub(crate) struct CarrierRuntimeConfig {
    pub protocol: CarrierProtocolProfile,
    pub disconnect_timeout: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_carrier_profile_uses_current_protocol_settings() {
        let desc = CarrierDesc::default();

        assert_eq!(desc.version, 1);
        assert!(!desc.send_compressed);
        assert!(desc.enable_disconnect_detection);
        assert_eq!(desc.connection_timeout_ms, 5000);
        assert_eq!(desc.protocol_profile(), CarrierProtocolProfile::default());
    }

    #[test]
    fn carrier_profile_can_be_selected_by_project_code() {
        let mut desc = CarrierDesc::new("127.0.0.1:33435");
        desc.with_protocol_profile(CarrierProtocolProfile::new(5, true));

        assert_eq!(desc.server_address, "127.0.0.1:33435");
        assert_eq!(desc.version, 5);
        assert!(desc.send_compressed);
    }
}
