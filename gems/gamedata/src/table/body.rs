//! Decoded in-memory representation of `GameData` table assets.

use std::borrow::Cow;
use std::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64,
};

use bevy_color::LinearRgba;

use crate::GameDataError;
use crate::identity::{RowGuid, RowIndex};
use crate::release::SchemaHash;

/// Dependency kind stored in the per-table `DependencyIndex` section.
pub const DEPENDENCY_KIND_FOREIGN_KEY: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarType {
    String,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Crc32,
    RowIndex,
    RowKey,
    ForeignKey,
    NonZeroI8,
    NonZeroI16,
    NonZeroI32,
    NonZeroI64,
    NonZeroU8,
    NonZeroU16,
    NonZeroU32,
    NonZeroU64,
    LinearRgba,
}

impl ScalarType {
    #[inline]
    #[must_use]
    pub const fn id(self) -> u8 {
        match self {
            Self::String => 0x01,
            Self::Bool => 0x02,
            Self::I8 => 0x03,
            Self::I16 => 0x04,
            Self::I32 => 0x05,
            Self::I64 => 0x06,
            Self::U8 => 0x07,
            Self::U16 => 0x08,
            Self::U32 => 0x09,
            Self::U64 => 0x0a,
            Self::F32 => 0x0b,
            Self::F64 => 0x0c,
            Self::Crc32 => 0x0d,
            Self::RowIndex => 0x0e,
            Self::RowKey => 0x0f,
            Self::ForeignKey => 0x10,
            Self::NonZeroI8 => 0x11,
            Self::NonZeroI16 => 0x12,
            Self::NonZeroI32 => 0x13,
            Self::NonZeroI64 => 0x14,
            Self::NonZeroU8 => 0x15,
            Self::NonZeroU16 => 0x16,
            Self::NonZeroU32 => 0x17,
            Self::NonZeroU64 => 0x18,
            Self::LinearRgba => 0x19,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0x01 => Some(Self::String),
            0x02 => Some(Self::Bool),
            0x03 => Some(Self::I8),
            0x04 => Some(Self::I16),
            0x05 => Some(Self::I32),
            0x06 => Some(Self::I64),
            0x07 => Some(Self::U8),
            0x08 => Some(Self::U16),
            0x09 => Some(Self::U32),
            0x0a => Some(Self::U64),
            0x0b => Some(Self::F32),
            0x0c => Some(Self::F64),
            0x0d => Some(Self::Crc32),
            0x0e => Some(Self::RowIndex),
            0x0f => Some(Self::RowKey),
            0x10 => Some(Self::ForeignKey),
            0x11 => Some(Self::NonZeroI8),
            0x12 => Some(Self::NonZeroI16),
            0x13 => Some(Self::NonZeroI32),
            0x14 => Some(Self::NonZeroI64),
            0x15 => Some(Self::NonZeroU8),
            0x16 => Some(Self::NonZeroU16),
            0x17 => Some(Self::NonZeroU32),
            0x18 => Some(Self::NonZeroU64),
            0x19 => Some(Self::LinearRgba),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeEndpointType {
    F32,
    I32,
    U32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeBounds {
    Exclusive,
    Inclusive,
}

impl RangeBounds {
    #[inline]
    #[must_use]
    pub const fn id(self) -> u8 {
        match self {
            Self::Exclusive => 0x01,
            Self::Inclusive => 0x02,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0x01 => Some(Self::Exclusive),
            0x02 => Some(Self::Inclusive),
            _ => None,
        }
    }
}

impl RangeEndpointType {
    #[inline]
    #[must_use]
    pub const fn id(self) -> u8 {
        match self {
            Self::F32 => 0x01,
            Self::I32 => 0x02,
            Self::U32 => 0x03,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0x01 => Some(Self::F32),
            0x02 => Some(Self::I32),
            0x03 => Some(Self::U32),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RangeType {
    pub bounds: RangeBounds,
    pub endpoint: RangeEndpointType,
}

impl RangeType {
    #[inline]
    #[must_use]
    pub const fn new(bounds: RangeBounds, endpoint: RangeEndpointType) -> Self {
        Self { bounds, endpoint }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomType {
    Scalar(ScalarType),
    Range(RangeType),
}

impl AtomType {
    #[inline]
    #[must_use]
    pub const fn cell_type(self) -> CellType {
        match self {
            Self::Scalar(scalar) => CellType::Scalar(scalar),
            Self::Range(range) => CellType::Range(range),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PairType {
    pub first: AtomType,
    pub second: AtomType,
}

impl PairType {
    #[inline]
    #[must_use]
    pub const fn new(first: AtomType, second: AtomType) -> Self {
        Self { first, second }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListElementType {
    Scalar(ScalarType),
    Range(RangeType),
    Pair(PairType),
}

impl ListElementType {
    #[inline]
    #[must_use]
    pub const fn atom_type(self) -> Option<AtomType> {
        match self {
            Self::Scalar(scalar) => Some(AtomType::Scalar(scalar)),
            Self::Range(range) => Some(AtomType::Range(range)),
            Self::Pair(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellType {
    Scalar(ScalarType),
    Range(RangeType),
    List(ListElementType),
}

impl CellType {
    #[inline]
    #[must_use]
    pub const fn list_element_type(self) -> Option<ListElementType> {
        match self {
            Self::List(element) => Some(element),
            Self::Scalar(_) | Self::Range(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnDescriptor {
    pub crc: u32,
    pub cell_type: CellType,
    pub flags: u32,
}

/// Validated outbound edge from one hot column to another table in the release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableDependency {
    pub column_crc: u32,
    pub target_table_name_crc: u32,
    pub target_schema_hash: SchemaHash,
    pub kind: u32,
}

/// Import/encode cell value used before bytes are interned into a table asset.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue<'a> {
    String(Cow<'a, str>),
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    NonZeroI8(NonZeroI8),
    NonZeroI16(NonZeroI16),
    NonZeroI32(NonZeroI32),
    NonZeroI64(NonZeroI64),
    NonZeroU8(NonZeroU8),
    NonZeroU16(NonZeroU16),
    NonZeroU32(NonZeroU32),
    NonZeroU64(NonZeroU64),
    F32(f32),
    F64(f64),
    LinearRgba(LinearRgba),
    RangeF32(::core::range::Range<f32>),
    RangeInclusiveF32(::core::range::RangeInclusive<f32>),
    RangeU32(::core::range::Range<u32>),
    RangeInclusiveU32(::core::range::RangeInclusive<u32>),
    RangeI32(::core::range::Range<i32>),
    RangeInclusiveI32(::core::range::RangeInclusive<i32>),
    Crc32(u32),
    RowIndex(RowIndex),
    RowKey(Cow<'a, str>),
    ForeignKey(ForeignKeyValue),
    List(ListValue<'a>),
}

impl<'a> CellValue<'a> {
    #[inline]
    #[must_use]
    pub fn string(value: impl Into<Cow<'a, str>>) -> Self {
        Self::String(value.into())
    }

    #[inline]
    #[must_use]
    pub fn row_key(value: impl Into<Cow<'a, str>>) -> Self {
        Self::RowKey(value.into())
    }

    #[inline]
    #[must_use]
    pub const fn foreign_key(value: ForeignKeyValue) -> Self {
        Self::ForeignKey(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListValue<'a> {
    Strings(Vec<Cow<'a, str>>),
    Bools(Vec<bool>),
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    NonZeroI8(Vec<NonZeroI8>),
    NonZeroI16(Vec<NonZeroI16>),
    NonZeroI32(Vec<NonZeroI32>),
    NonZeroI64(Vec<NonZeroI64>),
    NonZeroU8(Vec<NonZeroU8>),
    NonZeroU16(Vec<NonZeroU16>),
    NonZeroU32(Vec<NonZeroU32>),
    NonZeroU64(Vec<NonZeroU64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    LinearRgba(Vec<LinearRgba>),
    RangeF32(Vec<::core::range::Range<f32>>),
    RangeInclusiveF32(Vec<::core::range::RangeInclusive<f32>>),
    RangeU32(Vec<::core::range::Range<u32>>),
    RangeInclusiveU32(Vec<::core::range::RangeInclusive<u32>>),
    RangeI32(Vec<::core::range::Range<i32>>),
    RangeInclusiveI32(Vec<::core::range::RangeInclusive<i32>>),
    Crc32(Vec<u32>),
    RowIndexes(Vec<RowIndex>),
    RowKeys(Vec<Cow<'a, str>>),
    ForeignKeys(Vec<ForeignKeyValue>),
    Pairs {
        pair_type: PairType,
        values: Vec<PairValue<'a>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairValue<'a> {
    first: Box<CellValue<'a>>,
    second: Box<CellValue<'a>>,
}

impl<'a> PairValue<'a> {
    #[inline]
    #[must_use]
    pub fn new(first: CellValue<'a>, second: CellValue<'a>) -> Self {
        Self {
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    #[inline]
    #[must_use]
    pub fn first(&self) -> &CellValue<'a> {
        &self.first
    }

    #[inline]
    #[must_use]
    pub fn second(&self) -> &CellValue<'a> {
        &self.second
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ForeignKeyValue(RowIndex);

impl ForeignKeyValue {
    #[inline]
    #[must_use]
    pub const fn row(index: RowIndex) -> Self {
        Self(index)
    }

    #[inline]
    #[must_use]
    pub const fn row_index(self) -> RowIndex {
        self.0
    }
}

impl<'a> ListValue<'a> {
    #[inline]
    #[must_use]
    pub fn strings<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'a, str>>,
    {
        Self::Strings(values.into_iter().map(Into::into).collect())
    }

    #[inline]
    #[must_use]
    pub fn row_keys<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'a, str>>,
    {
        Self::RowKeys(values.into_iter().map(Into::into).collect())
    }

    #[inline]
    #[must_use]
    pub fn foreign_keys<I>(values: I) -> Self
    where
        I: IntoIterator<Item = ForeignKeyValue>,
    {
        Self::ForeignKeys(values.into_iter().collect())
    }

    #[inline]
    #[must_use]
    pub const fn element_type(&self) -> ListElementType {
        match self {
            Self::Strings(_) => ListElementType::Scalar(ScalarType::String),
            Self::Bools(_) => ListElementType::Scalar(ScalarType::Bool),
            Self::I8(_) => ListElementType::Scalar(ScalarType::I8),
            Self::I16(_) => ListElementType::Scalar(ScalarType::I16),
            Self::I32(_) => ListElementType::Scalar(ScalarType::I32),
            Self::I64(_) => ListElementType::Scalar(ScalarType::I64),
            Self::U8(_) => ListElementType::Scalar(ScalarType::U8),
            Self::U16(_) => ListElementType::Scalar(ScalarType::U16),
            Self::U32(_) => ListElementType::Scalar(ScalarType::U32),
            Self::U64(_) => ListElementType::Scalar(ScalarType::U64),
            Self::NonZeroI8(_) => ListElementType::Scalar(ScalarType::NonZeroI8),
            Self::NonZeroI16(_) => ListElementType::Scalar(ScalarType::NonZeroI16),
            Self::NonZeroI32(_) => ListElementType::Scalar(ScalarType::NonZeroI32),
            Self::NonZeroI64(_) => ListElementType::Scalar(ScalarType::NonZeroI64),
            Self::NonZeroU8(_) => ListElementType::Scalar(ScalarType::NonZeroU8),
            Self::NonZeroU16(_) => ListElementType::Scalar(ScalarType::NonZeroU16),
            Self::NonZeroU32(_) => ListElementType::Scalar(ScalarType::NonZeroU32),
            Self::NonZeroU64(_) => ListElementType::Scalar(ScalarType::NonZeroU64),
            Self::F32(_) => ListElementType::Scalar(ScalarType::F32),
            Self::F64(_) => ListElementType::Scalar(ScalarType::F64),
            Self::LinearRgba(_) => ListElementType::Scalar(ScalarType::LinearRgba),
            Self::RangeF32(_) => ListElementType::Range(RangeType::new(
                RangeBounds::Exclusive,
                RangeEndpointType::F32,
            )),
            Self::RangeInclusiveF32(_) => ListElementType::Range(RangeType::new(
                RangeBounds::Inclusive,
                RangeEndpointType::F32,
            )),
            Self::RangeU32(_) => ListElementType::Range(RangeType::new(
                RangeBounds::Exclusive,
                RangeEndpointType::U32,
            )),
            Self::RangeInclusiveU32(_) => ListElementType::Range(RangeType::new(
                RangeBounds::Inclusive,
                RangeEndpointType::U32,
            )),
            Self::RangeI32(_) => ListElementType::Range(RangeType::new(
                RangeBounds::Exclusive,
                RangeEndpointType::I32,
            )),
            Self::RangeInclusiveI32(_) => ListElementType::Range(RangeType::new(
                RangeBounds::Inclusive,
                RangeEndpointType::I32,
            )),
            Self::Crc32(_) => ListElementType::Scalar(ScalarType::Crc32),
            Self::RowIndexes(_) => ListElementType::Scalar(ScalarType::RowIndex),
            Self::RowKeys(_) => ListElementType::Scalar(ScalarType::RowKey),
            Self::ForeignKeys(_) => ListElementType::Scalar(ScalarType::ForeignKey),
            Self::Pairs { pair_type, .. } => ListElementType::Pair(*pair_type),
        }
    }

    // Every arm reads `values.len()`, but each `values` is a differently typed
    // vector, so the arms cannot be collapsed into one or-pattern.
    #[allow(clippy::match_same_arms)]
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Strings(values) => values.len(),
            Self::Bools(values) => values.len(),
            Self::I8(values) => values.len(),
            Self::I16(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::U8(values) => values.len(),
            Self::U16(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::NonZeroI8(values) => values.len(),
            Self::NonZeroI16(values) => values.len(),
            Self::NonZeroI32(values) => values.len(),
            Self::NonZeroI64(values) => values.len(),
            Self::NonZeroU8(values) => values.len(),
            Self::NonZeroU16(values) => values.len(),
            Self::NonZeroU32(values) => values.len(),
            Self::NonZeroU64(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::LinearRgba(values) => values.len(),
            Self::RangeF32(values) => values.len(),
            Self::RangeInclusiveF32(values) => values.len(),
            Self::RangeU32(values) => values.len(),
            Self::RangeInclusiveU32(values) => values.len(),
            Self::RangeI32(values) => values.len(),
            Self::RangeInclusiveI32(values) => values.len(),
            Self::Crc32(values) => values.len(),
            Self::RowIndexes(values) => values.len(),
            Self::RowKeys(values) => values.len(),
            Self::ForeignKeys(values) => values.len(),
            Self::Pairs { values, .. } => values.len(),
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// UTF-8 span inside a loaded table asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRef {
    pub offset: u32,
    pub len: u32,
}

impl TextRef {
    /// Borrows this span out of the backing table-asset bytes.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] when `offset` or `offset + len`
    /// leaves the address space, when the span runs past the end of `bytes`,
    /// or when those bytes are not valid UTF-8.
    pub fn resolve<'a>(&self, bytes: &'a [u8]) -> Result<&'a str, GameDataError> {
        let start = usize::try_from(self.offset).map_err(|_| {
            GameDataError::Decode(format!("text offset {} exceeds address space", self.offset))
        })?;
        let end = start.checked_add(self.len as usize).ok_or_else(|| {
            GameDataError::Decode(format!(
                "text span {}..{} overflows address space",
                self.offset,
                self.offset.saturating_add(self.len)
            ))
        })?;
        let slice = bytes.get(start..end).ok_or_else(|| {
            GameDataError::Decode(format!(
                "text span {start}..{end} out of bounds in {} table-asset bytes",
                bytes.len()
            ))
        })?;
        std::str::from_utf8(slice).map_err(|err| {
            GameDataError::Decode(format!(
                "text span {start}..{end} is not valid UTF-8: {err}"
            ))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListRef {
    pub offset: u32,
    pub len: u32,
    pub string_pool_base: Option<u32>,
}

impl ListRef {
    /// Borrows this list payload out of the backing table-asset bytes.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] when `offset` or `offset + len`
    /// leaves the address space, or when the span runs past the end of
    /// `bytes`.
    pub fn resolve<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], GameDataError> {
        let start = usize::try_from(self.offset).map_err(|_| {
            GameDataError::Decode(format!("list offset {} exceeds address space", self.offset))
        })?;
        let end = start.checked_add(self.len as usize).ok_or_else(|| {
            GameDataError::Decode(format!(
                "list span {}..{} overflows address space",
                self.offset,
                self.offset.saturating_add(self.len)
            ))
        })?;
        bytes.get(start..end).ok_or_else(|| {
            GameDataError::Decode(format!(
                "list span {start}..{end} out of bounds in {} table-asset bytes",
                bytes.len()
            ))
        })
    }
}

/// Decoded cell payload referencing backing table-asset bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellRef {
    String(TextRef),
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    NonZeroI8(NonZeroI8),
    NonZeroI16(NonZeroI16),
    NonZeroI32(NonZeroI32),
    NonZeroI64(NonZeroI64),
    NonZeroU8(NonZeroU8),
    NonZeroU16(NonZeroU16),
    NonZeroU32(NonZeroU32),
    NonZeroU64(NonZeroU64),
    F32(f32),
    F64(f64),
    LinearRgba(LinearRgba),
    RangeF32(::core::range::Range<f32>),
    RangeInclusiveF32(::core::range::RangeInclusive<f32>),
    RangeU32(::core::range::Range<u32>),
    RangeInclusiveU32(::core::range::RangeInclusive<u32>),
    RangeI32(::core::range::Range<i32>),
    RangeInclusiveI32(::core::range::RangeInclusive<i32>),
    Crc32(u32),
    RowIndex(RowIndex),
    List(ListRef),
}

impl CellRef {
    #[must_use]
    pub fn as_str<'a>(&self, bytes: &'a [u8]) -> Option<&'a str> {
        match self {
            Self::String(text) => text.resolve(bytes).ok(),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F64(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f32(&self) -> Option<f32> {
        match self {
            Self::F32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_linear_rgba(&self) -> Option<LinearRgba> {
        match self {
            Self::LinearRgba(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_range_f32(&self) -> Option<::core::range::Range<f32>> {
        match self {
            Self::RangeF32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_range_inclusive_f32(&self) -> Option<::core::range::RangeInclusive<f32>> {
        match self {
            Self::RangeInclusiveF32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_range_u32(&self) -> Option<::core::range::Range<u32>> {
        match self {
            Self::RangeU32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_range_inclusive_u32(&self) -> Option<::core::range::RangeInclusive<u32>> {
        match self {
            Self::RangeInclusiveU32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_range_i32(&self) -> Option<::core::range::Range<i32>> {
        match self {
            Self::RangeI32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_range_inclusive_i32(&self) -> Option<::core::range::RangeInclusive<i32>> {
        match self {
            Self::RangeInclusiveI32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_crc32(&self) -> Option<u32> {
        match self {
            Self::Crc32(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_row_index(&self) -> Option<RowIndex> {
        match self {
            Self::RowIndex(index) => Some(*index),
            _ => None,
        }
    }
}

/// Parsed table body with string spans into backing table-asset bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct TableBody {
    pub(crate) columns: Vec<ColumnDescriptor>,
    pub(crate) row_key_crcs: Vec<u32>,
    pub(crate) row_guids: Vec<RowGuid>,
    pub(crate) row_names: Vec<Option<TextRef>>,
    /// Column-major storage matching the import encoder (`cells[col][row]`).
    pub(crate) cells: Vec<Vec<Option<CellRef>>>,
    /// Outbound FK/projection edges validated at import time.
    pub(crate) dependencies: Vec<TableDependency>,
}

impl TableBody {
    #[inline]
    #[must_use]
    pub(crate) fn dependencies(&self) -> &[TableDependency] {
        &self.dependencies
    }
}
