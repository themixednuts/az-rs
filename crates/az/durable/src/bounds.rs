use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

/// Hard cap for one adapter page.
pub const MAX_PAGE_ITEMS: usize = 4_096;

/// Nonzero caller-selected page bound capped by the durable contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedPageSize(NonZeroUsize);

impl BoundedPageSize {
    /// Constructs a page bound in `1..=4096`.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError`] for zero or a value above [`MAX_PAGE_ITEMS`].
    pub const fn new(value: usize) -> Result<Self, BoundError> {
        let Some(value) = NonZeroUsize::new(value) else {
            return Err(BoundError::Zero);
        };
        if value.get() > MAX_PAGE_ITEMS {
            return Err(BoundError::ExceedsLimit {
                actual: value.get(),
                maximum: MAX_PAGE_ITEMS,
            });
        }
        Ok(Self(value))
    }

    /// Returns the numeric bound.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// Owned page whose item count was checked before construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPage<T>(Box<[T]>);

impl<T> BoundedPage<T> {
    /// Takes ownership of a page no larger than `limit`.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError::ExceedsLimit`] when the adapter returned too many items.
    pub fn try_from_boxed(items: Box<[T]>, limit: BoundedPageSize) -> Result<Self, BoundError> {
        if items.len() > limit.get() {
            return Err(BoundError::ExceedsLimit {
                actual: items.len(),
                maximum: limit.get(),
            });
        }
        Ok(Self(items))
    }

    /// Borrows the page contents.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    /// Transfers ownership to the caller.
    #[must_use]
    pub fn into_boxed(self) -> Box<[T]> {
        self.0
    }
}

/// Invalid bounded collection size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BoundError {
    /// A page bound must allow at least one item.
    #[error("bound must be nonzero")]
    Zero,
    /// A collection exceeded its declared or hard limit.
    #[error("bounded collection has {actual} items; maximum is {maximum}")]
    ExceedsLimit {
        /// Observed count.
        actual: usize,
        /// Accepted count.
        maximum: usize,
    },
}
