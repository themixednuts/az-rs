use az_core::crc::Crc32;
use bevy_color::LinearRgba;
use ron::Value;
use std::borrow::Cow;

use crate::GameDataError;
use crate::identity::RowIndex;
use crate::table::encode::CellValue;
use crate::table::{CellType, RangeBounds, RangeEndpointType, RangeType, ScalarType};

use super::list::list_cell_from_values;
use super::number::{
    number_to_f32, number_to_f64, number_to_i8, number_to_i16, number_to_i32, number_to_i64,
    number_to_nonzero_i8, number_to_nonzero_i16, number_to_nonzero_i32, number_to_nonzero_i64,
    number_to_nonzero_u8, number_to_nonzero_u16, number_to_nonzero_u32, number_to_nonzero_u64,
    number_to_u8, number_to_u16, number_to_u32, number_to_u64,
};
use super::source::ParsedCell;
use crate::descriptor::AuthoringTableSchema as TableSchema;
use crate::table_set::ColumnSchema;

pub(super) fn field_cell_value<'a>(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    row: &[(&'a str, &'a Value)],
) -> Result<Option<ParsedCell>, GameDataError> {
    let Some((_, value)) = row.iter().find(|(name, _)| *name == field.name()) else {
        if field.is_required() {
            return Err(GameDataError::Decode(format!(
                "table `{}` row {row_index} is missing required field `{}`",
                table.name(),
                field.name()
            )));
        }
        return Ok(None);
    };

    match value {
        Value::Option(None) | Value::Unit => Ok(None),
        Value::Option(Some(value)) => field_cell_value_from_value(table, field, row_index, value),
        value => field_cell_value_from_value(table, field, row_index, value),
    }
}

pub(super) fn row_fields<'a>(
    table: &dyn TableSchema,
    row_index: usize,
    value: &'a Value,
) -> Result<Vec<(&'a str, &'a Value)>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} must be a RON row map, found {}",
            table.name(),
            value_kind(value)
        )));
    };

    let mut fields = Vec::with_capacity(map.len());
    for (key, value) in map.iter() {
        let Value::String(name) = key else {
            return Err(GameDataError::Decode(format!(
                "table `{}` row {row_index} has non-string field key {}",
                table.name(),
                value_kind(key)
            )));
        };
        fields.push((name.as_str(), value));
    }
    Ok(fields)
}

/// Expands the plain numeric cell arms, which differ only in the `Number`
/// converter and the `CellValue` constructor that shares its name with the
/// `ScalarType`. Every other cell type carries its own parsing and is written
/// out after the table; the compiler still checks the match is exhaustive.
macro_rules! field_cell_arms {
    (
        ($cell_type:expr, $value:expr)
        numbers { $( $variant:ident => $convert:expr; )+ }
        $( $rest:tt )*
    ) => {
        match ($cell_type, $value) {
            $(
                (CellType::Scalar(ScalarType::$variant), Value::Number(number)) => {
                    ParsedCell::Value(CellValue::$variant($convert(*number)?))
                }
            )+
            $( $rest )*
        }
    };
}

pub(super) fn field_cell_value_from_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<Option<ParsedCell>, GameDataError> {
    let cell = field_cell_arms! {
        (field.cell_type(), value)
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
        }
        (_, Value::String(value))
            if !field.enum_variants().is_empty()
                && !matches!(field.cell_type(), CellType::Scalar(ScalarType::String)) =>
        {
            enum_cell_value_from_string(table, field, row_index, value)?
        }
        (CellType::Scalar(ScalarType::String), Value::String(value)) => {
            ParsedCell::Value(CellValue::string(value.clone()))
        }
        (CellType::Scalar(ScalarType::RowKey), Value::String(value)) => {
            ParsedCell::Value(CellValue::row_key(value.clone()))
        }
        (CellType::Scalar(ScalarType::ForeignKey), Value::String(value)) => {
            ParsedCell::ForeignKey(value.clone())
        }
        (CellType::List(_), Value::Seq(values)) => {
            list_cell_from_values(table, field, row_index, values)?
        }
        (CellType::Scalar(ScalarType::Bool), Value::Bool(value)) => {
            ParsedCell::Value(CellValue::Bool(*value))
        }
        (CellType::Scalar(ScalarType::F32), Value::Number(value)) => {
            ParsedCell::Value(CellValue::F32(number_to_f32(*value)))
        }
        (CellType::Scalar(ScalarType::F64), Value::Number(value)) => {
            ParsedCell::Value(CellValue::F64(number_to_f64(*value)))
        }
        (CellType::Scalar(ScalarType::LinearRgba), Value::String(value)) => {
            if value.trim().is_empty() && !field.is_required() {
                return Ok(None);
            }
            ParsedCell::Value(CellValue::LinearRgba(linear_rgba_from_hex(
                table, field, row_index, value,
            )?))
        }
        (CellType::Range(range_type), value) => {
            ParsedCell::Value(range_cell_value(table, field, row_index, range_type, value)?)
        }
        (CellType::Scalar(ScalarType::Crc32), Value::Number(value)) => {
            ParsedCell::Value(CellValue::Crc32(number_to_u32(*value)?))
        }
        (CellType::Scalar(ScalarType::Crc32), Value::String(value)) => {
            ParsedCell::Value(CellValue::Crc32(Crc32::from_str_lower(value).value()))
        }
        (CellType::Scalar(ScalarType::RowIndex), Value::Number(value)) => {
            let one_based = number_to_u32(*value)?;
            ParsedCell::Value(CellValue::RowIndex(
                RowIndex::from_one_based(one_based).ok_or_else(|| {
                    GameDataError::Decode(format!(
                        "table `{}` row {row_index} field `{}` has invalid one-based row index {one_based}",
                        table.name(),
                        field.name()
                    ))
                })?,
            ))
        }
        _ => {
            return Err(GameDataError::Decode(format!(
                "table `{}` row {row_index} field `{}` expected {:?}, found {}",
                table.name(),
                field.name(),
                field.cell_type(),
                value_kind(value)
            )));
        }
    };
    Ok(Some(cell))
}

fn range_cell_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    range_type: RangeType,
    value: &Value,
) -> Result<CellValue<'static>, GameDataError> {
    match (range_type.bounds, range_type.endpoint) {
        (RangeBounds::Exclusive, RangeEndpointType::F32) => Ok(CellValue::RangeF32(
            range_f32_value(table, field, row_index, value)?,
        )),
        (RangeBounds::Inclusive, RangeEndpointType::F32) => Ok(CellValue::RangeInclusiveF32(
            range_inclusive_f32_value(table, field, row_index, value)?,
        )),
        (RangeBounds::Exclusive, RangeEndpointType::U32) => Ok(CellValue::RangeU32(
            range_u32_value(table, field, row_index, value)?,
        )),
        (RangeBounds::Inclusive, RangeEndpointType::U32) => Ok(CellValue::RangeInclusiveU32(
            range_inclusive_u32_value(table, field, row_index, value)?,
        )),
        (RangeBounds::Exclusive, RangeEndpointType::I32) => Ok(CellValue::RangeI32(
            range_i32_value(table, field, row_index, value)?,
        )),
        (RangeBounds::Inclusive, RangeEndpointType::I32) => Ok(CellValue::RangeInclusiveI32(
            range_inclusive_i32_value(table, field, row_index, value)?,
        )),
    }
}

pub(super) fn linear_rgba_from_hex(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &str,
) -> Result<LinearRgba, GameDataError> {
    let value = value.trim();
    let hex = value.strip_prefix('#').ok_or_else(|| {
        color_error(
            table,
            field,
            row_index,
            value,
            "expected `#RRGGBB` or `#RRGGBBAA`",
        )
    })?;

    let alpha = match hex.len() {
        6 => 255,
        8 => hex_byte(table, field, row_index, value, hex, 6)?,
        _ => {
            return Err(color_error(
                table,
                field,
                row_index,
                value,
                "expected six or eight hex digits",
            ));
        }
    };

    let red = hex_byte(table, field, row_index, value, hex, 0)?;
    let green = hex_byte(table, field, row_index, value, hex, 2)?;
    let blue = hex_byte(table, field, row_index, value, hex, 4)?;
    let channel = |component| f32::from(component) / 255.0;
    Ok(LinearRgba::new(
        channel(red),
        channel(green),
        channel(blue),
        channel(alpha),
    ))
}

fn hex_byte(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &str,
    hex: &str,
    start: usize,
) -> Result<u8, GameDataError> {
    let end = start + 2;
    let Some(component) = hex.get(start..end) else {
        return Err(color_error(
            table,
            field,
            row_index,
            value,
            "hex component is truncated",
        ));
    };
    u8::from_str_radix(component, 16).map_err(|_| {
        color_error(
            table,
            field,
            row_index,
            value,
            "hex component contains a non-hex digit",
        )
    })
}

fn color_error(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &str,
    reason: &str,
) -> GameDataError {
    GameDataError::Decode(format!(
        "table `{}` row {row_index} field `{}` expected linear RGBA hex color, found `{value}`: {reason}",
        table.name(),
        field.name()
    ))
}

fn enum_cell_value_from_string(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &str,
) -> Result<ParsedCell, GameDataError> {
    let discriminant = enum_discriminant_from_string(table, field, row_index, value)?;
    enum_cell_value_from_discriminant(table, field, row_index, discriminant).map(ParsedCell::Value)
}

pub(super) fn enum_discriminant_from_string(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &str,
) -> Result<i64, GameDataError> {
    let token = value.trim();
    field
        .enum_variants()
        .iter()
        .copied()
        .find(|variant| variant.matches(token))
        .map(super::super::table_set::EnumVariantMeta::discriminant)
        .ok_or_else(|| {
            GameDataError::Decode(format!(
                "table `{}` row {row_index} field `{}` expected enum token, found `{value}`",
                table.name(),
                field.name()
            ))
        })
}

fn enum_cell_value_from_discriminant(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    discriminant: i64,
) -> Result<CellValue<'static>, GameDataError> {
    let CellType::Scalar(scalar) = field.cell_type() else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` has enum metadata on non-enum cell type {:?}",
            table.name(),
            field.name(),
            field.cell_type()
        )));
    };
    enum_cell_value_from_discriminant_for_scalar(table, field, row_index, scalar, discriminant)
}

pub(super) fn enum_cell_value_from_discriminant_for_scalar(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    scalar: ScalarType,
    discriminant: i64,
) -> Result<CellValue<'static>, GameDataError> {
    let value = match scalar {
        ScalarType::I8 => CellValue::I8(
            i8::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::I16 => CellValue::I16(
            i16::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::I32 => CellValue::I32(
            i32::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::I64 => CellValue::I64(discriminant),
        ScalarType::U8 => CellValue::U8(
            u8::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::U16 => CellValue::U16(
            u16::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::U32 => CellValue::U32(
            u32::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::U64 => CellValue::U64(
            u64::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        ScalarType::Crc32 => CellValue::Crc32(
            u32::try_from(discriminant)
                .map_err(|_| enum_discriminant_error(table, field, row_index, discriminant))?,
        ),
        cell_type => {
            return Err(GameDataError::Decode(format!(
                "table `{}` row {row_index} field `{}` has enum metadata on unsupported scalar type {:?}",
                table.name(),
                field.name(),
                cell_type
            )));
        }
    };
    Ok(value)
}

fn enum_discriminant_error(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    discriminant: i64,
) -> GameDataError {
    GameDataError::Decode(format!(
        "table `{}` row {row_index} field `{}` enum discriminant {discriminant} does not fit {:?}",
        table.name(),
        field.name(),
        field.cell_type()
    ))
}

pub(super) fn row_identity(
    table: &dyn TableSchema,
    row_index: usize,
    row_cells: &[Option<ParsedCell>],
) -> Result<(u32, Option<Cow<'static, str>>), GameDataError> {
    let Some((field_index, field)) = table
        .columns()
        .enumerate()
        .find(|(_, field)| field.is_row_key())
    else {
        return Ok((0, None));
    };
    let Some(cell) = row_cells.get(field_index).and_then(Option::as_ref) else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} has no value for row-key field `{}`",
            table.name(),
            field.name()
        )));
    };

    let ParsedCell::Value(cell) = cell else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` cannot use unresolved foreign-key source value as a row key",
            table.name(),
            field.name()
        )));
    };

    Ok(match cell {
        CellValue::String(value) | CellValue::RowKey(value) => {
            (Crc32::from_str_lower(value).value(), Some(value.clone()))
        }
        CellValue::U32(value) | CellValue::Crc32(value) => (*value, None),
        CellValue::I8(value) => (u32::from_ne_bytes(i32::from(*value).to_ne_bytes()), None),
        CellValue::I16(value) => (u32::from_ne_bytes(i32::from(*value).to_ne_bytes()), None),
        CellValue::I32(value) => (u32::from_ne_bytes(value.to_ne_bytes()), None),
        CellValue::I64(value) => (
            u32::try_from(*value).map_err(|_| row_key_error(table, row_index, field))?,
            None,
        ),
        CellValue::U8(value) => (u32::from(*value), None),
        CellValue::U16(value) => (u32::from(*value), None),
        CellValue::U64(value) => (
            u32::try_from(*value).map_err(|_| row_key_error(table, row_index, field))?,
            None,
        ),
        CellValue::NonZeroI8(value) => (
            u32::from_ne_bytes(i32::from(value.get()).to_ne_bytes()),
            None,
        ),
        CellValue::NonZeroI16(value) => (
            u32::from_ne_bytes(i32::from(value.get()).to_ne_bytes()),
            None,
        ),
        CellValue::NonZeroI32(value) => (u32::from_ne_bytes(value.get().to_ne_bytes()), None),
        CellValue::NonZeroI64(value) => (
            u32::try_from(value.get()).map_err(|_| row_key_error(table, row_index, field))?,
            None,
        ),
        CellValue::NonZeroU8(value) => (u32::from(value.get()), None),
        CellValue::NonZeroU16(value) => (u32::from(value.get()), None),
        CellValue::NonZeroU32(value) => (value.get(), None),
        CellValue::NonZeroU64(value) => (
            u32::try_from(value.get()).map_err(|_| row_key_error(table, row_index, field))?,
            None,
        ),
        CellValue::Bool(_)
        | CellValue::F32(_)
        | CellValue::F64(_)
        | CellValue::LinearRgba(_)
        | CellValue::RangeF32(_)
        | CellValue::RangeInclusiveF32(_)
        | CellValue::RangeU32(_)
        | CellValue::RangeInclusiveU32(_)
        | CellValue::RangeI32(_)
        | CellValue::RangeInclusiveI32(_)
        | CellValue::RowIndex(_)
        | CellValue::ForeignKey(_)
        | CellValue::List(_) => {
            return Err(GameDataError::Decode(format!(
                "table `{}` row {row_index} field `{}` cannot use {:?} as a row key",
                table.name(),
                field.name(),
                cell
            )));
        }
    })
}

pub(super) fn range_f32_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<::core::range::Range<f32>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(range_value_kind_error(table, field, row_index, value));
    };
    Ok(::core::range::Range {
        start: range_component_f32(table, field, row_index, map, "start")?,
        end: range_component_f32(table, field, row_index, map, "end")?,
    })
}

pub(super) fn range_inclusive_f32_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<::core::range::RangeInclusive<f32>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(range_value_kind_error(table, field, row_index, value));
    };
    Ok(::core::range::RangeInclusive {
        start: range_component_f32(table, field, row_index, map, "start")?,
        last: range_component_f32(table, field, row_index, map, "last")?,
    })
}

pub(super) fn range_u32_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<::core::range::Range<u32>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(range_value_kind_error(table, field, row_index, value));
    };
    Ok(::core::range::Range {
        start: range_component_u32(table, field, row_index, map, "start")?,
        end: range_component_u32(table, field, row_index, map, "end")?,
    })
}

pub(super) fn range_inclusive_u32_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<::core::range::RangeInclusive<u32>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(range_value_kind_error(table, field, row_index, value));
    };
    Ok(::core::range::RangeInclusive {
        start: range_component_u32(table, field, row_index, map, "start")?,
        last: range_component_u32(table, field, row_index, map, "last")?,
    })
}

pub(super) fn range_i32_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<::core::range::Range<i32>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(range_value_kind_error(table, field, row_index, value));
    };
    Ok(::core::range::Range {
        start: range_component_i32(table, field, row_index, map, "start")?,
        end: range_component_i32(table, field, row_index, map, "end")?,
    })
}

pub(super) fn range_inclusive_i32_value(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> Result<::core::range::RangeInclusive<i32>, GameDataError> {
    let Value::Map(map) = value else {
        return Err(range_value_kind_error(table, field, row_index, value));
    };
    Ok(::core::range::RangeInclusive {
        start: range_component_i32(table, field, row_index, map, "start")?,
        last: range_component_i32(table, field, row_index, map, "last")?,
    })
}

fn range_component_f32(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    map: &ron::value::Map,
    component: &str,
) -> Result<f32, GameDataError> {
    Ok(number_to_f32(range_component_number(
        table, field, row_index, map, component,
    )?))
}

fn range_component_u32(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    map: &ron::value::Map,
    component: &str,
) -> Result<u32, GameDataError> {
    number_to_u32(range_component_number(
        table, field, row_index, map, component,
    )?)
}

fn range_component_i32(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    map: &ron::value::Map,
    component: &str,
) -> Result<i32, GameDataError> {
    number_to_i32(range_component_number(
        table, field, row_index, map, component,
    )?)
}

fn range_component_number(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    map: &ron::value::Map,
    component: &str,
) -> Result<ron::value::Number, GameDataError> {
    let Some((_, value)) = map
        .iter()
        .find(|(key, _)| matches!(key, Value::String(name) if name == component))
    else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` range is missing `{component}`",
            table.name(),
            field.name()
        )));
    };
    let Value::Number(value) = value else {
        return Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` range component `{component}` expected number, found {}",
            table.name(),
            field.name(),
            value_kind(value)
        )));
    };
    Ok(*value)
}

fn range_value_kind_error(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    value: &Value,
) -> GameDataError {
    GameDataError::Decode(format!(
        "table `{}` row {row_index} field `{}` expected range map, found {}",
        table.name(),
        field.name(),
        value_kind(value)
    ))
}

fn row_key_error(table: &dyn TableSchema, row_index: usize, field: ColumnSchema) -> GameDataError {
    GameDataError::Decode(format!(
        "table `{}` row {row_index} field `{}` row key does not fit u32",
        table.name(),
        field.name()
    ))
}

pub(super) const fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "bool",
        Value::Char(_) => "char",
        Value::Map(_) => "map",
        Value::Number(_) => "number",
        Value::Option(_) => "option",
        Value::String(_) => "string",
        Value::Bytes(_) => "bytes",
        Value::Seq(_) => "sequence",
        Value::Unit => "unit",
    }
}
