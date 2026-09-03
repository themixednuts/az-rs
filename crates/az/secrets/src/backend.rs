use std::{future::Future, pin::Pin};

use crate::{ResolvedSecret, SecretError, SecretRef};

/// Static capabilities of one configured backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    /// The backend has a true provider batch operation.
    pub batch: bool,
    /// The backend preserves arbitrary bytes.
    pub binary: bool,
    /// The backend performs a remote get operation.
    pub remote_get: bool,
}

impl BackendCapabilities {
    /// Capabilities of the encrypted local store.
    pub const LOCAL_BINARY: Self = Self {
        batch: false,
        binary: true,
        remote_get: false,
    };

    /// Capabilities of explicit process-environment resolution.
    pub const ENV_TEXT: Self = Self {
        batch: false,
        binary: false,
        remote_get: false,
    };

    /// Conservative capabilities of a heterogeneous router.
    pub const ROUTER: Self = Self {
        batch: false,
        binary: false,
        remote_get: false,
    };
}

/// One asynchronous secret-resolution result.
pub type SecretFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ResolvedSecret, SecretError>> + Send + 'a>>;

/// One ordered, all-or-nothing batch result.
pub type SecretBatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ResolvedSecret>, SecretError>> + Send + 'a>>;

/// Narrow provider-neutral secret resolution boundary.
pub trait SecretBackend: Send + Sync {
    /// Resolves one logical reference without selecting a fallback.
    fn resolve<'a>(&'a self, secret: &'a SecretRef) -> SecretFuture<'a>;

    /// Resolves every reference in order or returns no partial result.
    fn resolve_many<'a>(&'a self, secrets: &'a [SecretRef]) -> SecretBatchFuture<'a> {
        Box::pin(async move {
            let mut resolved = Vec::with_capacity(secrets.len());
            for secret in secrets {
                resolved.push(self.resolve(secret).await?);
            }
            Ok(resolved)
        })
    }

    /// Describes static backend behavior without provider types.
    fn capabilities(&self) -> BackendCapabilities;
}
