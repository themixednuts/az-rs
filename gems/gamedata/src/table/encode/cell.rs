#[cfg(any(feature = "authoring", test))]
use std::borrow::Cow;

use bevy_color::LinearRgba;
#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;

use crate::GameDataError;
use crate::identity::RowIndex;
use crate::table::body::{
    AtomType, CellRef, CellType, ListElementType, PairType, RangeBounds, RangeEndpointType,
    RangeType, ScalarType,
};
#[cfg(any(feature = "authoring", test))]
use crate::table::body::{CellValue, ListValue};
#[cfg(any(feature = "authoring", test))]
use crate::table::encode::pool::StringPoolBuilder;
use crate::table::encode::scalar::{
    read_f32, read_f64, read_i8, read_i16, read_nonzero_i8, read_nonzero_i16, read_nonzero_i32,
    read_nonzero_i64, read_nonzero_u8, read_nonzero_u16, read_nonzero_u32, read_nonzero_u64,
    read_u8, read_u16, read_u32, read_vlq_i32,
};
#[cfg(any(feature = "authoring", test))]
use crate::table::encode::text::write_string_ref;
use crate::table::encode::text::{read_list_ref, read_pooled_string_ref, read_text_ref};
use crate::table::vlq;

const CELL_TYPE_SCALAR: u8 = 1;
const CELL_TYPE_RANGE: u8 = 2;
const CELL_TYPE_LIST: u8 = 3;
const LIST_ELEMENT_SCALAR: u8 = 1;
const LIST_ELEMENT_RANGE: u8 = 2;
const LIST_ELEMENT_PAIR: u8 = 3;
const ATOM_SCALAR: u8 = 1;
const ATOM_RANGE: u8 = 2;

#[cfg(any(feature = "authoring", test))]
pub(super) fn write_cell_type(bytes: &mut Vec<u8>, cell_type: CellType) {
    match cell_type {
        CellType::Scalar(scalar) => {
            bytes.put_u8(CELL_TYPE_SCALAR);
            bytes.put_u8(scalar.id());
        }
        CellType::Range(range) => {
            bytes.put_u8(CELL_TYPE_RANGE);
            write_range_type(bytes, range);
        }
        CellType::List(element) => {
            bytes.put_u8(CELL_TYPE_LIST);
            write_list_element_type(bytes, element);
        }
    }
}

pub(super) fn read_cell_type(data: &mut &[u8], field: &str) -> Result<CellType, GameDataError> {
    match read_u8(data, &format!("{field}.kind"))? {
        CELL_TYPE_SCALAR => {
            read_scalar_type(data, &format!("{field}.scalar")).map(CellType::Scalar)
        }
        CELL_TYPE_RANGE => read_range_type(data, &format!("{field}.range")).map(CellType::Range),
        CELL_TYPE_LIST => {
            read_list_element_type(data, &format!("{field}.list")).map(CellType::List)
        }
        tag => Err(GameDataError::Decode(format!(
            "{field}.kind has unknown CellType tag {tag:#x}"
        ))),
    }
}

pub fn read_list_element_type(
    data: &mut &[u8],
    field: &str,
) -> Result<ListElementType, GameDataError> {
    match read_u8(data, &format!("{field}.kind"))? {
        LIST_ELEMENT_SCALAR => {
            read_scalar_type(data, &format!("{field}.scalar")).map(ListElementType::Scalar)
        }
        LIST_ELEMENT_RANGE => {
            read_range_type(data, &format!("{field}.range")).map(ListElementType::Range)
        }
        LIST_ELEMENT_PAIR => {
            read_pair_type(data, &format!("{field}.pair")).map(ListElementType::Pair)
        }
        tag => Err(GameDataError::Decode(format!(
            "{field}.kind has unknown ListElementType tag {tag:#x}"
        ))),
    }
}

#[cfg(any(feature = "authoring", test))]
fn write_list_element_type(bytes: &mut Vec<u8>, element_type: ListElementType) {
    match element_type {
        ListElementType::Scalar(scalar) => {
            bytes.put_u8(LIST_ELEMENT_SCALAR);
            bytes.put_u8(scalar.id());
        }
        ListElementType::Range(range) => {
            bytes.put_u8(LIST_ELEMENT_RANGE);
            write_range_type(bytes, range);
        }
        ListElementType::Pair(pair) => {
            bytes.put_u8(LIST_ELEMENT_PAIR);
            write_pair_type(bytes, pair);
        }
    }
}

#[cfg(any(feature = "authoring", test))]
fn write_atom_type(bytes: &mut Vec<u8>, atom: AtomType) {
    match atom {
        AtomType::Scalar(scalar) => {
            bytes.put_u8(ATOM_SCALAR);
            bytes.put_u8(scalar.id());
        }
        AtomType::Range(range) => {
            bytes.put_u8(ATOM_RANGE);
            write_range_type(bytes, range);
        }
    }
}

fn read_atom_type(data: &mut &[u8], field: &str) -> Result<AtomType, GameDataError> {
    match read_u8(data, &format!("{field}.kind"))? {
        ATOM_SCALAR => read_scalar_type(data, &format!("{field}.scalar")).map(AtomType::Scalar),
        ATOM_RANGE => read_range_type(data, &format!("{field}.range")).map(AtomType::Range),
        tag => Err(GameDataError::Decode(format!(
            "{field}.kind has unknown AtomType tag {tag:#x}"
        ))),
    }
}

#[cfg(any(feature = "authoring", test))]
fn write_pair_type(bytes: &mut Vec<u8>, pair: PairType) {
    write_atom_type(bytes, pair.first);
    write_atom_type(bytes, pair.second);
}

fn read_pair_type(data: &mut &[u8], field: &str) -> Result<PairType, GameDataError> {
    let first = read_atom_type(data, &format!("{field}.first"))?;
    let second = read_atom_type(data, &format!("{field}.second"))?;
    Ok(PairType::new(first, second))
}

fn read_scalar_type(data: &mut &[u8], field: &str) -> Result<ScalarType, GameDataError> {
    let id = read_u8(data, field)?;
    ScalarType::from_id(id)
        .ok_or_else(|| GameDataError::Decode(format!("{field} has unknown ScalarType id {id:#x}")))
}

#[cfg(any(feature = "authoring", test))]
fn write_range_type(bytes: &mut Vec<u8>, range: RangeType) {
    bytes.put_u8(range.bounds.id());
    bytes.put_u8(range.endpoint.id());
}

fn read_range_type(data: &mut &[u8], field: &str) -> Result<RangeType, GameDataError> {
    let bounds_id = read_u8(data, &format!("{field}.bounds"))?;
    let endpoint_id = read_u8(data, &format!("{field}.endpoint"))?;
    let bounds = RangeBounds::from_id(bounds_id).ok_or_else(|| {
        GameDataError::Decode(format!(
            "{field}.bounds has unknown RangeBounds id {bounds_id:#x}"
        ))
    })?;
    let endpoint = RangeEndpointType::from_id(endpoint_id).ok_or_else(|| {
        GameDataError::Decode(format!(
            "{field}.endpoint has unknown RangeEndpointType id {endpoint_id:#x}"
        ))
    })?;
    Ok(RangeType::new(bounds, endpoint))
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn write_cell_value(
    bytes: &mut Vec<u8>,
    cell: &CellValue<'_>,
    cell_type: CellType,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    match (cell_type, cell) {
        (CellType::List(element_type), CellValue::List(value)) => {
            write_list_cell(bytes, value, element_type, string_pool, use_string_pool)
        }
        (CellType::Scalar(scalar_type), cell) => {
            write_scalar_cell(bytes, cell, scalar_type, string_pool, use_string_pool)
        }
        (CellType::Range(range_type), cell) => write_range_cell(bytes, cell, range_type),
        (cell_type, cell) => Err(cell_type_mismatch(cell, cell_type)),
    }
}

#[cfg(any(feature = "authoring", test))]
fn cell_type_mismatch(cell: &CellValue<'_>, cell_type: CellType) -> GameDataError {
    GameDataError::Decode(format!(
        "cell value {cell:?} does not match column cell type {cell_type:?}"
    ))
}

#[cfg(any(feature = "authoring", test))]
fn write_list_cell(
    bytes: &mut Vec<u8>,
    value: &ListValue<'_>,
    element_type: ListElementType,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    if value.element_type() != element_type {
        return Err(GameDataError::Decode(format!(
            "list value element type {:?} does not match column element type {:?}",
            value.element_type(),
            element_type
        )));
    }
    let mut payload = Vec::new();
    write_list_element_type(&mut payload, value.element_type());
    payload.put_u32_le(
        u32::try_from(value.len())
            .map_err(|_| GameDataError::Decode("list element count exceeds u32".into()))?,
    );
    write_list_elements(&mut payload, value, string_pool, use_string_pool)?;
    bytes.put_u32_le(
        u32::try_from(payload.len())
            .map_err(|_| GameDataError::Decode("list payload length exceeds u32".into()))?,
    );
    bytes.put_slice(&payload);
    Ok(())
}

#[cfg(any(feature = "authoring", test))]
fn write_scalar_cell(
    bytes: &mut Vec<u8>,
    cell: &CellValue<'_>,
    scalar_type: ScalarType,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    match (scalar_type, cell) {
        (ScalarType::String, CellValue::String(value))
        | (ScalarType::RowKey, CellValue::RowKey(value)) => {
            return write_text_value(bytes, value, string_pool, use_string_pool);
        }
        (ScalarType::Bool, CellValue::Bool(value)) => bytes.put_u8(u8::from(*value)),
        (ScalarType::I8, CellValue::I8(value)) => bytes.put_i8(*value),
        (ScalarType::I16, CellValue::I16(value)) => bytes.put_i16_le(*value),
        (ScalarType::I32, CellValue::I32(value)) => vlq::write_i64(bytes, i64::from(*value)),
        (ScalarType::I64, CellValue::I64(value)) => vlq::write_i64(bytes, *value),
        (ScalarType::U8, CellValue::U8(value)) => bytes.put_u8(*value),
        (ScalarType::U16, CellValue::U16(value)) => bytes.put_u16_le(*value),
        (ScalarType::U32, CellValue::U32(value)) => vlq::write_u32(bytes, *value),
        (ScalarType::U64, CellValue::U64(value)) => vlq::write_u64(bytes, *value),
        (ScalarType::NonZeroI8, CellValue::NonZeroI8(value)) => bytes.put_i8(value.get()),
        (ScalarType::NonZeroI16, CellValue::NonZeroI16(value)) => bytes.put_i16_le(value.get()),
        (ScalarType::NonZeroI32, CellValue::NonZeroI32(value)) => {
            vlq::write_i64(bytes, i64::from(value.get()));
        }
        (ScalarType::NonZeroI64, CellValue::NonZeroI64(value)) => {
            vlq::write_i64(bytes, value.get());
        }
        (ScalarType::NonZeroU8, CellValue::NonZeroU8(value)) => bytes.put_u8(value.get()),
        (ScalarType::NonZeroU16, CellValue::NonZeroU16(value)) => bytes.put_u16_le(value.get()),
        (ScalarType::NonZeroU32, CellValue::NonZeroU32(value)) => {
            vlq::write_u32(bytes, value.get());
        }
        (ScalarType::NonZeroU64, CellValue::NonZeroU64(value)) => {
            vlq::write_u64(bytes, value.get());
        }
        (ScalarType::F32, CellValue::F32(value)) => bytes.put_f32_le(*value),
        (ScalarType::F64, CellValue::F64(value)) => bytes.put_f64_le(*value),
        (ScalarType::LinearRgba, CellValue::LinearRgba(value)) => {
            bytes.put_f32_le(value.red);
            bytes.put_f32_le(value.green);
            bytes.put_f32_le(value.blue);
            bytes.put_f32_le(value.alpha);
        }
        (ScalarType::Crc32, CellValue::Crc32(value)) => bytes.put_u32_le(*value),
        (ScalarType::ForeignKey, CellValue::ForeignKey(value)) => {
            vlq::write_u32(bytes, value.row_index().one_based().get());
        }
        (ScalarType::RowIndex, CellValue::RowIndex(value)) => {
            vlq::write_u32(bytes, value.one_based().get());
        }
        (scalar_type, cell) => {
            return Err(cell_type_mismatch(cell, CellType::Scalar(scalar_type)));
        }
    }
    Ok(())
}

#[cfg(any(feature = "authoring", test))]
fn write_range_cell(
    bytes: &mut Vec<u8>,
    cell: &CellValue<'_>,
    range_type: RangeType,
) -> Result<(), GameDataError> {
    match (range_type.bounds, range_type.endpoint, cell) {
        (RangeBounds::Exclusive, RangeEndpointType::F32, CellValue::RangeF32(value)) => {
            bytes.put_f32_le(value.start);
            bytes.put_f32_le(value.end);
        }
        (RangeBounds::Inclusive, RangeEndpointType::F32, CellValue::RangeInclusiveF32(value)) => {
            bytes.put_f32_le(value.start);
            bytes.put_f32_le(value.last);
        }
        (RangeBounds::Exclusive, RangeEndpointType::U32, CellValue::RangeU32(value)) => {
            vlq::write_u32(bytes, value.start);
            vlq::write_u32(bytes, value.end);
        }
        (RangeBounds::Inclusive, RangeEndpointType::U32, CellValue::RangeInclusiveU32(value)) => {
            vlq::write_u32(bytes, value.start);
            vlq::write_u32(bytes, value.last);
        }
        (RangeBounds::Exclusive, RangeEndpointType::I32, CellValue::RangeI32(value)) => {
            vlq::write_i64(bytes, i64::from(value.start));
            vlq::write_i64(bytes, i64::from(value.end));
        }
        (RangeBounds::Inclusive, RangeEndpointType::I32, CellValue::RangeInclusiveI32(value)) => {
            vlq::write_i64(bytes, i64::from(value.start));
            vlq::write_i64(bytes, i64::from(value.last));
        }
        (_, _, cell) => return Err(cell_type_mismatch(cell, CellType::Range(range_type))),
    }
    Ok(())
}

#[cfg(any(feature = "authoring", test))]
fn write_text_value(
    bytes: &mut Vec<u8>,
    value: &str,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    if use_string_pool {
        let (offset, len) = string_pool.offsets(value)?;
        write_string_ref(bytes, offset, len);
    } else {
        let len = u32::try_from(value.len())
            .map_err(|_| GameDataError::Decode("string length exceeds u32".into()))?;
        bytes.put_u32_le(len);
        bytes.put_slice(value.as_bytes());
    }
    Ok(())
}

/// Expands the `ListValue` arms whose elements are written identically: a fixed
/// element type plus the `CellValue` constructor that rewraps each item. That
/// pair is the entire difference between them, so listing it beats 26 copies of
/// the same six-argument call. Variants with per-element structure are written
/// out after the table, and the compiler still checks the match is exhaustive.
#[cfg(any(feature = "authoring", test))]
macro_rules! list_element_arms {
    (
        ($bytes:expr, $value:expr, $string_pool:expr, $use_string_pool:expr)
        uniform { $( $variant:ident => $element:expr, $cell:expr; )+ }
        $( $rest:tt )*
    ) => {
        match $value {
            $(
                ListValue::$variant(values) => write_scalar_list_items(
                    $bytes,
                    values,
                    $element,
                    $cell,
                    $string_pool,
                    $use_string_pool,
                ),
            )+
            $( $rest )*
        }
    };
}

#[cfg(any(feature = "authoring", test))]
fn write_list_elements(
    bytes: &mut Vec<u8>,
    value: &ListValue<'_>,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    list_element_arms! {
        (bytes, value, string_pool, use_string_pool)
        uniform {
            Bools => ListElementType::Scalar(ScalarType::Bool), CellValue::Bool;
            I8 => ListElementType::Scalar(ScalarType::I8), CellValue::I8;
            I16 => ListElementType::Scalar(ScalarType::I16), CellValue::I16;
            I32 => ListElementType::Scalar(ScalarType::I32), CellValue::I32;
            I64 => ListElementType::Scalar(ScalarType::I64), CellValue::I64;
            U8 => ListElementType::Scalar(ScalarType::U8), CellValue::U8;
            U16 => ListElementType::Scalar(ScalarType::U16), CellValue::U16;
            U32 => ListElementType::Scalar(ScalarType::U32), CellValue::U32;
            U64 => ListElementType::Scalar(ScalarType::U64), CellValue::U64;
            NonZeroI8 => ListElementType::Scalar(ScalarType::NonZeroI8), CellValue::NonZeroI8;
            NonZeroI16 => ListElementType::Scalar(ScalarType::NonZeroI16), CellValue::NonZeroI16;
            NonZeroI32 => ListElementType::Scalar(ScalarType::NonZeroI32), CellValue::NonZeroI32;
            NonZeroI64 => ListElementType::Scalar(ScalarType::NonZeroI64), CellValue::NonZeroI64;
            NonZeroU8 => ListElementType::Scalar(ScalarType::NonZeroU8), CellValue::NonZeroU8;
            NonZeroU16 => ListElementType::Scalar(ScalarType::NonZeroU16), CellValue::NonZeroU16;
            NonZeroU32 => ListElementType::Scalar(ScalarType::NonZeroU32), CellValue::NonZeroU32;
            NonZeroU64 => ListElementType::Scalar(ScalarType::NonZeroU64), CellValue::NonZeroU64;
            F32 => ListElementType::Scalar(ScalarType::F32), CellValue::F32;
            F64 => ListElementType::Scalar(ScalarType::F64), CellValue::F64;
            LinearRgba => ListElementType::Scalar(ScalarType::LinearRgba), CellValue::LinearRgba;
            RangeF32 =>
                ListElementType::Range(RangeType::new(RangeBounds::Exclusive, RangeEndpointType::F32)),
                CellValue::RangeF32;
            RangeInclusiveF32 =>
                ListElementType::Range(RangeType::new(RangeBounds::Inclusive, RangeEndpointType::F32)),
                CellValue::RangeInclusiveF32;
            RangeU32 =>
                ListElementType::Range(RangeType::new(RangeBounds::Exclusive, RangeEndpointType::U32)),
                CellValue::RangeU32;
            RangeInclusiveU32 =>
                ListElementType::Range(RangeType::new(RangeBounds::Inclusive, RangeEndpointType::U32)),
                CellValue::RangeInclusiveU32;
            RangeI32 =>
                ListElementType::Range(RangeType::new(RangeBounds::Exclusive, RangeEndpointType::I32)),
                CellValue::RangeI32;
            RangeInclusiveI32 =>
                ListElementType::Range(RangeType::new(RangeBounds::Inclusive, RangeEndpointType::I32)),
                CellValue::RangeInclusiveI32;
            Crc32 => ListElementType::Scalar(ScalarType::Crc32), CellValue::Crc32;
            RowIndexes => ListElementType::Scalar(ScalarType::RowIndex), CellValue::RowIndex;
        }
        ListValue::Strings(values) => write_string_list_elements(
            bytes,
            values,
            ScalarType::String,
            string_pool,
            use_string_pool,
        ),
        ListValue::RowKeys(values) => write_string_list_elements(
            bytes,
            values,
            ScalarType::RowKey,
            string_pool,
            use_string_pool,
        ),
        ListValue::ForeignKeys(values) => {
            for (index, value) in values.iter().enumerate() {
                write_list_element(
                    bytes,
                    &CellValue::foreign_key(*value),
                    ListElementType::Scalar(ScalarType::ForeignKey),
                    index,
                    string_pool,
                    use_string_pool,
                )?;
            }
            Ok(())
        },
        ListValue::Pairs { pair_type, values } => {
            for (index, value) in values.iter().enumerate() {
                write_pair_list_element(
                    bytes,
                    value.first(),
                    value.second(),
                    *pair_type,
                    index,
                    string_pool,
                    use_string_pool,
                )?;
            }
            Ok(())
        },
    }
}

#[cfg(any(feature = "authoring", test))]
fn write_scalar_list_items<T: Copy>(
    bytes: &mut Vec<u8>,
    values: &[T],
    element_type: ListElementType,
    cell: impl Fn(T) -> CellValue<'static>,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    for (index, value) in values.iter().copied().enumerate() {
        write_list_element(
            bytes,
            &cell(value),
            element_type,
            index,
            string_pool,
            use_string_pool,
        )?;
    }
    Ok(())
}

#[cfg(any(feature = "authoring", test))]
fn write_string_list_elements(
    bytes: &mut Vec<u8>,
    values: &[Cow<'_, str>],
    scalar_type: ScalarType,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    for (index, value) in values.iter().enumerate() {
        write_list_text_element(
            bytes,
            value.as_ref(),
            scalar_type,
            index,
            string_pool,
            use_string_pool,
        )?;
    }
    Ok(())
}

#[cfg(any(feature = "authoring", test))]
fn write_list_text_element(
    bytes: &mut Vec<u8>,
    value: &str,
    scalar_type: ScalarType,
    index: usize,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    let result = match scalar_type {
        ScalarType::String | ScalarType::RowKey => {
            write_text_value(bytes, value, string_pool, use_string_pool)
        }
        ScalarType::ForeignKey => Err(GameDataError::Decode(
            "foreign-key list elements must be resolved row indexes before encode".into(),
        )),
        other => Err(GameDataError::Decode(format!(
            "list text element cannot be encoded as {other:?}"
        ))),
    };
    result
        .map_err(|err| GameDataError::Decode(format!("list element {index} encode failed: {err}")))
}

#[cfg(any(feature = "authoring", test))]
fn write_list_element(
    bytes: &mut Vec<u8>,
    cell: &CellValue<'_>,
    element_type: ListElementType,
    index: usize,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    let Some(atom_type) = element_type.atom_type() else {
        return Err(GameDataError::Decode(format!(
            "list element {index} type {element_type:?} is not a scalar/range atom"
        )));
    };
    write_cell_value(
        bytes,
        cell,
        atom_type.cell_type(),
        string_pool,
        use_string_pool,
    )
    .map_err(|err| GameDataError::Decode(format!("list element {index} encode failed: {err}")))
}

#[cfg(any(feature = "authoring", test))]
fn write_pair_list_element(
    bytes: &mut Vec<u8>,
    first: &CellValue<'_>,
    second: &CellValue<'_>,
    pair_type: PairType,
    index: usize,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<(), GameDataError> {
    write_cell_value(
        bytes,
        first,
        pair_type.first.cell_type(),
        string_pool,
        use_string_pool,
    )
    .and_then(|()| {
        write_cell_value(
            bytes,
            second,
            pair_type.second.cell_type(),
            string_pool,
            use_string_pool,
        )
    })
    .map_err(|err| GameDataError::Decode(format!("list element {index} encode failed: {err}")))
}

pub(super) fn read_cell_ref(
    bytes: &[u8],
    data: &mut &[u8],
    cell_type: CellType,
    column_index: usize,
    row_index: usize,
    pool_base: Option<u32>,
) -> Result<CellRef, GameDataError> {
    match cell_type {
        CellType::List(_) => read_list_ref(
            bytes,
            data,
            pool_base,
            &format!("cells[{column_index}][{row_index}].list"),
        )
        .map(CellRef::List),
        CellType::Scalar(scalar_type) => {
            read_scalar_cell_ref(bytes, data, scalar_type, column_index, row_index, pool_base)
        }
        CellType::Range(range_type) => {
            read_range_cell_ref(data, range_type, column_index, row_index)
        }
    }
}

fn read_scalar_cell_ref(
    bytes: &[u8],
    data: &mut &[u8],
    scalar_type: ScalarType,
    column_index: usize,
    row_index: usize,
    pool_base: Option<u32>,
) -> Result<CellRef, GameDataError> {
    let at = |suffix: &str| format!("cells[{column_index}][{row_index}].{suffix}");
    match scalar_type {
        ScalarType::String | ScalarType::RowKey => {
            let field = at("string");
            if let Some(pool_base) = pool_base {
                read_pooled_string_ref(bytes, data, pool_base, &field).map(CellRef::String)
            } else {
                read_text_ref(bytes, data, &field).map(CellRef::String)
            }
        }
        ScalarType::ForeignKey => {
            let one_based = vlq::read_u32(data, &at("foreign_key.row_index"))?;
            let index = RowIndex::from_one_based(one_based).ok_or_else(|| {
                GameDataError::Decode(format!(
                    "cells[{column_index}][{row_index}] invalid foreign-key RowIndex one-based value {one_based}"
                ))
            })?;
            Ok(CellRef::RowIndex(index))
        }
        ScalarType::RowIndex => {
            let one_based = vlq::read_u32(data, &at("row_index"))?;
            let index = RowIndex::from_one_based(one_based).ok_or_else(|| {
                GameDataError::Decode(format!(
                    "cells[{column_index}][{row_index}] invalid RowIndex one-based value {one_based}"
                ))
            })?;
            Ok(CellRef::RowIndex(index))
        }
        ScalarType::Bool => read_u8(data, &at("bool")).map(|value| CellRef::Bool(value != 0)),
        ScalarType::I8 => read_i8(data, &at("i8")).map(CellRef::I8),
        ScalarType::I16 => read_i16(data, &at("i16")).map(CellRef::I16),
        ScalarType::I32 => read_vlq_i32(data, &at("i32")).map(CellRef::I32),
        ScalarType::I64 => vlq::read_i64(data, &at("i64")).map(CellRef::I64),
        ScalarType::U8 => read_u8(data, &at("u8")).map(CellRef::U8),
        ScalarType::U16 => read_u16(data, &at("u16")).map(CellRef::U16),
        ScalarType::U32 => vlq::read_u32(data, &at("u32")).map(CellRef::U32),
        ScalarType::U64 => vlq::read_u64(data, &at("u64")).map(CellRef::U64),
        ScalarType::NonZeroI8 => read_nonzero_i8(data, &at("nonzero_i8")).map(CellRef::NonZeroI8),
        ScalarType::NonZeroI16 => {
            read_nonzero_i16(data, &at("nonzero_i16")).map(CellRef::NonZeroI16)
        }
        ScalarType::NonZeroI32 => {
            read_nonzero_i32(data, &at("nonzero_i32")).map(CellRef::NonZeroI32)
        }
        ScalarType::NonZeroI64 => {
            read_nonzero_i64(data, &at("nonzero_i64")).map(CellRef::NonZeroI64)
        }
        ScalarType::NonZeroU8 => read_nonzero_u8(data, &at("nonzero_u8")).map(CellRef::NonZeroU8),
        ScalarType::NonZeroU16 => {
            read_nonzero_u16(data, &at("nonzero_u16")).map(CellRef::NonZeroU16)
        }
        ScalarType::NonZeroU32 => {
            read_nonzero_u32(data, &at("nonzero_u32")).map(CellRef::NonZeroU32)
        }
        ScalarType::NonZeroU64 => {
            read_nonzero_u64(data, &at("nonzero_u64")).map(CellRef::NonZeroU64)
        }
        ScalarType::F32 => read_f32(data, &at("f32")).map(CellRef::F32),
        ScalarType::F64 => read_f64(data, &at("f64")).map(CellRef::F64),
        ScalarType::LinearRgba => {
            read_linear_rgba(data, &at("linear_rgba")).map(CellRef::LinearRgba)
        }
        ScalarType::Crc32 => read_u32(data, &at("crc32")).map(CellRef::Crc32),
    }
}

fn read_range_cell_ref(
    data: &mut &[u8],
    range_type: RangeType,
    column_index: usize,
    row_index: usize,
) -> Result<CellRef, GameDataError> {
    let at = |suffix: &str| format!("cells[{column_index}][{row_index}].{suffix}");
    match (range_type.bounds, range_type.endpoint) {
        (RangeBounds::Exclusive, RangeEndpointType::F32) => {
            Ok(CellRef::RangeF32(::core::range::Range {
                start: read_f32(data, &at("range_f32.start"))?,
                end: read_f32(data, &at("range_f32.end"))?,
            }))
        }
        (RangeBounds::Inclusive, RangeEndpointType::F32) => {
            Ok(CellRef::RangeInclusiveF32(::core::range::RangeInclusive {
                start: read_f32(data, &at("range_inclusive_f32.start"))?,
                last: read_f32(data, &at("range_inclusive_f32.last"))?,
            }))
        }
        (RangeBounds::Exclusive, RangeEndpointType::U32) => {
            Ok(CellRef::RangeU32(::core::range::Range {
                start: vlq::read_u32(data, &at("range_u32.start"))?,
                end: vlq::read_u32(data, &at("range_u32.end"))?,
            }))
        }
        (RangeBounds::Inclusive, RangeEndpointType::U32) => {
            Ok(CellRef::RangeInclusiveU32(::core::range::RangeInclusive {
                start: vlq::read_u32(data, &at("range_inclusive_u32.start"))?,
                last: vlq::read_u32(data, &at("range_inclusive_u32.last"))?,
            }))
        }
        (RangeBounds::Exclusive, RangeEndpointType::I32) => {
            Ok(CellRef::RangeI32(::core::range::Range {
                start: read_vlq_i32(data, &at("range_i32.start"))?,
                end: read_vlq_i32(data, &at("range_i32.end"))?,
            }))
        }
        (RangeBounds::Inclusive, RangeEndpointType::I32) => {
            Ok(CellRef::RangeInclusiveI32(::core::range::RangeInclusive {
                start: read_vlq_i32(data, &at("range_inclusive_i32.start"))?,
                last: read_vlq_i32(data, &at("range_inclusive_i32.last"))?,
            }))
        }
    }
}

pub fn read_list_element_cell_ref(
    bytes: &[u8],
    data: &mut &[u8],
    element_type: ListElementType,
    pool_base: Option<u32>,
) -> Result<CellRef, GameDataError> {
    let Some(atom_type) = element_type.atom_type() else {
        return Err(GameDataError::Decode(format!(
            "list element type {element_type:?} is not a scalar/range atom"
        )));
    };
    read_atom_cell_ref(bytes, data, atom_type, pool_base)
}

pub fn read_atom_cell_ref(
    bytes: &[u8],
    data: &mut &[u8],
    atom_type: AtomType,
    pool_base: Option<u32>,
) -> Result<CellRef, GameDataError> {
    read_cell_ref(bytes, data, atom_type.cell_type(), 0, 0, pool_base)
}

fn read_linear_rgba(data: &mut &[u8], field: &str) -> Result<LinearRgba, GameDataError> {
    Ok(LinearRgba::new(
        read_f32(data, &format!("{field}.red"))?,
        read_f32(data, &format!("{field}.green"))?,
        read_f32(data, &format!("{field}.blue"))?,
        read_f32(data, &format!("{field}.alpha"))?,
    ))
}
