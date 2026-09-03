//! Cell and descriptor primitives shared by merged row schemas and semantic
//! manager projections.

use std::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64,
};

use az_core::crc::Crc32;
use bevy_color::LinearRgba;

use crate::identity::RowIndex;
use crate::table::{
    AtomType, CellRef, CellType, ListElementType, RangeBounds, RangeEndpointType, RangeType,
    ScalarType,
};

mod sealed {
    pub trait EnumRepresentationSeal {}
    pub trait RangeEndpointSeal {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignKeyMeta {
    table: &'static str,
    row: &'static str,
    column: &'static str,
    table_crc: u32,
    row_crc: u32,
    column_crc: u32,
}

impl ForeignKeyMeta {
    #[inline]
    #[must_use]
    pub const fn new(table: &'static str, row: &'static str, column: &'static str) -> Self {
        Self::from_crcs(
            table,
            row,
            column,
            Crc32::from_str_lower(table).value(),
            Crc32::from_str_lower(row).value(),
            Crc32::from_str_lower(column).value(),
        )
    }

    #[inline]
    #[must_use]
    pub const fn from_crcs(
        table: &'static str,
        row: &'static str,
        column: &'static str,
        table_crc: u32,
        row_crc: u32,
        column_crc: u32,
    ) -> Self {
        Self {
            table,
            row,
            column,
            table_crc,
            row_crc,
            column_crc,
        }
    }

    #[inline]
    #[must_use]
    pub const fn target_table(self) -> &'static str {
        self.table
    }

    #[inline]
    #[must_use]
    pub const fn target_row(self) -> &'static str {
        self.row
    }

    #[inline]
    #[must_use]
    pub const fn target_column(self) -> &'static str {
        self.column
    }

    #[inline]
    #[must_use]
    pub const fn target_table_crc(self) -> u32 {
        self.table_crc
    }

    #[inline]
    #[must_use]
    pub const fn target_row_crc(self) -> u32 {
        self.row_crc
    }

    #[inline]
    #[must_use]
    pub const fn target_column_crc(self) -> u32 {
        self.column_crc
    }
}

/// Column schema used by row descriptors and authored-table validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSchema {
    name: &'static str,
    column: &'static str,
    cell_type: CellType,
    row_key: bool,
    required: bool,
    foreign_keys: &'static [ForeignKeyMeta],
    enum_variants: &'static [EnumVariantMeta],
}

impl ColumnSchema {
    #[inline]
    #[must_use]
    pub(crate) const fn from_descriptor(
        descriptor: crate::descriptor::ColumnSchemaDescriptor,
        row_key: bool,
    ) -> Self {
        Self {
            name: descriptor.field_name(),
            column: descriptor.source_column_name(),
            cell_type: descriptor.cell_type(),
            row_key,
            required: row_key || descriptor.is_required(),
            foreign_keys: descriptor.foreign_key_targets(),
            enum_variants: descriptor.enum_variants(),
        }
    }

    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    #[inline]
    #[must_use]
    pub const fn column(self) -> &'static str {
        self.column
    }

    #[inline]
    #[must_use]
    pub const fn cell_type(self) -> CellType {
        self.cell_type
    }

    #[inline]
    #[must_use]
    pub const fn is_row_key(self) -> bool {
        self.row_key
    }

    #[inline]
    #[must_use]
    pub const fn is_required(self) -> bool {
        self.required
    }

    #[inline]
    #[must_use]
    pub const fn list_element_type(self) -> Option<ListElementType> {
        self.cell_type.list_element_type()
    }

    #[inline]
    #[must_use]
    pub const fn foreign_keys(self) -> &'static [ForeignKeyMeta] {
        self.foreign_keys
    }

    #[inline]
    #[must_use]
    pub const fn enum_variants(self) -> &'static [EnumVariantMeta] {
        self.enum_variants
    }

    #[inline]
    #[must_use]
    pub const fn column_crc(self) -> u32 {
        Crc32::from_str_lower(self.column).value()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumVariantMeta {
    name: &'static str,
    source_tokens: &'static [&'static str],
    discriminant: i64,
}

impl EnumVariantMeta {
    #[inline]
    #[must_use]
    pub const fn new(
        name: &'static str,
        source_tokens: &'static [&'static str],
        discriminant: i64,
    ) -> Self {
        Self {
            name,
            source_tokens,
            discriminant,
        }
    }

    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    #[inline]
    #[must_use]
    pub const fn source_tokens(self) -> &'static [&'static str] {
        self.source_tokens
    }

    #[inline]
    #[must_use]
    pub const fn discriminant(self) -> i64 {
        self.discriminant
    }

    #[inline]
    #[must_use]
    pub fn matches(self, value: &str) -> bool {
        self.name.eq_ignore_ascii_case(value)
            || self
                .source_tokens
                .iter()
                .any(|token| token.eq_ignore_ascii_case(value))
    }
}

/// Borrowed cell value type for a generated column marker.
pub trait Cell<'a>: Sized {
    const CELL_TYPE: CellType;

    fn read(cell: CellRef, bytes: &'a [u8]) -> Option<Self>;
}

pub trait Atom<'a>: Sized {
    const ATOM_TYPE: AtomType;

    fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self>;
}

pub trait EnumRepresentation<'a>: sealed::EnumRepresentationSeal + Copy {
    const SCALAR_TYPE: ScalarType;

    fn read_enum_cell(cell: CellRef, bytes: &'a [u8]) -> Option<Self>;
}

pub trait TableEnum<'a>: Copy {
    type Representation: EnumRepresentation<'a>;

    fn from_representation(value: Self::Representation) -> Option<Self>;
}

impl<'a, T> Cell<'a> for T
where
    T: TableEnum<'a>,
{
    const CELL_TYPE: CellType = CellType::Scalar(T::Representation::SCALAR_TYPE);

    #[inline]
    fn read(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        T::Representation::read_enum_cell(cell, bytes).and_then(T::from_representation)
    }
}

impl<'a, T> Atom<'a> for T
where
    T: TableEnum<'a>,
{
    const ATOM_TYPE: AtomType = AtomType::Scalar(T::Representation::SCALAR_TYPE);

    #[inline]
    fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        <Self as Cell<'a>>::read(cell, bytes)
    }
}

impl<'a> Cell<'a> for &'a str {
    const CELL_TYPE: CellType = CellType::Scalar(ScalarType::String);

    #[inline]
    fn read(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        match cell {
            CellRef::String(text) => text.resolve(bytes).ok(),
            _ => None,
        }
    }
}

impl<'a> Atom<'a> for &'a str {
    const ATOM_TYPE: AtomType = AtomType::Scalar(ScalarType::String);

    #[inline]
    fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        <&'a str as Cell<'a>>::read(cell, bytes)
    }
}

macro_rules! scalar_cell {
    ($ty:ty, $scalar:ident, $variant:ident) => {
        impl<'a> Cell<'a> for $ty {
            const CELL_TYPE: CellType = CellType::Scalar(ScalarType::$scalar);

            #[inline]
            fn read(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
                match cell {
                    CellRef::$variant(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl<'a> Atom<'a> for $ty {
            const ATOM_TYPE: AtomType = AtomType::Scalar(ScalarType::$scalar);

            #[inline]
            fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
                <$ty as Cell<'a>>::read(cell, bytes)
            }
        }
    };
}

scalar_cell!(f64, F64, F64);
scalar_cell!(f32, F32, F32);
scalar_cell!(i8, I8, I8);
scalar_cell!(i16, I16, I16);
scalar_cell!(i32, I32, I32);
scalar_cell!(i64, I64, I64);
scalar_cell!(u8, U8, U8);
scalar_cell!(u16, U16, U16);
scalar_cell!(u32, U32, U32);
scalar_cell!(u64, U64, U64);
scalar_cell!(NonZeroI8, NonZeroI8, NonZeroI8);
scalar_cell!(NonZeroI16, NonZeroI16, NonZeroI16);
scalar_cell!(NonZeroI32, NonZeroI32, NonZeroI32);
scalar_cell!(NonZeroI64, NonZeroI64, NonZeroI64);
scalar_cell!(NonZeroU8, NonZeroU8, NonZeroU8);
scalar_cell!(NonZeroU16, NonZeroU16, NonZeroU16);
scalar_cell!(NonZeroU32, NonZeroU32, NonZeroU32);
scalar_cell!(NonZeroU64, NonZeroU64, NonZeroU64);
scalar_cell!(LinearRgba, LinearRgba, LinearRgba);
scalar_cell!(bool, Bool, Bool);
scalar_cell!(RowIndex, RowIndex, RowIndex);

impl sealed::EnumRepresentationSeal for u8 {}

impl<'a> EnumRepresentation<'a> for u8 {
    const SCALAR_TYPE: ScalarType = ScalarType::U8;

    #[inline]
    fn read_enum_cell(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        match cell {
            CellRef::U8(value) => Some(value),
            _ => None,
        }
    }
}

impl sealed::EnumRepresentationSeal for i32 {}

impl<'a> EnumRepresentation<'a> for i32 {
    const SCALAR_TYPE: ScalarType = ScalarType::I32;

    #[inline]
    fn read_enum_cell(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        match cell {
            CellRef::I32(value) => Some(value),
            _ => None,
        }
    }
}

impl sealed::EnumRepresentationSeal for u32 {}

impl<'a> EnumRepresentation<'a> for u32 {
    const SCALAR_TYPE: ScalarType = ScalarType::U32;

    #[inline]
    fn read_enum_cell(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        match cell {
            CellRef::U32(value) => Some(value),
            _ => None,
        }
    }
}

impl sealed::EnumRepresentationSeal for Crc32 {}

impl<'a> EnumRepresentation<'a> for Crc32 {
    const SCALAR_TYPE: ScalarType = ScalarType::Crc32;

    #[inline]
    fn read_enum_cell(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        match cell {
            CellRef::Crc32(value) => Some(Self::from_u32(value)),
            _ => None,
        }
    }
}

pub trait RangeEndpoint: sealed::RangeEndpointSeal + Copy {
    const RANGE_ENDPOINT_TYPE: RangeEndpointType;

    fn range(cell: CellRef) -> Option<::core::range::Range<Self>>;
    fn range_inclusive(cell: CellRef) -> Option<::core::range::RangeInclusive<Self>>;
}

macro_rules! range_endpoint {
    ($ty:ty, $endpoint:ident, $range:ident, $inclusive:ident) => {
        impl sealed::RangeEndpointSeal for $ty {}

        impl RangeEndpoint for $ty {
            const RANGE_ENDPOINT_TYPE: RangeEndpointType = RangeEndpointType::$endpoint;

            #[inline]
            fn range(cell: CellRef) -> Option<::core::range::Range<Self>> {
                match cell {
                    CellRef::$range(value) => Some(value),
                    _ => None,
                }
            }

            #[inline]
            fn range_inclusive(cell: CellRef) -> Option<::core::range::RangeInclusive<Self>> {
                match cell {
                    CellRef::$inclusive(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

range_endpoint!(f32, F32, RangeF32, RangeInclusiveF32);
range_endpoint!(i32, I32, RangeI32, RangeInclusiveI32);
range_endpoint!(u32, U32, RangeU32, RangeInclusiveU32);

impl<'a, T: RangeEndpoint> Cell<'a> for ::core::range::Range<T> {
    const CELL_TYPE: CellType = CellType::Range(RangeType::new(
        RangeBounds::Exclusive,
        T::RANGE_ENDPOINT_TYPE,
    ));

    #[inline]
    fn read(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        T::range(cell)
    }
}

impl<'a, T: RangeEndpoint> Atom<'a> for ::core::range::Range<T> {
    const ATOM_TYPE: AtomType = AtomType::Range(RangeType::new(
        RangeBounds::Exclusive,
        T::RANGE_ENDPOINT_TYPE,
    ));

    #[inline]
    fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        <Self as Cell<'a>>::read(cell, bytes)
    }
}

impl<'a, T: RangeEndpoint> Cell<'a> for ::core::range::RangeInclusive<T> {
    const CELL_TYPE: CellType = CellType::Range(RangeType::new(
        RangeBounds::Inclusive,
        T::RANGE_ENDPOINT_TYPE,
    ));

    #[inline]
    fn read(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        T::range_inclusive(cell)
    }
}

impl<'a, T: RangeEndpoint> Atom<'a> for ::core::range::RangeInclusive<T> {
    const ATOM_TYPE: AtomType = AtomType::Range(RangeType::new(
        RangeBounds::Inclusive,
        T::RANGE_ENDPOINT_TYPE,
    ));

    #[inline]
    fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        <Self as Cell<'a>>::read(cell, bytes)
    }
}

impl<'a> Cell<'a> for Crc32 {
    const CELL_TYPE: CellType = CellType::Scalar(ScalarType::Crc32);

    #[inline]
    fn read(cell: CellRef, _bytes: &'a [u8]) -> Option<Self> {
        match cell {
            CellRef::Crc32(value) => Some(Self::from_u32(value)),
            _ => None,
        }
    }
}

impl<'a> Atom<'a> for Crc32 {
    const ATOM_TYPE: AtomType = AtomType::Scalar(ScalarType::Crc32);

    #[inline]
    fn read_atom(cell: CellRef, bytes: &'a [u8]) -> Option<Self> {
        <Self as Cell<'a>>::read(cell, bytes)
    }
}
