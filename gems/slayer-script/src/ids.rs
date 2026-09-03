//! Dense identifiers into immutable `SlayerScript` program tables.

/// Identifies a module in a project-defined module adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ModuleId(u32);

impl ModuleId {
    /// Creates a module identifier from its stable compiled value.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the stable compiled value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Native `SlayerScript` state identifier.
///
/// Native state storage is a signed 32-bit integer. `-1` is the canonical
/// absence sentinel; conversion from narrower replicated fields belongs to
/// the project protocol adapter rather than this generic runtime type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct StateId(i32);

impl StateId {
    /// Native sentinel meaning that no state is selected.
    pub const NONE: Self = Self(-1);

    /// Creates a state identifier with its exact native signed value.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Returns the exact native signed value.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }

    /// Returns whether this is the native absence sentinel.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == Self::NONE.0
    }
}

impl Default for StateId {
    fn default() -> Self {
        Self::NONE
    }
}

/// Identifies one layer in a [`crate::SlayerProgram`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct LayerId(u32);

impl LayerId {
    /// Creates a layer identifier from a dense table index.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the dense table index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        usize::try_from(self.0).expect("u32 layer identifiers must fit the target index space")
    }
}

/// Identifies one sequence in a [`crate::SlayerProgram`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SequenceId(u32);

impl SequenceId {
    /// Creates a sequence identifier from a dense table index.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the dense table index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        usize::try_from(self.0).expect("u32 sequence identifiers must fit the target index space")
    }
}

/// Monotonic identifier for one live sequence-transition record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SequenceRuntimeId(u32);

impl SequenceRuntimeId {
    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the instance-local monotonic value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Monotonic identifier for one live event-track record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EventRuntimeId(u32);

impl EventRuntimeId {
    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the instance-local monotonic value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Instance-local callback key formed by wrapping base-plus-authored-ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct CallbackRuntimeId(u32);

impl CallbackRuntimeId {
    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Opaque callback-local identifier assigned by the script compiler.
///
/// Native callback registration adds this word to the active layer's wrapping
/// nesting base to form [`CallbackRuntimeId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct CallbackAuthoredId(u32);

impl CallbackAuthoredId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::StateId;

    #[test]
    fn state_id_preserves_signed_native_values() {
        assert_eq!(StateId::new(i32::MIN).get(), i32::MIN);
        assert_eq!(StateId::new(i32::MAX).get(), i32::MAX);
        assert_eq!(std::mem::size_of::<StateId>(), std::mem::size_of::<i32>());
    }

    #[test]
    fn state_id_default_is_the_native_none_sentinel() {
        assert_eq!(StateId::default(), StateId::NONE);
        assert!(StateId::NONE.is_none());
        assert!(!StateId::new(0).is_none());
    }
}
