use az_core::crc::Crc32;
use bevy_color::LinearRgba;
use ron::Value;
use ron::value::Number;

use crate::GameDataError;
use crate::descriptor::AuthoringTableSchema as TableSchema;
use crate::identity::RowIndex;
use crate::table::encode::{CellValue, ListValue};
use crate::table::{
    AtomType, ListElementType, PairType, PairValue, RangeBounds, RangeEndpointType, RangeType,
    ScalarType,
};
use crate::table_set::ColumnSchema;

use super::number::{
    number_to_f32, number_to_f64, number_to_i8, number_to_i16, number_to_i32, number_to_i64,
    number_to_nonzero_i8, number_to_nonzero_i16, number_to_nonzero_i32, number_to_nonzero_i64,
    number_to_nonzero_u8, number_to_nonzero_u16, number_to_nonzero_u32, number_to_nonzero_u64,
    number_to_u8, number_to_u16, number_to_u32, number_to_u64,
};
use super::source::ParsedCell;
use super::value::{
    enum_cell_value_from_discriminant_for_scalar, enum_discriminant_from_string,
    linear_rgba_from_hex, range_f32_value, range_i32_value, range_inclusive_f32_value,
    range_inclusive_i32_value, range_inclusive_u32_value, range_u32_value, value_kind,
};

pub(super) fn list_cell_from_values(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    values: &[Value],
) -> Result<ParsedCell, GameDataError> {
    let element_type = field.list_element_type().ok_or_else(|| {
        GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` is a list without an element type",
            table.name(),
            field.name()
        ))
    })?;
    if !field.enum_variants().is_empty()
        && !matches!(element_type, ListElementType::Scalar(ScalarType::String))
    {
        return enum_list_cell_from_values(table, field, row_index, values, element_type);
    }
    match element_type {
        ListElementType::Scalar(scalar) => {
            scalar_list_cell(table, field, row_index, values, scalar)
        }
        ListElementType::Range(range_type) => {
            range_list_cell(table, field, row_index, values, range_type)
        }
        ListElementType::Pair(pair_type) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                pair_list_element(table, field, row_index, index, value, pair_type)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| ListValue::Pairs { pair_type, values })
            .map(CellValue::List)
            .map(ParsedCell::Value),
    }
}

/// Expands the numeric scalar-list arms, which differ only in the `Number`
/// converter and the `ListValue` constructor. The remaining element types carry
/// their own parsing and are written out after the table; the compiler still
/// checks the match is exhaustive.
macro_rules! scalar_list_arms {
    (
        ($table:expr, $field:expr, $row_index:expr, $values:expr, $scalar:expr)
        numbers { $( $variant:ident => $convert:expr, $list:expr; )+ }
        $( $rest:tt )*
    ) => {
        match $scalar {
            $(
                ScalarType::$variant => $values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        number_list_element($table, $field, $row_index, index, value, $convert)
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map($list)
                    .map(CellValue::List)
                    .map(ParsedCell::Value),
            )+
            $( $rest )*
        }
    };
}

fn scalar_list_cell(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    values: &[Value],
    scalar: ScalarType,
) -> Result<ParsedCell, GameDataError> {
    scalar_list_arms! {
        (table, field, row_index, values, scalar)
        numbers {
            I8 => number_to_i8, ListValue::I8;
            I16 => number_to_i16, ListValue::I16;
            I32 => number_to_i32, ListValue::I32;
            I64 => number_to_i64, ListValue::I64;
            U8 => number_to_u8, ListValue::U8;
            U16 => number_to_u16, ListValue::U16;
            U32 => number_to_u32, ListValue::U32;
            U64 => number_to_u64, ListValue::U64;
            NonZeroI8 => number_to_nonzero_i8, ListValue::NonZeroI8;
            NonZeroI16 => number_to_nonzero_i16, ListValue::NonZeroI16;
            NonZeroI32 => number_to_nonzero_i32, ListValue::NonZeroI32;
            NonZeroI64 => number_to_nonzero_i64, ListValue::NonZeroI64;
            NonZeroU8 => number_to_nonzero_u8, ListValue::NonZeroU8;
            NonZeroU16 => number_to_nonzero_u16, ListValue::NonZeroU16;
            NonZeroU32 => number_to_nonzero_u32, ListValue::NonZeroU32;
            NonZeroU64 => number_to_nonzero_u64, ListValue::NonZeroU64;
            F32 => |value| Ok(number_to_f32(value)), ListValue::F32;
            F64 => |value| Ok(number_to_f64(value)), ListValue::F64;
        }
        ScalarType::String => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                string_list_element(table, field, row_index, index, value).map(str::to_owned)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::strings)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        ScalarType::RowKey => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                string_list_element(table, field, row_index, index, value).map(str::to_owned)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::row_keys)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        ScalarType::ForeignKey => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                string_list_element(table, field, row_index, index, value).map(str::to_owned)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(ParsedCell::ForeignKeys),
        ScalarType::Bool => values
            .iter()
            .enumerate()
            .map(|(index, value)| bool_list_element(table, field, row_index, index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::Bools)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        ScalarType::LinearRgba => values
            .iter()
            .enumerate()
            .map(|(index, value)| color_list_element(table, field, row_index, index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::LinearRgba)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        ScalarType::Crc32 => values
            .iter()
            .enumerate()
            .map(|(index, value)| crc32_list_element(table, field, row_index, index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::Crc32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        ScalarType::RowIndex => values
            .iter()
            .enumerate()
            .map(|(index, value)| row_index_list_element(table, field, row_index, index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RowIndexes)
            .map(CellValue::List)
            .map(ParsedCell::Value),
    }
}

fn range_list_cell(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    values: &[Value],
    range_type: RangeType,
) -> Result<ParsedCell, GameDataError> {
    match (range_type.bounds, range_type.endpoint) {
        (RangeBounds::Exclusive, RangeEndpointType::F32) => values
            .iter()
            .map(|value| range_f32_value(table, field, row_index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RangeF32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        (RangeBounds::Inclusive, RangeEndpointType::F32) => values
            .iter()
            .map(|value| range_inclusive_f32_value(table, field, row_index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RangeInclusiveF32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        (RangeBounds::Exclusive, RangeEndpointType::U32) => values
            .iter()
            .map(|value| range_u32_value(table, field, row_index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RangeU32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        (RangeBounds::Inclusive, RangeEndpointType::U32) => values
            .iter()
            .map(|value| range_inclusive_u32_value(table, field, row_index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RangeInclusiveU32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        (RangeBounds::Exclusive, RangeEndpointType::I32) => values
            .iter()
            .map(|value| range_i32_value(table, field, row_index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RangeI32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
        (RangeBounds::Inclusive, RangeEndpointType::I32) => values
            .iter()
            .map(|value| range_inclusive_i32_value(table, field, row_index, value))
            .collect::<Result<Vec<_>, _>>()
            .map(ListValue::RangeInclusiveI32)
            .map(CellValue::List)
            .map(ParsedCell::Value),
    }
}

fn enum_list_cell_from_values(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    values: &[Value],
    element_type: ListElementType,
) -> Result<ParsedCell, GameDataError> {
    let ListElementType::Scalar(scalar) = element_type else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` has enum metadata on non-scalar list type {:?}",
            table.name(),
            field.name(),
            element_type
        )));
    };

    match scalar {
        ScalarType::I8 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::I8(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::I8),
        ScalarType::I16 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::I16(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::I16),
        ScalarType::I32 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::I32(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::I32),
        ScalarType::I64 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::I64(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::I64),
        ScalarType::U8 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::U8(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::U8),
        ScalarType::U16 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::U16(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::U16),
        ScalarType::U32 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::U32(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::U32),
        ScalarType::U64 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::U64(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::U64),
        ScalarType::Crc32 => enum_list_values(table, field, row_index, values, scalar, |value| {
            match value {
                CellValue::Crc32(value) => Ok(value),
                _ => unreachable!("enum scalar helper returned wrong cell type"),
            }
        })
        .map(ListValue::Crc32),
        scalar => Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` has enum metadata on unsupported list scalar type {:?}",
            table.name(),
            field.name(),
            scalar
        ))),
    }
    .map(CellValue::List)
    .map(ParsedCell::Value)
}

fn enum_list_values<T>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    values: &[Value],
    scalar: ScalarType,
    take: impl Fn(CellValue<'static>) -> Result<T, GameDataError>,
) -> Result<Vec<T>, GameDataError> {
    values
        .iter()
        .enumerate()
        .map(|(element_index, value)| {
            let Value::String(token) = value else {
                return Err(list_element_type_error(
                    table,
                    field,
                    row_index,
                    element_index,
                    "enum token string",
                    value,
                ));
            };
            let discriminant = enum_discriminant_from_string(table, field, row_index, token)?;
            enum_cell_value_from_discriminant_for_scalar(
                table,
                field,
                row_index,
                scalar,
                discriminant,
            )
            .and_then(&take)
        })
        .collect()
}

fn string_list_element<'a>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &'a Value,
) -> Result<&'a str, GameDataError> {
    if let Value::String(value) = value {
        Ok(value)
    } else {
        Err(list_element_type_error(
            table,
            field,
            row_index,
            element_index,
            "string",
            value,
        ))
    }
}

fn bool_list_element(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &Value,
) -> Result<bool, GameDataError> {
    if let Value::Bool(value) = value {
        Ok(*value)
    } else {
        Err(list_element_type_error(
            table,
            field,
            row_index,
            element_index,
            "bool",
            value,
        ))
    }
}

fn number_list_element<T>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &Value,
    convert: impl FnOnce(Number) -> Result<T, GameDataError>,
) -> Result<T, GameDataError> {
    if let Value::Number(value) = value {
        convert(*value)
    } else {
        Err(list_element_type_error(
            table,
            field,
            row_index,
            element_index,
            "number",
            value,
        ))
    }
}

fn row_index_list_element(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &Value,
) -> Result<RowIndex, GameDataError> {
    let one_based =
        number_list_element(table, field, row_index, element_index, value, number_to_u32)?;
    RowIndex::from_one_based(one_based).ok_or_else(|| {
        GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` list entry {element_index} has invalid one-based row index {one_based}",
            table.name(),
            field.name()
        ))
    })
}

fn crc32_list_element(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &Value,
) -> Result<u32, GameDataError> {
    match value {
        Value::String(value) => Ok(Crc32::from_str_lower(value).value()),
        Value::Number(value) => number_to_u32(*value),
        _ => Err(list_element_type_error(
            table,
            field,
            row_index,
            element_index,
            "string or number",
            value,
        )),
    }
}

fn color_list_element(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &Value,
) -> Result<LinearRgba, GameDataError> {
    if let Value::String(value) = value {
        linear_rgba_from_hex(table, field, row_index, value)
    } else {
        Err(list_element_type_error(
            table,
            field,
            row_index,
            element_index,
            "hex color string",
            value,
        ))
    }
}

fn pair_list_element(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &Value,
    pair_type: PairType,
) -> Result<PairValue<'static>, GameDataError> {
    let first = pair_component_value(table, field, row_index, element_index, value, "first")?;
    let second = pair_component_value(table, field, row_index, element_index, value, "second")?;
    Ok(PairValue::new(
        atom_cell_value(
            table,
            field,
            row_index,
            element_index,
            "first",
            pair_type.first,
            first,
        )?,
        atom_cell_value(
            table,
            field,
            row_index,
            element_index,
            "second",
            pair_type.second,
            second,
        )?,
    ))
}

fn pair_component_value<'a>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    value: &'a Value,
    component: &str,
) -> Result<&'a Value, GameDataError> {
    let Value::Map(map) = value else {
        return Err(list_element_type_error(
            table,
            field,
            row_index,
            element_index,
            "pair map",
            value,
        ));
    };
    map.iter()
        .find_map(|(key, value)| match key {
            Value::String(key) if key == component => Some(value),
            _ => None,
        })
        .ok_or_else(|| {
            GameDataError::Decode(format!(
                "table `{}` row {row_index} field `{}` list entry {element_index} pair is missing `{component}`",
                table.name(),
                field.name()
            ))
        })
}

fn atom_cell_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    component: &str,
    atom_type: AtomType,
    value: &Value,
) -> Result<CellValue<'static>, GameDataError> {
    match atom_type {
        AtomType::Range(RangeType {
            bounds: RangeBounds::Exclusive,
            endpoint: RangeEndpointType::F32,
        }) => range_f32_value(table, field, row_index, value).map(CellValue::RangeF32),
        AtomType::Range(RangeType {
            bounds: RangeBounds::Inclusive,
            endpoint: RangeEndpointType::F32,
        }) => range_inclusive_f32_value(table, field, row_index, value)
            .map(CellValue::RangeInclusiveF32),
        AtomType::Range(RangeType {
            bounds: RangeBounds::Exclusive,
            endpoint: RangeEndpointType::U32,
        }) => range_u32_value(table, field, row_index, value).map(CellValue::RangeU32),
        AtomType::Range(RangeType {
            bounds: RangeBounds::Inclusive,
            endpoint: RangeEndpointType::U32,
        }) => range_inclusive_u32_value(table, field, row_index, value)
            .map(CellValue::RangeInclusiveU32),
        AtomType::Range(RangeType {
            bounds: RangeBounds::Exclusive,
            endpoint: RangeEndpointType::I32,
        }) => range_i32_value(table, field, row_index, value).map(CellValue::RangeI32),
        AtomType::Range(RangeType {
            bounds: RangeBounds::Inclusive,
            endpoint: RangeEndpointType::I32,
        }) => range_inclusive_i32_value(table, field, row_index, value)
            .map(CellValue::RangeInclusiveI32),
        AtomType::Scalar(scalar) => scalar_atom_cell_value(
            table,
            field,
            row_index,
            element_index,
            component,
            scalar,
            value,
        ),
    }
}

/// Expands the numeric pair-component arms, which differ only in the `Number`
/// converter and the `CellValue` constructor that shares its name with the
/// `ScalarType`. The remaining atoms carry their own parsing and are written
/// out after the table.
macro_rules! pair_component_arms {
    (
        ($table:expr, $field:expr, $row_index:expr, $element_index:expr, $component:expr,
         $value:expr, $scalar:expr)
        numbers { $( $variant:ident => $convert:expr; )+ }
        $( $rest:tt )*
    ) => {
        match $scalar {
            $(
                ScalarType::$variant => CellValue::$variant(number_pair_component(
                    $table,
                    $field,
                    $row_index,
                    $element_index,
                    $component,
                    $value,
                    $convert,
                )?),
            )+
            $( $rest )*
        }
    };
}

fn scalar_atom_cell_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    component: &str,
    scalar: ScalarType,
    value: &Value,
) -> Result<CellValue<'static>, GameDataError> {
    Ok(pair_component_arms! {
        (table, field, row_index, element_index, component, value, scalar)
        numbers {
            I8 => number_to_i8;
            I16 => number_to_i16;
            I32 => number_to_i32;
            I64 => number_to_i64;
            U8 => number_to_u8;
            U16 => number_to_u16;
            U32 => number_to_u32;
            U64 => number_to_u64;
            NonZeroI8 => number_to_nonzero_i8;
            NonZeroI16 => number_to_nonzero_i16;
            NonZeroI32 => number_to_nonzero_i32;
            NonZeroI64 => number_to_nonzero_i64;
            NonZeroU8 => number_to_nonzero_u8;
            NonZeroU16 => number_to_nonzero_u16;
            NonZeroU32 => number_to_nonzero_u32;
            NonZeroU64 => number_to_nonzero_u64;
            F32 => |value| Ok(number_to_f32(value));
            F64 => |value| Ok(number_to_f64(value));
        }
        ScalarType::String => CellValue::string(
            string_pair_component(table, field, row_index, element_index, component, value)?
                .to_owned(),
        ),
        ScalarType::RowKey => CellValue::row_key(
            string_pair_component(table, field, row_index, element_index, component, value)?
                .to_owned(),
        ),
        ScalarType::Bool => CellValue::Bool(bool_pair_component(
            table,
            field,
            row_index,
            element_index,
            component,
            value,
        )?),
        ScalarType::LinearRgba => CellValue::LinearRgba(linear_rgba_from_hex(
            table,
            field,
            row_index,
            string_pair_component(table, field, row_index, element_index, component, value)?,
        )?),
        ScalarType::Crc32 => CellValue::Crc32(match value {
            Value::String(value) => Crc32::from_str_lower(value).value(),
            Value::Number(_) => number_pair_component(
                table,
                field,
                row_index,
                element_index,
                component,
                value,
                number_to_u32,
            )?,
            _ => {
                return Err(pair_component_type_error(
                    table,
                    field,
                    row_index,
                    element_index,
                    component,
                    "string or number",
                    value,
                ));
            }
        }),
        ScalarType::RowIndex => {
            let one_based = number_pair_component(
                table,
                field,
                row_index,
                element_index,
                component,
                value,
                number_to_u32,
            )?;
            CellValue::RowIndex(RowIndex::from_one_based(one_based).ok_or_else(|| {
                GameDataError::Decode(format!(
                    "table `{}` row {row_index} field `{}` list entry {element_index} pair component `{component}` has invalid one-based row index {one_based}",
                    table.name(),
                    field.name()
                ))
            })?)
        }
        ScalarType::ForeignKey => {
            return Err(GameDataError::Decode(format!(
                "table `{}` row {row_index} field `{}` list entry {element_index} pair component `{component}` cannot contain a foreign key atom",
                table.name(),
                field.name()
            )));
        }
    })
}

fn string_pair_component<'a>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    component: &str,
    value: &'a Value,
) -> Result<&'a str, GameDataError> {
    if let Value::String(value) = value {
        Ok(value)
    } else {
        Err(pair_component_type_error(
            table,
            field,
            row_index,
            element_index,
            component,
            "string",
            value,
        ))
    }
}

fn bool_pair_component(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    component: &str,
    value: &Value,
) -> Result<bool, GameDataError> {
    if let Value::Bool(value) = value {
        Ok(*value)
    } else {
        Err(pair_component_type_error(
            table,
            field,
            row_index,
            element_index,
            component,
            "bool",
            value,
        ))
    }
}

fn number_pair_component<T>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    component: &str,
    value: &Value,
    convert: impl FnOnce(Number) -> Result<T, GameDataError>,
) -> Result<T, GameDataError> {
    if let Value::Number(value) = value {
        convert(*value)
    } else {
        Err(pair_component_type_error(
            table,
            field,
            row_index,
            element_index,
            component,
            "number",
            value,
        ))
    }
}

fn pair_component_type_error(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    component: &str,
    expected: &str,
    value: &Value,
) -> GameDataError {
    GameDataError::Decode(format!(
        "table `{}` row {row_index} field `{}` list entry {element_index} pair component `{component}` expected {expected}, found {}",
        table.name(),
        field.name(),
        value_kind(value)
    ))
}

fn list_element_type_error(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    element_index: usize,
    expected: &str,
    value: &Value,
) -> GameDataError {
    GameDataError::Decode(format!(
        "table `{}` row {row_index} field `{}` list entry {element_index} expected {expected}, found {}",
        table.name(),
        field.name(),
        value_kind(value)
    ))
}
