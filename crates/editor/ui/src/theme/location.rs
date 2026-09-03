//! Theme location identifiers.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Represents different types of theme locations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThemeLocation {
    /// Base themes shipped with the application (lowest precedence)
    Base(PathBuf),
    /// User themes in an editor-shell resolved config directory
    User(PathBuf),
    /// Custom user-specified directory (highest precedence)
    Custom(PathBuf),
}

impl ThemeLocation {
    /// Get the actual filesystem path for this location.
    ///
    /// # Errors
    ///
    /// Currently always returns `Ok`; the `Result` return type is preserved so
    /// future location kinds may report resolution failures.
    pub fn path(&self) -> Result<PathBuf> {
        match self {
            Self::Base(path) | Self::User(path) | Self::Custom(path) => Ok(path.clone()),
        }
    }

    /// Check if this location exists and is accessible.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.path().is_ok_and(|p| p.exists())
    }

    /// Create a base location from a path.
    #[must_use]
    pub const fn base(path: PathBuf) -> Self {
        Self::Base(path)
    }
}

impl From<PathBuf> for ThemeLocation {
    fn from(path: PathBuf) -> Self {
        Self::Custom(path)
    }
}

impl From<&Path> for ThemeLocation {
    fn from(path: &Path) -> Self {
        Self::Custom(path.to_path_buf())
    }
}

impl TryFrom<&str> for ThemeLocation {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        if value.to_lowercase().as_str() == "user" {
            Err(Error::InvalidThemeLocation(value.to_string()))
        } else {
            // Try to parse as a path
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Ok(Self::Custom(path))
            } else {
                Err(Error::InvalidThemeLocation(value.to_string()))
            }
        }
    }
}

impl From<ThemeLocation> for PathBuf {
    fn from(location: ThemeLocation) -> Self {
        location.path().unwrap_or_else(|_| Self::new())
    }
}

impl std::fmt::Display for ThemeLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Base(path) => write!(f, "base:{}", path.display()),
            Self::User(path) => write!(f, "user:{}", path.display()),
            Self::Custom(path) => write!(f, "{}", path.display()),
        }
    }
}
