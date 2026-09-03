//! AWS Secrets Manager adapter for trusted Azoth composition roots.

use std::{collections::BTreeMap, fmt, future::Future, pin::Pin};

use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::Client;
use az_secrets::{
    BackendBuildContext, BackendCapabilities, BackendFactory, BackendFactoryFuture, ResolvedSecret,
    SecretBackend, SecretBackendRegistry, SecretError, SecretFuture, SecretMountConfig, SecretRef,
};

/// Stable manifest backend id contributed by this package.
pub const BACKEND_ID: &str = "aws-secrets-manager";

/// Registry factory for [`BACKEND_ID`].
#[derive(Debug, Default)]
pub struct AwsSecretsManagerFactory;

/// Contributes the AWS backend factory to a trusted host registry.
///
/// # Errors
///
/// Returns a duplicate-backend error if another linked package already owns
/// [`BACKEND_ID`].
pub fn register_backend(registry: &mut SecretBackendRegistry) -> Result<(), SecretError> {
    registry.register(BACKEND_ID, std::sync::Arc::new(AwsSecretsManagerFactory))
}

impl BackendFactory for AwsSecretsManagerFactory {
    fn build<'a>(
        &'a self,
        mount: &'a str,
        config: &'a SecretMountConfig,
        _context: BackendBuildContext<'a>,
    ) -> BackendFactoryFuture<'a> {
        Box::pin(async move {
            if config.backend != BACKEND_ID
                || config
                    .options
                    .keys()
                    .any(|key| !matches!(key.as_str(), "profile" | "region" | "name_prefix"))
            {
                return Err(SecretError::InvalidBackendConfiguration);
            }
            let option = |name: &str| {
                config
                    .options
                    .get(name)
                    .map(|value| {
                        value
                            .as_str()
                            .ok_or(SecretError::InvalidBackendConfiguration)
                    })
                    .transpose()
            };
            let profile = option("profile")?;
            let region = option("region")?;
            let name_prefix = option("name_prefix")?.unwrap_or_default();
            if [profile, region]
                .into_iter()
                .flatten()
                .any(|hint| hint.trim().is_empty() || hint != hint.trim())
            {
                return Err(SecretError::InvalidBackendConfiguration);
            }
            let backend =
                AwsSecretsManagerBackend::from_ambient(mount, name_prefix, profile, region).await?;
            Ok(std::sync::Arc::new(backend) as std::sync::Arc<dyn SecretBackend>)
        })
    }
}

/// Remote-get backend using AWS's ambient credential chain.
pub struct AwsSecretsManagerBackend {
    mount: Box<str>,
    name_prefix: Box<str>,
    client: Client,
}

impl AwsSecretsManagerBackend {
    /// Constructs an adapter around an already configured AWS client.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for an invalid mount.
    pub fn new(
        mount: impl AsRef<str>,
        name_prefix: impl Into<Box<str>>,
        client: Client,
    ) -> Result<Self, SecretError> {
        let mount = mount.as_ref();
        SecretRef::parse(format!("secret://{mount}/validation"))?;
        Ok(Self {
            mount: mount.to_owned().into_boxed_str(),
            name_prefix: name_prefix.into(),
            client,
        })
    }

    /// Loads AWS's standard ambient credential chain and optional non-secret
    /// profile/region hints.
    ///
    /// # Errors
    ///
    /// Returns a configuration error for an invalid mount. Credential or
    /// network failures remain deferred to resolution, where the SDK supplies
    /// their typed request result.
    pub async fn from_ambient(
        mount: impl AsRef<str>,
        name_prefix: impl Into<Box<str>>,
        profile: Option<&str>,
        region: Option<&str>,
    ) -> Result<Self, SecretError> {
        let mut loader = aws_config::defaults(BehaviorVersion::latest());
        if let Some(profile) = profile {
            loader = loader.profile_name(profile);
        }
        if let Some(region) = region {
            loader = loader.region(aws_config::Region::new(region.to_owned()));
        }
        let config = loader.load().await;
        Self::new(mount, name_prefix, Client::new(&config))
    }

    fn name(&self, secret: &SecretRef) -> Result<String, SecretError> {
        if secret.mount() != &*self.mount {
            return Err(SecretError::BackendMountMismatch {
                expected: self.mount.clone(),
                actual: secret.mount().to_owned().into_boxed_str(),
            });
        }
        if secret.path_within_mount().is_empty() {
            return Err(SecretError::InvalidReference);
        }
        Ok(format!(
            "{}{}",
            self.name_prefix,
            secret.path_within_mount()
        ))
    }

    fn material(
        binary: Option<&aws_sdk_secretsmanager::primitives::Blob>,
        text: Option<&str>,
    ) -> Result<ResolvedSecret, SecretError> {
        let bytes = if let Some(binary) = binary {
            binary.as_ref().to_vec()
        } else if let Some(text) = text {
            text.as_bytes().to_vec()
        } else {
            return Err(SecretError::EmptyMaterial);
        };
        if bytes.is_empty() {
            return Err(SecretError::EmptyMaterial);
        }
        Ok(ResolvedSecret::from_bytes(bytes))
    }

    fn ordered_materials(
        entries: &[aws_sdk_secretsmanager::types::SecretValueEntry],
        names: &[String],
        secrets: &[SecretRef],
    ) -> Result<Vec<ResolvedSecret>, SecretError> {
        let mut by_name = BTreeMap::new();
        for entry in entries {
            let name = entry.name().ok_or(SecretError::BackendUnavailable)?;
            if !names.iter().any(|expected| expected == name)
                || by_name.insert(name, entry).is_some()
            {
                return Err(SecretError::BackendUnavailable);
            }
        }
        names
            .iter()
            .zip(secrets)
            .map(|(name, secret)| {
                let entry = by_name
                    .get(name.as_str())
                    .ok_or_else(|| SecretError::Missing {
                        reference: secret.clone(),
                    })?;
                Self::material(entry.secret_binary(), entry.secret_string())
            })
            .collect()
    }
}

impl fmt::Debug for AwsSecretsManagerBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AwsSecretsManagerBackend")
            .field("mount", &self.mount)
            .field("name_prefix", &self.name_prefix)
            .finish_non_exhaustive()
    }
}

impl SecretBackend for AwsSecretsManagerBackend {
    fn resolve<'a>(&'a self, secret: &'a SecretRef) -> SecretFuture<'a> {
        Box::pin(async move {
            let name = self.name(secret)?;
            let output = self
                .client
                .get_secret_value()
                .secret_id(name)
                .version_stage("AWSCURRENT")
                .send()
                .await
                .map_err(|error| {
                    let transport_error = if matches!(
                        error,
                        aws_sdk_secretsmanager::error::SdkError::ConstructionFailure(_)
                    ) {
                        SecretError::Authentication
                    } else {
                        SecretError::Network
                    };
                    error.as_service_error().map_or(transport_error, |service| {
                        if service.is_resource_not_found_exception() {
                            SecretError::Missing {
                                reference: secret.clone(),
                            }
                        } else if service.meta().code() == Some("AccessDeniedException") {
                            SecretError::PermissionDenied
                        } else {
                            SecretError::BackendUnavailable
                        }
                    })
                })?;
            Self::material(output.secret_binary(), output.secret_string())
        })
    }

    fn resolve_many<'a>(
        &'a self,
        secrets: &'a [SecretRef],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ResolvedSecret>, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            if secrets.is_empty() {
                return Ok(Vec::new());
            }
            let names = secrets
                .iter()
                .map(|secret| self.name(secret))
                .collect::<Result<Vec<_>, _>>()?;
            let output = self
                .client
                .batch_get_secret_value()
                .set_secret_id_list(Some(names.clone()))
                .send()
                .await
                .map_err(|error| {
                    if matches!(
                        error,
                        aws_sdk_secretsmanager::error::SdkError::ConstructionFailure(_)
                    ) {
                        SecretError::Authentication
                    } else if error.as_service_error().is_some() {
                        SecretError::BackendUnavailable
                    } else {
                        SecretError::Network
                    }
                })?;
            if let Some(error) = output.errors().first() {
                return Err(match error.error_code() {
                    Some("AccessDeniedException") => SecretError::PermissionDenied,
                    Some("ResourceNotFoundException") => {
                        let index = error
                            .secret_id()
                            .and_then(|failed| names.iter().position(|name| name == failed))
                            .unwrap_or(0);
                        SecretError::Missing {
                            reference: secrets[index].clone(),
                        }
                    }
                    _ => SecretError::BackendUnavailable,
                });
            }
            Self::ordered_materials(output.secret_values(), &names, secrets)
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            batch: true,
            binary: true,
            remote_get: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn backend() -> AwsSecretsManagerBackend {
        let config = aws_sdk_secretsmanager::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .build();
        AwsSecretsManagerBackend::new("shared", "sample/", Client::from_conf(config)).unwrap()
    }

    #[test]
    fn logical_paths_map_to_prefixed_vendor_names() {
        let backend = backend();
        assert_eq!(
            backend
                .name(&SecretRef::parse("secret://shared/api-key").unwrap())
                .unwrap(),
            "sample/api-key"
        );
        assert!(matches!(
            backend.name(&SecretRef::parse("secret://project/api-key").unwrap()),
            Err(SecretError::BackendMountMismatch { .. })
        ));
    }

    #[test]
    fn material_preserves_binary_and_fails_closed_on_empty_values() {
        let binary = aws_sdk_secretsmanager::primitives::Blob::new(b"binary\0value".to_vec());
        assert_eq!(
            AwsSecretsManagerBackend::material(Some(&binary), None)
                .unwrap()
                .as_bytes(),
            b"binary\0value"
        );
        assert!(matches!(
            AwsSecretsManagerBackend::material(None, Some("")),
            Err(SecretError::EmptyMaterial)
        ));
        assert!(matches!(
            AwsSecretsManagerBackend::material(None, None),
            Err(SecretError::EmptyMaterial)
        ));
    }

    #[test]
    fn batch_results_restore_request_order_and_require_every_value() {
        let refs = [
            SecretRef::parse("secret://shared/one").unwrap(),
            SecretRef::parse("secret://shared/two").unwrap(),
        ];
        let names = ["sample/one".to_owned(), "sample/two".to_owned()];
        let entries = [
            aws_sdk_secretsmanager::types::SecretValueEntry::builder()
                .name("sample/two")
                .secret_string("two")
                .build(),
            aws_sdk_secretsmanager::types::SecretValueEntry::builder()
                .name("sample/one")
                .secret_string("one")
                .build(),
        ];
        let values = AwsSecretsManagerBackend::ordered_materials(&entries, &names, &refs).unwrap();
        assert_eq!(values[0].as_bytes(), b"one");
        assert_eq!(values[1].as_bytes(), b"two");

        assert!(matches!(
            AwsSecretsManagerBackend::ordered_materials(&entries[..1], &names, &refs),
            Err(SecretError::Missing { .. })
        ));

        let rogue = [aws_sdk_secretsmanager::types::SecretValueEntry::builder()
            .name("sample/unrequested")
            .secret_string("rogue")
            .build()];
        assert!(matches!(
            AwsSecretsManagerBackend::ordered_materials(&rogue, &names, &refs),
            Err(SecretError::BackendUnavailable)
        ));
    }

    #[test]
    fn factory_rejects_credentials_and_malformed_identity_hints() {
        for options in [
            BTreeMap::from([(
                "access_key".to_owned(),
                serde_json::Value::String("inline".to_owned()),
            )]),
            BTreeMap::from([(
                "profile".to_owned(),
                serde_json::Value::String("  ".to_owned()),
            )]),
        ] {
            let config = SecretMountConfig {
                backend: BACKEND_ID.to_owned(),
                options,
            };
            let result = futures::executor::block_on(AwsSecretsManagerFactory.build(
                "shared",
                &config,
                BackendBuildContext {
                    project_name: "sample",
                    project_root: std::path::Path::new("sample"),
                },
            ));
            assert!(matches!(
                result,
                Err(SecretError::InvalidBackendConfiguration)
            ));
        }
    }
}
