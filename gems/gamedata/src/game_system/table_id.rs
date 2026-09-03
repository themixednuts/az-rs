use std::fmt;

use az_core::crc::Crc32;

/// Stable identity of one physical table within a merged row-schema family.
///
/// Authored table names are compared with Lumberyard's lowercase CRC policy.
/// The active asset catalog owns name-to-product discovery; generated code does
/// not enumerate physical tables.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct TableId(Crc32);

impl TableId {
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(Crc32::from_str_lower(name))
    }

    #[must_use]
    pub const fn from_crc(crc: Crc32) -> Self {
        Self(crc)
    }

    #[must_use]
    pub const fn crc(self) -> Crc32 {
        self.0
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0.value()
    }
}

impl fmt::Debug for TableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "TableId({:#010x})", self.value())
    }
}

impl From<&str> for TableId {
    fn from(name: &str) -> Self {
        Self::from_name(name)
    }
}

impl From<Crc32> for TableId {
    fn from(crc: Crc32) -> Self {
        Self::from_crc(crc)
    }
}

impl From<TableId> for Crc32 {
    fn from(id: TableId) -> Self {
        id.crc()
    }
}
