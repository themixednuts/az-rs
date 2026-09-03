//! Opaque, redacted, size-bounded external credentials.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Maximum accepted credential payload. Steam session tickets and EOS Connect
/// tokens are well under this; the bound rejects malformed or hostile input
/// before it reaches a provider SDK.
pub const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

/// The named scheme a credential envelope carries.
///
/// Each provider declares the schemes it accepts; a Steam session ticket and a
/// Steam Web API ticket are *distinct* schemes and must never be substituted
/// for one another (ADR 0026).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialScheme(String);

impl CredentialScheme {
    pub const STEAM_SESSION_TICKET: &'static str = "steam-session-ticket";
    pub const STEAM_WEB_API_TICKET: &'static str = "steam-web-api-ticket";
    pub const EOS_CONNECT: &'static str = "eos-connect";
    pub const LOCAL_PASSWORD: &'static str = "local-password";

    #[must_use]
    pub fn new(scheme: impl Into<String>) -> Self {
        Self(scheme.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CredentialScheme {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The relying party a credential is intended for (game, service, or Web API
/// audience). Providers reject credentials minted for a different audience.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Audience(String);

impl Audience {
    #[must_use]
    pub fn new(audience: impl Into<String>) -> Self {
        Self(audience.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Audience {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An external credential in transit from a client to the auth host.
///
/// The payload bytes are opaque to the engine, never logged (the [`Debug`] impl
/// redacts them), size-bounded on construction, and zeroized on drop.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialEnvelope {
    pub scheme: CredentialScheme,
    pub audience: Audience,
    #[serde(with = "base64_bytes")]
    payload: Vec<u8>,
}

impl CredentialEnvelope {
    /// Builds an envelope, rejecting an empty or oversized payload.
    ///
    /// # Errors
    /// Returns [`CredentialError`] if the payload is empty or exceeds
    /// [`MAX_CREDENTIAL_BYTES`].
    pub fn new(
        scheme: CredentialScheme,
        audience: Audience,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Self, CredentialError> {
        let payload = payload.into();
        if payload.is_empty() {
            return Err(CredentialError::Empty);
        }
        if payload.len() > MAX_CREDENTIAL_BYTES {
            return Err(CredentialError::TooLarge {
                len: payload.len(),
                max: MAX_CREDENTIAL_BYTES,
            });
        }
        Ok(Self {
            scheme,
            audience,
            payload,
        })
    }

    /// The opaque payload. Callers (providers) must not log it.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.payload.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl std::fmt::Debug for CredentialEnvelope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialEnvelope")
            .field("scheme", &self.scheme)
            .field("audience", &self.audience)
            .field(
                "payload",
                &format_args!("<redacted {} bytes>", self.payload.len()),
            )
            .finish()
    }
}

impl Drop for CredentialEnvelope {
    fn drop(&mut self) {
        self.payload.zeroize();
    }
}

/// Owns provider-side cancellation/release of a credential handle.
///
/// For SDKs (Steam session tickets) that require the client to cancel the
/// ticket after use, dropping the lease runs the release exactly once.
#[must_use = "dropping the lease releases the provider credential handle"]
pub struct CredentialLease {
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl CredentialLease {
    /// A lease that runs `release` once when dropped or explicitly cancelled.
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }

    /// A lease with no provider handle to release.
    pub fn noop() -> Self {
        Self { release: None }
    }

    /// Explicitly release the handle now instead of at drop.
    pub fn cancel(mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl std::fmt::Debug for CredentialLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialLease")
            .field("active", &self.release.is_some())
            .finish()
    }
}

impl Drop for CredentialLease {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// Errors constructing a [`CredentialEnvelope`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("credential payload is empty")]
    Empty,
    #[error("credential payload is {len} bytes, exceeding the {max}-byte limit")]
    TooLarge { len: usize, max: usize },
}

mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD
            .decode(encoded.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}
