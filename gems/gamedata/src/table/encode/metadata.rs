use std::collections::HashMap;

use bytes::Buf;
#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;
use uuid::Uuid;

use crate::GameDataError;
use crate::identity::{RowGuid, RowIndex};
use crate::release::SchemaHash;
use crate::table::body::{TableDependency, TextRef};
#[cfg(any(feature = "authoring", test))]
use crate::table::encode::EncodeRow;
#[cfg(any(feature = "authoring", test))]
use crate::table::encode::pool::StringPoolBuilder;
use crate::table::encode::scalar::{read_u8, read_u32, read_u64};
#[cfg(any(feature = "authoring", test))]
use crate::table::encode::text::write_string_ref;
use crate::table::encode::text::{read_pooled_string_ref, read_text_ref};

const SCHEMA_SECTION_LEN: usize = 24;

/// Deterministic row GUID for externally imported rows.
#[cfg(any(feature = "authoring", test))]
#[must_use]
pub fn import_row_guid(table_name_crc: u32, row_type_crc: u32, row_key_crc: u32) -> RowGuid {
    let namespace = Uuid::from_u128(
        0x415a_5442_4c01_0000u128 | (u128::from(table_name_crc) << 32) | u128::from(row_type_crc),
    );
    RowGuid::from_uuid(Uuid::new_v5(&namespace, &row_key_crc.to_le_bytes()))
}

/// Deterministic row GUID for externally imported rows with an authored row name.
#[cfg(any(feature = "authoring", test))]
#[must_use]
pub fn import_row_guid_with_name(
    table_name_crc: u32,
    row_type_crc: u32,
    row_name: &str,
) -> RowGuid {
    let namespace = Uuid::from_u128(
        0x415a_5442_4c01_0000u128 | (u128::from(table_name_crc) << 32) | u128::from(row_type_crc),
    );
    let mut name = Vec::with_capacity("row-name:".len() + row_name.len());
    name.extend_from_slice(b"row-name:");
    name.extend_from_slice(row_name.as_bytes());
    RowGuid::from_uuid(Uuid::new_v5(&namespace, &name))
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn encode_schema_section(
    schema_hash: SchemaHash,
    table_name_crc: u32,
    row_type_crc: u32,
    column_count: u32,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(SCHEMA_SECTION_LEN);
    bytes.put_u64_le(schema_hash.0);
    bytes.put_u32_le(table_name_crc);
    bytes.put_u32_le(row_type_crc);
    bytes.put_u32_le(column_count);
    bytes.put_u32_le(0);
    bytes
}

pub(super) fn decode_schema_section(
    payload: &[u8],
) -> Result<(SchemaHash, u32, u32, u32), GameDataError> {
    if payload.len() < SCHEMA_SECTION_LEN {
        return Err(GameDataError::Decode(format!(
            "SCHEMA section too short: {} bytes (need {SCHEMA_SECTION_LEN})",
            payload.len()
        )));
    }
    let mut data = payload;
    Ok((
        SchemaHash(read_u64(&mut data, "schema.schema_hash")?),
        read_u32(&mut data, "schema.table_name_crc")?,
        read_u32(&mut data, "schema.row_type_crc")?,
        read_u32(&mut data, "schema.column_count")?,
    ))
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn encode_row_guids_section(rows: &[EncodeRow<'_>]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rows.len() * 16);
    for row in rows {
        bytes.extend_from_slice(row.row_guid.as_uuid().as_bytes());
    }
    bytes
}

pub(super) fn decode_row_guids_section(
    payload: &[u8],
    row_count: u32,
) -> Result<Vec<RowGuid>, GameDataError> {
    let expected = usize::try_from(row_count)
        .map_err(|_| GameDataError::Decode("row_count exceeds usize".into()))?
        .saturating_mul(16);
    if payload.len() != expected {
        return Err(GameDataError::Decode(format!(
            "ROW_GUIDS section length {} does not match row_count {row_count} (expected {expected})",
            payload.len()
        )));
    }
    let mut guids = Vec::with_capacity(row_count as usize);
    for index in 0..row_count as usize {
        let start = index * 16;
        let end = start + 16;
        let slice = payload.get(start..end).expect("bounds checked");
        guids.push(RowGuid::from_uuid(Uuid::from_bytes(
            slice.try_into().expect("slice length"),
        )));
    }
    Ok(guids)
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn encode_row_key_aliases_section(
    rows: &[EncodeRow<'_>],
) -> Result<Vec<u8>, GameDataError> {
    let mut alias_counts = HashMap::<u32, usize>::new();
    for row in rows.iter().filter(|row| row.key_crc != 0) {
        *alias_counts.entry(row.key_crc).or_default() += 1;
    }
    let alias_count = rows
        .iter()
        .filter(|row| row.key_crc != 0 && alias_counts.get(&row.key_crc) == Some(&1))
        .count();

    let mut bytes = Vec::new();
    bytes.put_u32_le(
        u32::try_from(alias_count)
            .map_err(|_| GameDataError::Decode("alias count exceeds u32".into()))?,
    );
    for (row_index, row) in rows.iter().enumerate() {
        if row.key_crc == 0 || alias_counts.get(&row.key_crc) != Some(&1) {
            continue;
        }
        bytes.put_u32_le(row.key_crc);
        bytes.put_u32_le(u32::try_from(row_index + 1).expect("row index fits in u32"));
    }
    Ok(bytes)
}

pub(super) fn decode_row_key_aliases_section(
    payload: &[u8],
) -> Result<HashMap<u32, RowIndex>, GameDataError> {
    if payload.len() < 4 {
        return Err(GameDataError::Decode(
            "ROW_KEY_ALIASES section too short".into(),
        ));
    }
    let mut data = payload;
    let count = read_u32(&mut data, "aliases.count")?;
    let expected_len = 4usize
        .checked_add(
            usize::try_from(count)
                .map_err(|_| GameDataError::Decode("alias count exceeds usize".into()))?
                .checked_mul(8)
                .ok_or_else(|| GameDataError::Decode("alias section length overflows".into()))?,
        )
        .ok_or_else(|| GameDataError::Decode("alias section length overflows".into()))?;
    if payload.len() != expected_len {
        return Err(GameDataError::Decode(format!(
            "ROW_KEY_ALIASES section length {} does not match count {count}",
            payload.len()
        )));
    }
    let mut aliases = HashMap::new();
    for _ in 0..count {
        let key_crc = read_u32(&mut data, "aliases.key_crc")?;
        let row_index = read_u32(&mut data, "aliases.row_index")?;
        let row_index = RowIndex::from_one_based(row_index).ok_or_else(|| {
            GameDataError::Decode(format!("invalid RowIndex one-based value {row_index}"))
        })?;
        if aliases.insert(key_crc, row_index).is_some() {
            return Err(GameDataError::Decode(format!(
                "duplicate row key alias crc {key_crc}"
            )));
        }
    }
    if data.remaining() != 0 {
        return Err(GameDataError::Decode(format!(
            "ROW_KEY_ALIASES section has {} trailing byte(s)",
            data.remaining()
        )));
    }
    Ok(aliases)
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn encode_dependency_index_section(dependencies: &[TableDependency]) -> Option<Vec<u8>> {
    if dependencies.is_empty() {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.put_u32_le(u32::try_from(dependencies.len()).expect("dependency count fits in u32"));
    for dependency in dependencies {
        bytes.put_u32_le(dependency.column_crc);
        bytes.put_u32_le(dependency.target_table_name_crc);
        bytes.put_u64_le(dependency.target_schema_hash.0);
        bytes.put_u32_le(dependency.kind);
    }
    Some(bytes)
}

pub(super) fn decode_dependency_index_section(
    payload: &[u8],
) -> Result<Vec<TableDependency>, GameDataError> {
    let mut data = payload;
    let count = read_u32(&mut data, "dependencies.count")? as usize;
    let mut dependencies = Vec::with_capacity(count);
    for index in 0..count {
        dependencies.push(TableDependency {
            column_crc: read_u32(&mut data, &format!("dependencies[{index}].column_crc"))?,
            target_table_name_crc: read_u32(
                &mut data,
                &format!("dependencies[{index}].target_table_name_crc"),
            )?,
            target_schema_hash: SchemaHash(read_u64(
                &mut data,
                &format!("dependencies[{index}].target_schema_hash"),
            )?),
            kind: read_u32(&mut data, &format!("dependencies[{index}].kind"))?,
        });
    }
    if data.remaining() != 0 {
        return Err(GameDataError::Decode(format!(
            "DEPENDENCY_INDEX section has {} trailing byte(s)",
            data.remaining()
        )));
    }
    Ok(dependencies)
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn encode_debug_names_section(
    rows: &[EncodeRow<'_>],
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Option<Vec<u8>> {
    if rows
        .iter()
        .all(|row| row.debug_name.as_ref().is_none_or(|name| name.is_empty()))
    {
        return None;
    }
    let mut bytes = Vec::new();
    for row in rows {
        match row.debug_name.as_deref().filter(|name| !name.is_empty()) {
            Some(name) => {
                bytes.put_u8(1);
                if use_string_pool {
                    let (offset, len) = string_pool.offsets(name).expect("debug name interned");
                    write_string_ref(&mut bytes, offset, len);
                } else {
                    let len = u32::try_from(name.len()).expect("debug name length fits in u32");
                    bytes.put_u32_le(len);
                    bytes.put_slice(name.as_bytes());
                }
            }
            None => bytes.put_u8(0),
        }
    }
    Some(bytes)
}

pub(super) fn decode_debug_names_section(
    bytes: &[u8],
    payload: &[u8],
    row_count: u32,
    pool_base: Option<u32>,
) -> Result<Vec<Option<TextRef>>, GameDataError> {
    let mut data = payload;
    let mut names = Vec::with_capacity(row_count as usize);
    for row_index in 0..row_count {
        let present = read_u8(&mut data, &format!("debug_names[{row_index}].present"))?;
        if present == 0 {
            names.push(None);
            continue;
        }
        if present != 1 {
            return Err(GameDataError::Decode(format!(
                "debug_names[{row_index}] expected present flag 0 or 1, got {present}"
            )));
        }
        let field = format!("debug_names[{row_index}].string");
        names.push(Some(if let Some(pool_base) = pool_base {
            read_pooled_string_ref(bytes, &mut data, pool_base, &field)?
        } else {
            read_text_ref(bytes, &mut data, &field)?
        }));
    }
    if data.remaining() != 0 {
        return Err(GameDataError::Decode(format!(
            "DEBUG_NAMES section has {} trailing byte(s)",
            data.remaining()
        )));
    }
    Ok(names)
}
