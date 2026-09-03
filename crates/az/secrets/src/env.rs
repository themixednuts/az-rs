use std::{fmt, sync::Arc};

use crate::{
    BackendCapabilities, ResolvedSecret, SecretBackend, SecretError, SecretFuture, SecretRef,
    reference::valid_component,
};

trait EnvironmentSource: Send + Sync {
    fn read(&self, name: &str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvironmentSource for ProcessEnvironment {
    fn read(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Explicit process-environment backend for one configured mount.
pub struct EnvSecretBackend {
    mount: Box<str>,
    variable_prefix: Box<str>,
    environment: Arc<dyn EnvironmentSource>,
}

impl EnvSecretBackend {
    /// Binds one logical mount to process variables with `variable_prefix`.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::InvalidBackendConfiguration`] unless the mount is
    /// one safe component and the prefix contains only `A-Z`, `0-9`, or `_`.
    pub fn new(
        mount: impl AsRef<str>,
        variable_prefix: impl AsRef<str>,
    ) -> Result<Self, SecretError> {
        Self::with_source(mount, variable_prefix, Arc::new(ProcessEnvironment))
    }

    fn with_source(
        mount: impl AsRef<str>,
        variable_prefix: impl AsRef<str>,
        environment: Arc<dyn EnvironmentSource>,
    ) -> Result<Self, SecretError> {
        let mount = mount.as_ref();
        let variable_prefix = variable_prefix.as_ref();
        let valid_prefix = !variable_prefix.is_empty()
            && variable_prefix
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
        if !valid_component(mount) || !valid_prefix {
            return Err(SecretError::InvalidBackendConfiguration);
        }
        Ok(Self {
            mount: mount.to_owned().into_boxed_str(),
            variable_prefix: variable_prefix.to_owned().into_boxed_str(),
            environment,
        })
    }

    fn variable_name(&self, secret: &SecretRef) -> Result<String, SecretError> {
        if secret.mount() != &*self.mount {
            return Err(SecretError::BackendMountMismatch {
                expected: self.mount.clone(),
                actual: secret.mount().to_owned().into_boxed_str(),
            });
        }
        let path = secret.path_within_mount();
        if path.is_empty() {
            return Err(SecretError::InvalidReference);
        }
        let mut name = String::with_capacity(self.variable_prefix.len() + path.len());
        name.push_str(&self.variable_prefix);
        for byte in path.bytes() {
            if byte.is_ascii_alphanumeric() {
                name.push(char::from(byte.to_ascii_uppercase()));
            } else {
                name.push('_');
            }
        }
        Ok(name)
    }
}

impl fmt::Debug for EnvSecretBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvSecretBackend")
            .field("mount", &self.mount)
            .field("variable_prefix", &self.variable_prefix)
            .finish_non_exhaustive()
    }
}

impl SecretBackend for EnvSecretBackend {
    fn resolve<'a>(&'a self, secret: &'a SecretRef) -> SecretFuture<'a> {
        Box::pin(async move {
            let name = self.variable_name(secret)?;
            let value = self
                .environment
                .read(&name)
                .ok_or_else(|| SecretError::Missing {
                    reference: secret.clone(),
                })?;
            if value.is_empty() {
                return Err(SecretError::EmptyMaterial);
            }
            Ok(ResolvedSecret::from_bytes(value.into_bytes()))
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::ENV_TEXT
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use super::*;
    use crate::{SecretError, SecretRef};

    struct MapEnvironment(BTreeMap<String, String>);

    impl EnvironmentSource for MapEnvironment {
        fn read(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    #[test]
    fn explicit_env_mount_maps_only_its_logical_path() {
        let backend = EnvSecretBackend::with_source(
            "ci",
            "AZ_SECRET_",
            Arc::new(MapEnvironment(BTreeMap::from([(
                "AZ_SECRET_DB_URL".to_owned(),
                "postgres://example".to_owned(),
            )]))),
        )
        .expect("valid env mount");

        let value = futures::executor::block_on(
            backend.resolve(&SecretRef::parse("secret://ci/db-url").expect("valid reference")),
        )
        .expect("resolve explicit env value");
        assert_eq!(
            value.as_utf8().expect("text environment value"),
            "postgres://example"
        );

        let wrong_mount = futures::executor::block_on(
            backend.resolve(&SecretRef::parse("secret://project/db-url").expect("valid reference")),
        );
        assert!(matches!(
            wrong_mount,
            Err(SecretError::BackendMountMismatch { .. })
        ));
    }

    #[test]
    fn missing_empty_and_mount_only_values_fail_closed() {
        let backend = EnvSecretBackend::with_source(
            "ci",
            "AZ_SECRET_",
            Arc::new(MapEnvironment(BTreeMap::from([(
                "AZ_SECRET_EMPTY".to_owned(),
                String::new(),
            )]))),
        )
        .expect("valid env mount");

        let missing = futures::executor::block_on(
            backend.resolve(&SecretRef::parse("secret://ci/missing").expect("valid reference")),
        );
        assert!(matches!(missing, Err(SecretError::Missing { .. })));

        let empty = futures::executor::block_on(
            backend.resolve(&SecretRef::parse("secret://ci/empty").expect("valid reference")),
        );
        assert!(matches!(empty, Err(SecretError::EmptyMaterial)));

        let mount_only = futures::executor::block_on(
            backend.resolve(&SecretRef::parse("secret://ci").expect("valid reference")),
        );
        assert!(matches!(mount_only, Err(SecretError::InvalidReference)));
    }
}
