use serde::{Deserialize, Serialize};

/// One authored operation applied to a base value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValueOp<T> {
    Set(T),
    Add(T),
    Subtract(T),
    Multiply(T),
    Divide(T),
}

impl<T> ValueOp<T> {
    #[inline]
    #[must_use]
    pub const fn value(&self) -> &T {
        match self {
            Self::Set(value)
            | Self::Add(value)
            | Self::Subtract(value)
            | Self::Multiply(value)
            | Self::Divide(value) => value,
        }
    }
}

/// Ordered authored value operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValueOps<T>(Vec<ValueOp<T>>);

impl<T> ValueOps<T> {
    #[inline]
    #[must_use]
    pub const fn new(ops: Vec<ValueOp<T>>) -> Self {
        Self(ops)
    }

    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[ValueOp<T>] {
        &self.0
    }

    #[inline]
    #[must_use]
    pub fn into_inner(self) -> Vec<ValueOp<T>> {
        self.0
    }
}

impl<T> Default for ValueOps<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

/// Unordered include/exclude filter terms.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FilterSet<T> {
    pub include: Vec<T>,
    pub exclude: Vec<T>,
}

impl<T> FilterSet<T> {
    #[inline]
    #[must_use]
    pub const fn new(include: Vec<T>, exclude: Vec<T>) -> Self {
        Self { include, exclude }
    }
}

impl<T> Default for FilterSet<T> {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

/// Ordered filter term for source formats where order is semantically relevant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterTerm<T> {
    Include(T),
    Exclude(T),
}

/// Authored filter with explicit wildcard states.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MatchFilter<T> {
    #[default]
    Any,
    Never,
    Terms(FilterSet<T>),
}
