use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::SecretError;

const SCHEME: &str = "secret://";

/// A fail-closed, backend-neutral logical secret reference.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretRef(Box<str>);

impl SecretRef {
    /// Maximum encoded reference length accepted from configuration.
    pub const MAX_BYTES: usize = 1_024;

    /// Parses a `secret://` reference with a safe relative path.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::InvalidReference`] for a missing scheme, empty or
    /// oversized path, traversal, separator ambiguity, or unsupported byte.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, SecretError> {
        let value = value.as_ref();
        let Some(path) = value.strip_prefix(SCHEME) else {
            return Err(SecretError::InvalidReference);
        };
        if value.len() > Self::MAX_BYTES || !valid_path(path) {
            return Err(SecretError::InvalidReference);
        }
        Ok(Self(value.to_owned().into_boxed_str()))
    }

    /// Returns the complete encoded reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the backend-neutral path after `secret://`.
    #[must_use]
    pub fn path(&self) -> &str {
        debug_assert!(self.0.starts_with(SCHEME));
        self.0.get(SCHEME.len()..).unwrap_or_default()
    }

    /// Returns the first path component used for exact mount routing.
    #[must_use]
    pub fn mount(&self) -> &str {
        self.path().split('/').next().unwrap_or_default()
    }

    /// Returns the logical path after the mount component.
    #[must_use]
    pub fn path_within_mount(&self) -> &str {
        self.path().split_once('/').map_or("", |(_, path)| path)
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("SecretRef").field(&self.0).finish()
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for SecretRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

pub fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains('\\')
        && path.split('/').all(valid_component)
}
