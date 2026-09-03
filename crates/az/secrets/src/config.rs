use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Serializable configuration for one exact logical secret mount.
///
/// Options are deliberately provider-neutral and must contain no credentials;
/// the selected backend factory validates the keys it understands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMountConfig {
    /// Stable id owned by one registered backend factory.
    pub backend: String,
    /// Non-secret, backend-specific construction hints.
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_json::Value>,
}

impl SecretMountConfig {
    /// Constructs a mount with no backend-specific options.
    #[must_use]
    pub fn new(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            options: BTreeMap::new(),
        }
    }
}
