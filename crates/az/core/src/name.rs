//! String interning — analog of Lumberyard `AZ::Name` and `CryEngine`
//! `CCryName`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock, RwLock};

static NAME_POOL: OnceLock<RwLock<NamePool>> = OnceLock::new();

fn pool() -> &'static RwLock<NamePool> {
    NAME_POOL.get_or_init(|| RwLock::new(NamePool::new()))
}

struct NamePool {
    strings: HashMap<Arc<str>, Arc<str>>,
}

impl NamePool {
    fn new() -> Self {
        Self {
            strings: HashMap::new(),
        }
    }

    fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some(existing) = self.strings.get(s) {
            Arc::clone(existing)
        } else {
            let arc: Arc<str> = s.into();
            self.strings.insert(Arc::clone(&arc), Arc::clone(&arc));
            arc
        }
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.strings.len()
    }
}

/// Interned string with case-sensitive comparison and pointer-equality
/// fast-path. Named `AzName` to avoid collision with `bevy::prelude::Name`.
#[derive(Clone, bevy_reflect::Reflect)]
#[reflect(opaque)]
pub struct AzName {
    inner: Arc<str>,
}

impl AzName {
    /// # Panics
    ///
    /// Panics if the global name pool lock is poisoned.
    pub fn new(s: impl AsRef<str>) -> Self {
        let inner = pool()
            .write()
            .expect("Name pool lock poisoned")
            .intern(s.as_ref());
        Self { inner }
    }

    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Total number of interned strings in the global pool.
    ///
    /// # Panics
    ///
    /// Panics if the global name pool lock is poisoned.
    #[must_use]
    pub fn pool_size() -> usize {
        pool().read().expect("Name pool lock poisoned").len()
    }
}

impl std::fmt::Debug for AzName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AzName(\"{}\")", self.as_str())
    }
}

impl std::fmt::Display for AzName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PartialEq for AzName {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for AzName {}

impl Hash for AzName {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl From<&str> for AzName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AzName {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl Default for AzName {
    fn default() -> Self {
        Self::new("")
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for AzName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for AzName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        <String as serde::Deserialize>::deserialize(deserializer).map(Self::new)
    }
}

impl AsRef<str> for AzName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for AzName {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for AzName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for AzName {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

/// Interned string with case-insensitive comparison; preserves
/// original case for display.
#[derive(Clone, bevy_reflect::Reflect)]
#[reflect(opaque)]
pub struct AzNameCaseInsensitive {
    inner: Arc<str>,
    lower: Arc<str>,
}

impl AzNameCaseInsensitive {
    /// # Panics
    ///
    /// Panics if the global name pool lock is poisoned.
    pub fn new(s: impl AsRef<str>) -> Self {
        let s_ref = s.as_ref();
        let inner = pool()
            .write()
            .expect("Name pool lock poisoned")
            .intern(s_ref);
        let lower = pool()
            .write()
            .expect("Name pool lock poisoned")
            .intern(&s_ref.to_lowercase());
        Self { inner, lower }
    }

    /// Original-case string.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Lowercased copy used for comparisons.
    #[inline]
    #[must_use]
    pub fn as_lower(&self) -> &str {
        &self.lower
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

impl std::fmt::Debug for AzNameCaseInsensitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AzNameCaseInsensitive(\"{}\")", self.as_str())
    }
}

impl std::fmt::Display for AzNameCaseInsensitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PartialEq for AzNameCaseInsensitive {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.lower, &other.lower)
    }
}

impl Eq for AzNameCaseInsensitive {}

impl Hash for AzNameCaseInsensitive {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.lower).hash(state);
    }
}

impl From<&str> for AzNameCaseInsensitive {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AzNameCaseInsensitive {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl Default for AzNameCaseInsensitive {
    fn default() -> Self {
        Self::new("")
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for AzNameCaseInsensitive {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for AzNameCaseInsensitive {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        <String as serde::Deserialize>::deserialize(deserializer).map(Self::new)
    }
}

impl AsRef<str> for AzNameCaseInsensitive {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_interning_returns_same_arc() {
        let name1 = AzName::new("test");
        let name2 = AzName::new("test");
        let name3 = AzName::new("other");

        assert_eq!(name1, name2);
        assert_ne!(name1, name3);
        assert!(Arc::ptr_eq(&name1.inner, &name2.inner));
    }

    #[test]
    fn name_case_insensitive_compares_lowercased() {
        let name1 = AzNameCaseInsensitive::new("Test");
        let name2 = AzNameCaseInsensitive::new("test");
        let name3 = AzNameCaseInsensitive::new("TEST");

        assert_eq!(name1, name2);
        assert_eq!(name1, name3);

        assert_eq!(name1.as_str(), "Test");
        assert_eq!(name2.as_str(), "test");
    }

    #[test]
    fn name_equality_with_str() {
        let name = AzName::new("hello");
        assert_eq!(name, "hello");
        assert_ne!(name, "world");
    }
}
