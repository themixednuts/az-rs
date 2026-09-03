//! Error types for the `AZoth` Editor UI crate.

use thiserror::Error;

/// Errors that can occur in the theme system.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("TOML serialization error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("Invalid theme location: {0}")]
    InvalidThemeLocation(String),

    #[error("Invalid theme setting: {0}")]
    InvalidThemeSetting(String),

    #[error("Theme '{0}' not found")]
    ThemeNotFound(String),

    #[error("Theme variant '{0}' not found")]
    ThemeVariantNotFound(String),

    #[error("Color parsing error: {0}")]
    ColorParse(String),

    #[error("Theme system not initialized")]
    NotInitialized,
}

/// Result type alias for theme operations.
pub type Result<T> = std::result::Result<T, Error>;
