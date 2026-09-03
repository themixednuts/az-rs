use std::{
    collections::{BTreeMap, btree_map::Entry},
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
};

use crate::{
    EnvSecretBackend, LocalSecretStore, SecretBackend, SecretError, SecretMountConfig,
    SecretRouter, reference::valid_component,
};

/// Built-in encrypted local-store backend id.
pub const LOCAL_BACKEND_ID: &str = "local";
/// Built-in explicit environment backend id.
pub const ENV_BACKEND_ID: &str = "env";

/// Non-secret project identity supplied to backend factories by a trusted
/// composition root.
#[derive(Clone, Copy)]
pub struct BackendBuildContext<'a> {
    /// Project display name used to derive the local data-home namespace.
    pub project_name: &'a str,
    /// Project working-tree root used to distinguish adjacent checkouts.
    pub project_root: &'a Path,
}

/// Asynchronous backend construction result.
pub type BackendFactoryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Arc<dyn SecretBackend>, SecretError>> + Send + 'a>>;

/// Open construction seam contributed by a backend package.
pub trait BackendFactory: Send + Sync {
    fn build<'a>(
        &'a self,
        mount: &'a str,
        config: &'a SecretMountConfig,
        context: BackendBuildContext<'a>,
    ) -> BackendFactoryFuture<'a>;
}

/// Backend-id registry owned by a trusted host or tool composition root.
#[derive(Default)]
pub struct SecretBackendRegistry {
    factories: BTreeMap<Box<str>, Arc<dyn BackendFactory>>,
}

impl SecretBackendRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            factories: BTreeMap::new(),
        }
    }

    /// Registers one backend id. Duplicate ownership fails closed.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for an invalid id or a duplicate error
    /// when another factory already owns it.
    pub fn register(
        &mut self,
        id: impl Into<Box<str>>,
        factory: Arc<dyn BackendFactory>,
    ) -> Result<(), SecretError> {
        let id = id.into();
        if !valid_component(&id) {
            return Err(SecretError::InvalidBackendConfiguration);
        }
        match self.factories.entry(id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(factory);
                Ok(())
            }
            Entry::Occupied(_) => Err(SecretError::DuplicateBackend { backend: id }),
        }
    }

    /// Constructs every configured mount and returns one exact router.
    ///
    /// # Errors
    ///
    /// Returns the first missing-factory, construction, or routing error.
    pub async fn build_router(
        &self,
        mounts: &BTreeMap<String, SecretMountConfig>,
        context: BackendBuildContext<'_>,
    ) -> Result<SecretRouter, SecretError> {
        let mut router = SecretRouter::new();
        for (mount, config) in mounts {
            let factory = self.factories.get(config.backend.as_str()).ok_or_else(|| {
                SecretError::BackendNotRegistered {
                    backend: config.backend.clone().into_boxed_str(),
                }
            })?;
            let backend = factory.build(mount, config, context).await?;
            router = router.mount(mount, backend)?;
        }
        Ok(router)
    }

    /// Builds project routing, adding the zero-config `project -> local` route
    /// only when the project did not explicitly reroute that mount.
    ///
    /// # Errors
    ///
    /// Returns the first missing-factory, construction, or routing error.
    pub async fn build_project_router(
        &self,
        mounts: &BTreeMap<String, SecretMountConfig>,
        context: BackendBuildContext<'_>,
    ) -> Result<SecretRouter, SecretError> {
        let mut effective = mounts.clone();
        effective
            .entry("project".to_owned())
            .or_insert_with(|| SecretMountConfig::new(LOCAL_BACKEND_ID));
        self.build_router(&effective, context).await
    }
}

/// Registers the vendor-free backends shipped by `az-secrets`.
///
/// # Errors
///
/// Returns a duplicate error if the registry already owns a core id.
pub fn register_core_backends(registry: &mut SecretBackendRegistry) -> Result<(), SecretError> {
    registry.register(LOCAL_BACKEND_ID, Arc::new(LocalFactory))?;
    registry.register(ENV_BACKEND_ID, Arc::new(EnvFactory))?;
    Ok(())
}

struct LocalFactory;

impl BackendFactory for LocalFactory {
    fn build<'a>(
        &'a self,
        _mount: &'a str,
        config: &'a SecretMountConfig,
        context: BackendBuildContext<'a>,
    ) -> BackendFactoryFuture<'a> {
        Box::pin(async move {
            if config.backend != LOCAL_BACKEND_ID || !config.options.is_empty() {
                return Err(SecretError::InvalidBackendConfiguration);
            }
            Ok(Arc::new(LocalSecretStore::for_project(
                context.project_name,
                context.project_root,
            )?) as Arc<dyn SecretBackend>)
        })
    }
}

struct EnvFactory;

impl BackendFactory for EnvFactory {
    fn build<'a>(
        &'a self,
        mount: &'a str,
        config: &'a SecretMountConfig,
        _context: BackendBuildContext<'a>,
    ) -> BackendFactoryFuture<'a> {
        Box::pin(async move {
            if config.backend != ENV_BACKEND_ID
                || config.options.len() != 1
                || !config.options.contains_key("var_prefix")
            {
                return Err(SecretError::InvalidBackendConfiguration);
            }
            let prefix = config.options["var_prefix"]
                .as_str()
                .ok_or(SecretError::InvalidBackendConfiguration)?;
            Ok(Arc::new(EnvSecretBackend::new(mount, prefix)?) as Arc<dyn SecretBackend>)
        })
    }
}
