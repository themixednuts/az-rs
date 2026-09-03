use std::{collections::BTreeMap, sync::Arc};

use crate::{
    BackendCapabilities, SecretBackend, SecretError, SecretFuture, SecretRef,
    reference::valid_component,
};

/// Exact first-component routing for configured secret backends.
#[derive(Default)]
pub struct SecretRouter {
    mounts: BTreeMap<Box<str>, Arc<dyn SecretBackend>>,
}

impl SecretRouter {
    /// Creates an empty fail-closed router.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mounts: BTreeMap::new(),
        }
    }

    /// Adds one exact mount.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid or duplicate mount name.
    pub fn mount(
        mut self,
        mount: impl AsRef<str>,
        backend: Arc<dyn SecretBackend>,
    ) -> Result<Self, SecretError> {
        let mount = mount.as_ref();
        if !valid_component(mount) {
            return Err(SecretError::InvalidBackendConfiguration);
        }
        if self.mounts.contains_key(mount) {
            return Err(SecretError::DuplicateMount {
                mount: mount.to_owned().into_boxed_str(),
            });
        }
        self.mounts
            .insert(mount.to_owned().into_boxed_str(), backend);
        Ok(self)
    }
}

impl SecretBackend for SecretRouter {
    fn resolve<'a>(&'a self, secret: &'a SecretRef) -> SecretFuture<'a> {
        Box::pin(async move {
            let backend =
                self.mounts
                    .get(secret.mount())
                    .ok_or_else(|| SecretError::MountNotConfigured {
                        mount: secret.mount().to_owned().into_boxed_str(),
                    })?;
            backend.resolve(secret).await
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::ROUTER
    }
}
