//! Validated logical asset paths shared by authoring and runtime boundaries.

use std::{fmt, str::FromStr};

#[cfg(feature = "serde")]
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};

/// Owned project-relative logical asset locator.
///
/// Asset identity belongs to the asset registry. This type carries only the
/// normalized author-facing path used to resolve that identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, bevy_reflect::Reflect)]
#[reflect(opaque)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct AssetPathBuf(String);

impl AssetPathBuf {
    /// Creates a validated asset path.
    ///
    /// # Errors
    ///
    /// Returns [`AssetPathError`] when the value is empty or is not a
    /// normalized project-relative path.
    pub fn new(value: impl Into<String>) -> Result<Self, AssetPathError> {
        let value = value.into();
        validate_asset_path(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        self.file_name()?
            .rsplit_once('.')
            .map(|(_, extension)| extension)
    }

    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.0
            .rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
    }

    #[must_use]
    pub fn parent(&self) -> Option<&str> {
        self.0.rsplit_once('/').map(|(parent, _)| parent)
    }

    /// Appends one path segment.
    ///
    /// # Errors
    ///
    /// Returns [`AssetPathError`] when the resulting path is invalid.
    pub fn push(&mut self, segment: &str) -> Result<(), AssetPathError> {
        let next = if self.0.is_empty() {
            segment.to_owned()
        } else {
            format!("{}/{segment}", self.0)
        };
        validate_asset_path(&next)?;
        self.0 = next;
        Ok(())
    }

    /// Replaces the file extension.
    ///
    /// # Errors
    ///
    /// Returns [`AssetPathError`] when the extension or resulting path is
    /// invalid.
    pub fn set_extension(&mut self, extension: &str) -> Result<(), AssetPathError> {
        let extension = extension.strip_prefix('.').unwrap_or(extension);
        if extension.is_empty() || extension.contains(['/', '\\']) {
            return Err(AssetPathError::InvalidExtension);
        }
        let base_end = self
            .file_name()
            .and_then(|name| name.rfind('.').map(|dot| self.0.len() - name.len() + dot))
            .unwrap_or(self.0.len());
        let next = format!("{}.{}", &self.0[..base_end], extension);
        validate_asset_path(&next)?;
        self.0 = next;
        Ok(())
    }
}

impl fmt::Display for AssetPathBuf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for AssetPathBuf {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for AssetPathBuf {
    type Err = AssetPathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for AssetPathBuf {
    type Error = AssetPathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for AssetPathBuf {
    type Error = AssetPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for AssetPathBuf {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AssetPathError {
    #[error("asset path cannot be empty")]
    Empty,
    #[error("asset path must be project-relative")]
    Absolute,
    #[error("asset path cannot include a Windows drive prefix")]
    WindowsDrivePrefix,
    #[error("asset path must use '/' separators")]
    Backslash,
    #[error("asset path cannot contain '.' segments")]
    CurrentDirectory,
    #[error("asset path cannot contain '..' segments")]
    ParentDirectory,
    #[error("asset path cannot contain empty segments")]
    EmptySegment,
    #[error("asset path cannot end with '/'")]
    TrailingSlash,
    #[error("asset path extension is invalid")]
    InvalidExtension,
}

fn validate_asset_path(value: &str) -> Result<(), AssetPathError> {
    if value.is_empty() {
        return Err(AssetPathError::Empty);
    }
    if value.starts_with('/') {
        return Err(AssetPathError::Absolute);
    }
    if value.as_bytes().get(1) == Some(&b':') && value.as_bytes()[0].is_ascii_alphabetic() {
        return Err(AssetPathError::WindowsDrivePrefix);
    }
    if value.contains('\\') {
        return Err(AssetPathError::Backslash);
    }
    if value.ends_with('/') {
        return Err(AssetPathError::TrailingSlash);
    }
    for segment in value.split('/') {
        match segment {
            "" => return Err(AssetPathError::EmptySegment),
            "." => return Err(AssetPathError::CurrentDirectory),
            ".." => return Err(AssetPathError::ParentDirectory),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_edits_asset_paths() {
        let mut path = AssetPathBuf::new("textures/brick.png").unwrap();
        assert_eq!(path.extension(), Some("png"));
        assert_eq!(path.parent(), Some("textures"));
        path.set_extension("ktx2").unwrap();
        assert_eq!(path.as_str(), "textures/brick.ktx2");
    }

    #[test]
    fn rejects_non_relative_or_non_normalized_paths() {
        let absolute = std::env::temp_dir().join("asset-root");
        let absolute = absolute.to_string_lossy();
        for value in [
            "",
            "/root",
            absolute.as_ref(),
            "a\\b",
            "a/../b",
            "a//b",
            "a/",
        ] {
            assert!(AssetPathBuf::new(value).is_err(), "{value}");
        }
    }
}
