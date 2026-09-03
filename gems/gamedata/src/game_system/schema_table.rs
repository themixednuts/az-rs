use std::any::Any;
use std::marker::PhantomData;
use std::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64,
};
use std::ops::Deref;
use std::sync::Arc;

use az_core::crc::Crc32;
use glam::Vec4;

use super::{AnyColumnSlot, AnyRowRef, AnyTableRef};
use crate::game_system::read_list_count;
use crate::table::{AtomType, CellRef, CellType, ListElementType, ScalarType};
use crate::{GameDataError, GameDataSchemaRow, RowIndex, TableId};

/// A physical table viewed through one generated merged row schema.
#[derive(Debug)]
pub struct SchemaTable<R> {
    data: Arc<SchemaTableData<R>>,
}

/// An owning handle to one row in a materialized [`SchemaTable`].
///
/// The row and its table projection stay alive even after the runtime cache
/// releases the underlying table product.
#[derive(Debug)]
pub struct SchemaRow<R> {
    data: Arc<SchemaTableData<R>>,
    index: u32,
}

#[derive(Debug)]
pub(super) struct SchemaTableData<R> {
    name: Box<str>,
    rows: Box<[R]>,
    rows_by_key_crc: rustc_hash::FxHashMap<u32, u32>,
}

impl<R> Clone for SchemaTable<R> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

impl<R> Clone for SchemaRow<R> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            index: self.index,
        }
    }
}

#[derive(Debug)]
struct SchemaDecoder<'a, R> {
    table: AnyTableRef<'a>,
    columns: Vec<Option<AnyColumnSlot>>,
    marker: PhantomData<fn() -> R>,
}

/// One row in a [`SchemaTable`] during generated row materialization.
#[derive(Debug, Clone, Copy)]
pub struct SchemaRowRef<'table, 'asset, R> {
    decoder: &'table SchemaDecoder<'asset, R>,
    row: AnyRowRef<'asset>,
}

/// Runtime decoding contract for a field in a generated merged row schema.
pub trait SchemaValue: Sized {
    fn accepts(cell_type: CellType) -> bool;

    /// Decodes one cell, or `None` when the row leaves that cell absent.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] when `column` is out of range for the
    /// row, when the stored cell holds a different `CellRef` variant than this
    /// implementation accepts, or when a string or list cell points outside
    /// the table-asset bytes.
    fn read(row: AnyRowRef<'_>, column: AnyColumnSlot) -> Result<Option<Self>, GameDataError>;
}

impl<R> SchemaTable<R>
where
    R: GameDataSchemaRow,
{
    pub(super) fn materialize(
        table: AnyTableRef<'_>,
    ) -> Result<Arc<SchemaTableData<R>>, GameDataError> {
        let mut columns = Vec::with_capacity(R::SCHEMA.column_count());
        for descriptor in R::SCHEMA.columns() {
            let column = table
                .slot
                .index()?
                .columns
                .get(&descriptor.source_column_crc())
                .copied();
            let Some(index) = column else {
                if descriptor.is_required() {
                    return Err(GameDataError::Decode(format!(
                        "table `{}` is missing required {} field `{}` (source column `{}`)",
                        table.logical_name,
                        R::SCHEMA.name(),
                        descriptor.field_name(),
                        descriptor.source_column_name(),
                    )));
                }
                columns.push(None);
                continue;
            };
            let actual = table
                .body()?
                .columns
                .get(index as usize)
                .ok_or_else(|| {
                    GameDataError::Decode(format!(
                        "table `{}` column `{}` index {index} is out of range",
                        table.logical_name,
                        descriptor.source_column_name(),
                    ))
                })?
                .cell_type;
            if actual != descriptor.cell_type() {
                return Err(GameDataError::Decode(format!(
                    "table `{}` field `{}` has cell type {actual:?}, expected {:?}",
                    table.logical_name,
                    descriptor.field_name(),
                    descriptor.cell_type(),
                )));
            }
            columns.push(Some(AnyColumnSlot {
                index,
                column: descriptor.source_column_name(),
            }));
        }

        let decoder = SchemaDecoder {
            table,
            columns,
            marker: PhantomData,
        };
        let mut rows = Vec::with_capacity(table.len());
        for row in table.rows() {
            let row = R::decode(SchemaRowRef {
                decoder: &decoder,
                row,
            })?;
            rows.push(row);
        }
        Ok(Arc::new(SchemaTableData {
            name: table.logical_name().into(),
            rows: rows.into_boxed_slice(),
            rows_by_key_crc: table.slot.index()?.rows.clone(),
        }))
    }

    pub(super) fn from_cached(cached: Arc<dyn Any + Send + Sync>) -> Result<Self, GameDataError> {
        let data = cached.downcast::<SchemaTableData<R>>().map_err(|_| {
            GameDataError::Decode(format!(
                "cached GameData projection type mismatch for {}",
                std::any::type_name::<R>()
            ))
        })?;
        Ok(Self { data })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.data.name
    }

    #[must_use]
    pub fn id(&self) -> TableId {
        TableId::from_name(self.name())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.data.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.rows.is_empty()
    }

    #[must_use]
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &R> + '_ {
        self.data.rows.iter()
    }

    /// Iterate owning row handles suitable for retaining in semantic managers.
    ///
    /// # Panics
    ///
    /// Panics if the table holds more than `u32::MAX` rows, which the compiled
    /// table header cannot express.
    #[must_use]
    pub fn row_handles(&self) -> impl ExactSizeIterator<Item = SchemaRow<R>> + '_ {
        (0..self.data.rows.len()).map(|index| SchemaRow {
            data: self.data.clone(),
            index: u32::try_from(index).expect("GameData row index exceeds u32"),
        })
    }

    #[must_use]
    pub fn row_at(&self, index: RowIndex) -> Option<&R> {
        self.data.rows.get(index.zero_based() as usize)
    }

    #[must_use]
    pub fn row_handle_at(&self, index: RowIndex) -> Option<SchemaRow<R>> {
        self.data.rows.get(index.zero_based() as usize)?;
        Some(SchemaRow {
            data: self.data.clone(),
            index: index.zero_based(),
        })
    }

    #[must_use]
    pub fn get_by_key_crc(&self, key: impl Into<Crc32>) -> Option<&R> {
        let row = self.data.rows_by_key_crc.get(&key.into().value())?;
        self.data.rows.get(*row as usize)
    }

    #[must_use]
    pub fn get_handle_by_key_crc(&self, key: impl Into<Crc32>) -> Option<SchemaRow<R>> {
        let index = *self.data.rows_by_key_crc.get(&key.into().value())?;
        self.data.rows.get(index as usize)?;
        Some(SchemaRow {
            data: self.data.clone(),
            index,
        })
    }
}

impl<R> SchemaRow<R> {
    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.data.name
    }

    #[must_use]
    pub fn table_id(&self) -> TableId {
        TableId::from_name(self.table_name())
    }

    /// Returns this row's one-based index within its table.
    ///
    /// # Panics
    ///
    /// Panics if the stored zero-based index is `u32::MAX`, so that its
    /// one-based form wraps to zero. Handles are only ever minted from a real
    /// row position, so that cannot happen.
    #[must_use]
    pub fn row_index(&self) -> RowIndex {
        RowIndex::from_one_based(self.index + 1).expect("SchemaRow always stores a valid row index")
    }
}

impl<R> Deref for SchemaRow<R> {
    type Target = R;

    fn deref(&self) -> &Self::Target {
        &self.data.rows[self.index as usize]
    }
}

impl<R> AsRef<R> for SchemaRow<R> {
    fn as_ref(&self) -> &R {
        self
    }
}

impl<R: PartialEq> PartialEq for SchemaRow<R> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<R: Eq> Eq for SchemaRow<R> {}

impl<R> SchemaRowRef<'_, '_, R>
where
    R: GameDataSchemaRow,
{
    /// Decodes one schema field, or `None` when the physical table omits it.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] when `field_index` is past the end of
    /// `R::SCHEMA`, when `T` cannot decode the column's declared cell type, or
    /// when [`SchemaValue::read`] rejects the stored cell.
    pub fn get<T>(&self, field_index: usize) -> Result<Option<T>, GameDataError>
    where
        T: SchemaValue,
    {
        let descriptor = R::SCHEMA.column(field_index).ok_or_else(|| {
            GameDataError::Decode(format!(
                "row schema `{}` has no field at index {field_index}",
                R::SCHEMA.name(),
            ))
        })?;
        if !T::accepts(descriptor.cell_type()) {
            return Err(GameDataError::Decode(format!(
                "row schema `{}` field `{}` cannot decode {:?} into {}",
                R::SCHEMA.name(),
                descriptor.field_name(),
                descriptor.cell_type(),
                std::any::type_name::<T>(),
            )));
        }
        let Some(column) = self.decoder.columns[field_index] else {
            return Ok(None);
        };
        T::read(self.row, column)
    }

    /// Decodes one schema field that the row must carry.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::get`] returns, plus
    /// [`GameDataError::Decode`] naming the field when the cell is absent.
    pub fn require<T>(&self, field_index: usize) -> Result<T, GameDataError>
    where
        T: SchemaValue,
    {
        self.get(field_index)?.ok_or_else(|| {
            let field = R::SCHEMA
                .column(field_index)
                .map_or("<unknown>", |field| field.field_name());
            GameDataError::Decode(format!(
                "table `{}` row {} is missing required field `{field}`",
                self.decoder.table.logical_name(),
                self.row.zero_based_index(),
            ))
        })
    }
}

trait SchemaAtom: Sized {
    fn accepts(atom_type: AtomType) -> bool;
    fn read(cell: CellRef, bytes: &[u8]) -> Result<Self, GameDataError>;
}

impl<T> SchemaValue for T
where
    T: SchemaAtom,
{
    fn accepts(cell_type: CellType) -> bool {
        match cell_type {
            CellType::Scalar(scalar) => T::accepts(AtomType::Scalar(scalar)),
            CellType::Range(range) => T::accepts(AtomType::Range(range)),
            CellType::List(_) => false,
        }
    }

    fn read(row: AnyRowRef<'_>, column: AnyColumnSlot) -> Result<Option<Self>, GameDataError> {
        row.table
            .cell_ref_at(row.row, column.index)
            .map(|cell| T::read(cell, row.table.bytes()))
            .transpose()
    }
}

macro_rules! schema_atom {
    ($ty:ty, [$($scalar:pat),+ $(,)?], $cell:pat => $value:expr) => {
        impl SchemaAtom for $ty {
            fn accepts(atom_type: AtomType) -> bool {
                matches!(atom_type, $(AtomType::Scalar($scalar))|+)
            }

            fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
                match cell {
                    $cell => Ok($value),
                    other => Err(GameDataError::Decode(format!(
                        "cell {other:?} cannot decode as {}",
                        std::any::type_name::<Self>(),
                    ))),
                }
            }
        }
    };
}

schema_atom!(bool, [ScalarType::Bool], CellRef::Bool(value) => value);
schema_atom!(f32, [ScalarType::F32], CellRef::F32(value) => value);
schema_atom!(f64, [ScalarType::F64], CellRef::F64(value) => value);
schema_atom!(NonZeroI8, [ScalarType::NonZeroI8], CellRef::NonZeroI8(value) => value);
schema_atom!(NonZeroI16, [ScalarType::NonZeroI16], CellRef::NonZeroI16(value) => value);
schema_atom!(NonZeroI32, [ScalarType::NonZeroI32], CellRef::NonZeroI32(value) => value);
schema_atom!(NonZeroI64, [ScalarType::NonZeroI64], CellRef::NonZeroI64(value) => value);
schema_atom!(NonZeroU8, [ScalarType::NonZeroU8], CellRef::NonZeroU8(value) => value);
schema_atom!(NonZeroU16, [ScalarType::NonZeroU16], CellRef::NonZeroU16(value) => value);
schema_atom!(NonZeroU32, [ScalarType::NonZeroU32], CellRef::NonZeroU32(value) => value);
schema_atom!(NonZeroU64, [ScalarType::NonZeroU64], CellRef::NonZeroU64(value) => value);

macro_rules! integer_schema_atom {
    ($ty:ty, $scalar:ident, $cell:ident, $non_zero_scalar:ident, $non_zero_cell:ident) => {
        impl SchemaAtom for $ty {
            fn accepts(atom_type: AtomType) -> bool {
                matches!(
                    atom_type,
                    AtomType::Scalar(ScalarType::$scalar | ScalarType::$non_zero_scalar)
                )
            }

            fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
                match cell {
                    CellRef::$cell(value) => Ok(value),
                    CellRef::$non_zero_cell(value) => Ok(value.get()),
                    other => Err(GameDataError::Decode(format!(
                        "cell {other:?} cannot decode as {}",
                        std::any::type_name::<Self>(),
                    ))),
                }
            }
        }
    };
}

integer_schema_atom!(i8, I8, I8, NonZeroI8, NonZeroI8);
integer_schema_atom!(i16, I16, I16, NonZeroI16, NonZeroI16);
integer_schema_atom!(i32, I32, I32, NonZeroI32, NonZeroI32);
integer_schema_atom!(i64, I64, I64, NonZeroI64, NonZeroI64);
integer_schema_atom!(u8, U8, U8, NonZeroU8, NonZeroU8);
integer_schema_atom!(u16, U16, U16, NonZeroU16, NonZeroU16);

impl SchemaAtom for u32 {
    fn accepts(atom_type: AtomType) -> bool {
        matches!(
            atom_type,
            AtomType::Scalar(
                ScalarType::U32
                    | ScalarType::NonZeroU32
                    | ScalarType::Crc32
                    | ScalarType::RowIndex
                    | ScalarType::ForeignKey
            )
        )
    }

    fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
        match cell {
            CellRef::U32(value) | CellRef::Crc32(value) => Ok(value),
            CellRef::NonZeroU32(value) => Ok(value.get()),
            CellRef::RowIndex(value) => Ok(value.one_based().get()),
            other => Err(GameDataError::Decode(format!(
                "cell {other:?} cannot decode as u32"
            ))),
        }
    }
}

impl SchemaAtom for u64 {
    fn accepts(atom_type: AtomType) -> bool {
        matches!(
            atom_type,
            AtomType::Scalar(ScalarType::U64 | ScalarType::NonZeroU64)
        )
    }

    fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
        match cell {
            CellRef::U64(value) => Ok(value),
            CellRef::NonZeroU64(value) => Ok(value.get()),
            other => Err(GameDataError::Decode(format!(
                "cell {other:?} cannot decode as u64"
            ))),
        }
    }
}

impl SchemaAtom for Crc32 {
    fn accepts(atom_type: AtomType) -> bool {
        atom_type == AtomType::Scalar(ScalarType::Crc32)
    }

    fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
        match cell {
            CellRef::Crc32(value) => Ok(Self::from(value)),
            other => Err(GameDataError::Decode(format!(
                "cell {other:?} cannot decode as Crc32"
            ))),
        }
    }
}

impl SchemaAtom for RowIndex {
    fn accepts(atom_type: AtomType) -> bool {
        matches!(
            atom_type,
            AtomType::Scalar(ScalarType::RowIndex | ScalarType::ForeignKey)
        )
    }

    fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
        match cell {
            CellRef::RowIndex(value) => Ok(value),
            other => Err(GameDataError::Decode(format!(
                "cell {other:?} cannot decode as RowIndex"
            ))),
        }
    }
}

macro_rules! range_schema_atom {
    ($ty:ty, $range:expr, $cell:ident) => {
        impl SchemaAtom for $ty {
            fn accepts(atom_type: AtomType) -> bool {
                atom_type == AtomType::Range($range)
            }

            fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
                match cell {
                    CellRef::$cell(value) => Ok(value),
                    other => Err(GameDataError::Decode(format!(
                        "cell {other:?} cannot decode as {}",
                        std::any::type_name::<Self>(),
                    ))),
                }
            }
        }
    };
}

range_schema_atom!(
    ::core::range::Range<f32>,
    crate::RangeType::new(crate::RangeBounds::Exclusive, crate::RangeEndpointType::F32),
    RangeF32
);
range_schema_atom!(
    ::core::range::RangeInclusive<f32>,
    crate::RangeType::new(crate::RangeBounds::Inclusive, crate::RangeEndpointType::F32),
    RangeInclusiveF32
);
range_schema_atom!(
    ::core::range::Range<i32>,
    crate::RangeType::new(crate::RangeBounds::Exclusive, crate::RangeEndpointType::I32),
    RangeI32
);
range_schema_atom!(
    ::core::range::RangeInclusive<i32>,
    crate::RangeType::new(crate::RangeBounds::Inclusive, crate::RangeEndpointType::I32),
    RangeInclusiveI32
);
range_schema_atom!(
    ::core::range::Range<u32>,
    crate::RangeType::new(crate::RangeBounds::Exclusive, crate::RangeEndpointType::U32),
    RangeU32
);
range_schema_atom!(
    ::core::range::RangeInclusive<u32>,
    crate::RangeType::new(crate::RangeBounds::Inclusive, crate::RangeEndpointType::U32),
    RangeInclusiveU32
);

impl SchemaAtom for String {
    fn accepts(atom_type: AtomType) -> bool {
        matches!(
            atom_type,
            AtomType::Scalar(ScalarType::String | ScalarType::RowKey) | AtomType::Range(_)
        )
    }

    fn read(cell: CellRef, bytes: &[u8]) -> Result<Self, GameDataError> {
        render_cell(cell, bytes)
    }
}

impl SchemaAtom for Vec4 {
    fn accepts(atom_type: AtomType) -> bool {
        atom_type == AtomType::Scalar(ScalarType::LinearRgba)
    }

    fn read(cell: CellRef, _bytes: &[u8]) -> Result<Self, GameDataError> {
        match cell {
            CellRef::LinearRgba(value) => {
                Ok(Self::new(value.red, value.green, value.blue, value.alpha))
            }
            other => Err(GameDataError::Decode(format!(
                "cell {other:?} cannot decode as glam::Vec4"
            ))),
        }
    }
}

macro_rules! list_schema_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SchemaValue for Vec<$ty> {
                fn accepts(cell_type: CellType) -> bool {
                    matches!(
                        cell_type,
                        CellType::List(element)
                            if element.atom_type().is_some_and(<$ty as SchemaAtom>::accepts)
                    )
                }

                fn read(
                    row: AnyRowRef<'_>,
                    column: AnyColumnSlot,
                ) -> Result<Option<Self>, GameDataError> {
                    read_atom_list::<$ty>(row, column)
                }
            }
        )+
    };
}

list_schema_value!(
    bool,
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    f32,
    f64,
    NonZeroI8,
    NonZeroI16,
    NonZeroI32,
    NonZeroI64,
    NonZeroU8,
    NonZeroU16,
    NonZeroU32,
    NonZeroU64,
    Crc32,
    RowIndex,
    ::core::range::Range<f32>,
    ::core::range::RangeInclusive<f32>,
    ::core::range::Range<i32>,
    ::core::range::RangeInclusive<i32>,
    ::core::range::Range<u32>,
    ::core::range::RangeInclusive<u32>,
    Vec4,
);

impl<A, B> SchemaValue for Vec<(A, B)>
where
    A: SchemaAtom,
    B: SchemaAtom,
{
    fn accepts(cell_type: CellType) -> bool {
        matches!(
            cell_type,
            CellType::List(ListElementType::Pair(pair))
                if A::accepts(pair.first) && B::accepts(pair.second)
        )
    }

    fn read(row: AnyRowRef<'_>, column: AnyColumnSlot) -> Result<Option<Self>, GameDataError> {
        let Some(CellRef::List(list)) = row.table.cell_ref_at(row.row, column.index) else {
            return Ok(None);
        };
        let bytes = row.table.bytes();
        let mut data = list.resolve(bytes)?;
        let element_type = crate::table::encode::read_list_element_type(&mut data, "list.element")?;
        let ListElementType::Pair(pair) = element_type else {
            return Err(GameDataError::Decode(format!(
                "table `{}` column `{}` list element {element_type:?} cannot decode as {}",
                row.table.logical_name,
                column.column,
                std::any::type_name::<Self>(),
            )));
        };
        if !A::accepts(pair.first) || !B::accepts(pair.second) {
            return Err(GameDataError::Decode(format!(
                "table `{}` column `{}` pair {pair:?} cannot decode as {}",
                row.table.logical_name,
                column.column,
                std::any::type_name::<Self>(),
            )));
        }
        let count = read_list_count(&mut data, row.table.logical_name, column.column)?;
        let mut values = Self::with_capacity(count);
        for _ in 0..count {
            let first = crate::table::encode::read_atom_cell_ref(
                bytes,
                &mut data,
                pair.first,
                list.string_pool_base,
            )?;
            let second = crate::table::encode::read_atom_cell_ref(
                bytes,
                &mut data,
                pair.second,
                list.string_pool_base,
            )?;
            values.push((A::read(first, bytes)?, B::read(second, bytes)?));
        }
        Ok(Some(values))
    }
}

impl SchemaValue for Vec<String> {
    fn accepts(cell_type: CellType) -> bool {
        matches!(cell_type, CellType::List(_))
    }

    fn read(row: AnyRowRef<'_>, column: AnyColumnSlot) -> Result<Option<Self>, GameDataError> {
        let Some(CellRef::List(list)) = row.table.cell_ref_at(row.row, column.index) else {
            return Ok(None);
        };
        let bytes = row.table.bytes();
        let mut data = list.resolve(bytes)?;
        let element_type = crate::table::encode::read_list_element_type(&mut data, "list.element")?;
        let count = read_list_count(&mut data, row.table.logical_name, column.column)?;
        let mut values = Self::with_capacity(count);
        for _ in 0..count {
            match element_type {
                ListElementType::Scalar(_) | ListElementType::Range(_) => {
                    let cell = crate::table::encode::read_list_element_cell_ref(
                        bytes,
                        &mut data,
                        element_type,
                        list.string_pool_base,
                    )?;
                    values.push(render_cell(cell, bytes)?);
                }
                ListElementType::Pair(pair) => {
                    let first = crate::table::encode::read_atom_cell_ref(
                        bytes,
                        &mut data,
                        pair.first,
                        list.string_pool_base,
                    )?;
                    let second = crate::table::encode::read_atom_cell_ref(
                        bytes,
                        &mut data,
                        pair.second,
                        list.string_pool_base,
                    )?;
                    values.push(format!(
                        "(first: {}, second: {})",
                        render_cell(first, bytes)?,
                        render_cell(second, bytes)?,
                    ));
                }
            }
        }
        Ok(Some(values))
    }
}

fn read_atom_list<T>(
    row: AnyRowRef<'_>,
    column: AnyColumnSlot,
) -> Result<Option<Vec<T>>, GameDataError>
where
    T: SchemaAtom,
{
    let Some(CellRef::List(list)) = row.table.cell_ref_at(row.row, column.index) else {
        return Ok(None);
    };
    let bytes = row.table.bytes();
    let mut data = list.resolve(bytes)?;
    let element_type = crate::table::encode::read_list_element_type(&mut data, "list.element")?;
    let atom_type = element_type.atom_type().ok_or_else(|| {
        GameDataError::Decode(format!(
            "table `{}` column `{}` pair list cannot decode as {}",
            row.table.logical_name,
            column.column,
            std::any::type_name::<T>(),
        ))
    })?;
    if !T::accepts(atom_type) {
        return Err(GameDataError::Decode(format!(
            "table `{}` column `{}` list element {element_type:?} cannot decode as {}",
            row.table.logical_name,
            column.column,
            std::any::type_name::<T>(),
        )));
    }
    let count = read_list_count(&mut data, row.table.logical_name, column.column)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let cell = crate::table::encode::read_list_element_cell_ref(
            bytes,
            &mut data,
            element_type,
            list.string_pool_base,
        )?;
        values.push(T::read(cell, bytes)?);
    }
    Ok(Some(values))
}

// The numeric arms read alike but bind different scalar types, so they cannot
// be collapsed into one or-pattern.
#[allow(clippy::match_same_arms)]
fn render_cell(cell: CellRef, bytes: &[u8]) -> Result<String, GameDataError> {
    match cell {
        CellRef::String(value) => value.resolve(bytes).map(str::to_owned),
        CellRef::Bool(value) => Ok(value.to_string()),
        CellRef::I8(value) => Ok(value.to_string()),
        CellRef::I16(value) => Ok(value.to_string()),
        CellRef::I32(value) => Ok(value.to_string()),
        CellRef::I64(value) => Ok(value.to_string()),
        CellRef::U8(value) => Ok(value.to_string()),
        CellRef::U16(value) => Ok(value.to_string()),
        CellRef::U32(value) => Ok(value.to_string()),
        CellRef::U64(value) => Ok(value.to_string()),
        CellRef::NonZeroI8(value) => Ok(value.to_string()),
        CellRef::NonZeroI16(value) => Ok(value.to_string()),
        CellRef::NonZeroI32(value) => Ok(value.to_string()),
        CellRef::NonZeroI64(value) => Ok(value.to_string()),
        CellRef::NonZeroU8(value) => Ok(value.to_string()),
        CellRef::NonZeroU16(value) => Ok(value.to_string()),
        CellRef::NonZeroU32(value) => Ok(value.to_string()),
        CellRef::NonZeroU64(value) => Ok(value.to_string()),
        CellRef::F32(value) => Ok(value.to_string()),
        CellRef::F64(value) => Ok(value.to_string()),
        CellRef::Crc32(value) => Ok(value.to_string()),
        CellRef::RowIndex(value) => Ok(value.one_based().get().to_string()),
        CellRef::LinearRgba(value) => Ok(format!(
            "(red: {}, green: {}, blue: {}, alpha: {})",
            value.red, value.green, value.blue, value.alpha,
        )),
        CellRef::RangeF32(value) => Ok(format!("(start: {}, end: {})", value.start, value.end)),
        CellRef::RangeInclusiveF32(value) => {
            Ok(format!("(start: {}, last: {})", value.start, value.last))
        }
        CellRef::RangeU32(value) => Ok(format!("(start: {}, end: {})", value.start, value.end)),
        CellRef::RangeInclusiveU32(value) => {
            Ok(format!("(start: {}, last: {})", value.start, value.last))
        }
        CellRef::RangeI32(value) => Ok(format!("(start: {}, end: {})", value.start, value.end)),
        CellRef::RangeInclusiveI32(value) => {
            Ok(format!("(start: {}, last: {})", value.start, value.last))
        }
        CellRef::List(_) => Err(GameDataError::Decode(
            "nested GameData lists are not supported".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_rust_value_types_cover_compiled_semantic_scalars() {
        assert!(<u32 as SchemaValue>::accepts(CellType::Scalar(
            ScalarType::Crc32
        )));
        assert!(<u32 as SchemaValue>::accepts(CellType::Scalar(
            ScalarType::ForeignKey
        )));
        assert!(<String as SchemaValue>::accepts(CellType::Range(
            crate::RangeType::new(crate::RangeBounds::Inclusive, crate::RangeEndpointType::F32,)
        )));
        assert!(<Vec<String> as SchemaValue>::accepts(CellType::List(
            ListElementType::Pair(crate::PairType::new(
                AtomType::Scalar(ScalarType::String),
                AtomType::Scalar(ScalarType::U32),
            )),
        )));
    }
}
