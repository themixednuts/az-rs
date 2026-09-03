//! Trusted-host secret routing manifest schema.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ProjectManifestError;

/// Host-tool-only `[secrets]` routing configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSecrets {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mounts: BTreeMap<String, az_secrets::SecretMountConfig>,
}

impl ProjectSecrets {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }

    pub(crate) fn validate(&self) -> Result<(), ProjectManifestError> {
        for (mount, config) in &self.mounts {
            az_secrets::SecretRef::parse(format!("secret://{mount}/validation")).map_err(|_| {
                ProjectManifestError::InvalidSecretConfig {
                    mount: mount.clone(),
                    reason: "mount must be one non-empty logical-path component".to_owned(),
                }
            })?;
            if config.backend.trim().is_empty() || config.backend != config.backend.trim() {
                return Err(ProjectManifestError::InvalidSecretConfig {
                    mount: mount.clone(),
                    reason: "backend must be a non-empty backend id without surrounding whitespace"
                        .to_owned(),
                });
            }
        }
        Ok(())
    }
}
