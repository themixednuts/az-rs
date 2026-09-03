//! Redacted value types shared across Azoth trust boundaries.
//!
//! [`Secret`] preserves its inner representation for explicit serialization
//! adapters, but it does not implement `Deref`; observing material always uses
//! [`Secret::expose`].
//!
//! ```compile_fail
//! use az_secret_value::Secret;
//!
//! let secret = Secret::new(String::from("sensitive"));
//! let implicitly_exposed: &str = &secret;
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

/// A representation-preserving value with redacted diagnostics.
///
/// This wrapper does not itself promise zeroization. Use an inner type that
/// owns the required destruction semantics when material must be wiped.
#[repr(transparent)]
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wraps a value without changing its representation.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Explicitly borrows the sensitive value.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    /// Transfers the inner value to a new owner.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([REDACTED])")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}
