#[cfg(any(feature = "authoring", test))]
use std::collections::{BTreeMap, BTreeSet};

#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;

#[cfg(any(feature = "authoring", test))]
use crate::GameDataError;
#[cfg(any(feature = "authoring", test))]
use crate::table::body::{CellValue, ListValue};
#[cfg(any(feature = "authoring", test))]
use crate::table::encode::EncodeInput;

pub(super) const STRING_POOL_HEADER_LEN: u32 = 4;

#[cfg(any(feature = "authoring", test))]
pub(super) struct StringPoolBuilder {
    entries: BTreeMap<String, (u32, u32)>,
    blob: Vec<u8>,
}

#[cfg(any(feature = "authoring", test))]
impl StringPoolBuilder {
    pub(super) const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            blob: Vec::new(),
        }
    }

    pub(super) const fn is_empty(&self) -> bool {
        self.blob.is_empty()
    }

    pub(super) fn intern(&mut self, value: &str) -> Result<(u32, u32), GameDataError> {
        if let Some(offsets) = self.entries.get(value) {
            return Ok(*offsets);
        }
        let offset = u32::try_from(self.blob.len())
            .map_err(|_| GameDataError::Decode("string pool blob offset exceeds u32".into()))?;
        let len = u32::try_from(value.len())
            .map_err(|_| GameDataError::Decode("string length exceeds u32".into()))?;
        self.blob.extend_from_slice(value.as_bytes());
        self.entries.insert(value.to_owned(), (offset, len));
        Ok((offset, len))
    }

    pub(super) fn offsets(&self, value: &str) -> Result<(u32, u32), GameDataError> {
        self.entries.get(value).copied().ok_or_else(|| {
            GameDataError::Decode(format!("string `{value}` missing from string pool"))
        })
    }

    pub(super) fn finish(self) -> Option<Vec<u8>> {
        if self.blob.is_empty() {
            return None;
        }
        let mut payload = Vec::with_capacity(STRING_POOL_HEADER_LEN as usize + self.blob.len());
        payload.put_u32_le(
            u32::try_from(self.blob.len())
                .expect("string pool blob length fits in u32 after intern checks"),
        );
        payload.extend(self.blob);
        Some(payload)
    }
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn collect_string_pool<'input>(
    input: &'input EncodeInput<'_>,
) -> Result<StringPoolBuilder, GameDataError> {
    let mut pool = StringPoolBuilder::new();
    let mut pending: BTreeSet<&'input str> = BTreeSet::new();
    for column_cells in &input.cells {
        for cell in column_cells.iter().flatten() {
            collect_cell_strings(cell, &mut pending);
        }
    }
    for row in &input.rows {
        if let Some(name) = row.debug_name.as_deref().filter(|name| !name.is_empty()) {
            pending.insert(name);
        }
    }
    for value in pending {
        pool.intern(value)?;
    }
    Ok(pool)
}

#[cfg(any(feature = "authoring", test))]
fn collect_cell_strings<'input>(cell: &'input CellValue<'_>, pending: &mut BTreeSet<&'input str>) {
    match cell {
        CellValue::String(value) | CellValue::RowKey(value) => {
            pending.insert(value.as_ref());
        }
        CellValue::List(value) => {
            collect_list_strings(value, pending);
        }
        CellValue::Bool(_)
        | CellValue::I8(_)
        | CellValue::I16(_)
        | CellValue::I32(_)
        | CellValue::I64(_)
        | CellValue::U8(_)
        | CellValue::U16(_)
        | CellValue::U32(_)
        | CellValue::U64(_)
        | CellValue::NonZeroI8(_)
        | CellValue::NonZeroI16(_)
        | CellValue::NonZeroI32(_)
        | CellValue::NonZeroI64(_)
        | CellValue::NonZeroU8(_)
        | CellValue::NonZeroU16(_)
        | CellValue::NonZeroU32(_)
        | CellValue::NonZeroU64(_)
        | CellValue::F32(_)
        | CellValue::F64(_)
        | CellValue::LinearRgba(_)
        | CellValue::RangeF32(_)
        | CellValue::RangeInclusiveF32(_)
        | CellValue::RangeU32(_)
        | CellValue::RangeInclusiveU32(_)
        | CellValue::RangeI32(_)
        | CellValue::RangeInclusiveI32(_)
        | CellValue::Crc32(_)
        | CellValue::RowIndex(_)
        | CellValue::ForeignKey(_) => {}
    }
}

#[cfg(any(feature = "authoring", test))]
fn collect_list_strings<'input>(value: &'input ListValue<'_>, pending: &mut BTreeSet<&'input str>) {
    match value {
        ListValue::Strings(values) | ListValue::RowKeys(values) => {
            pending.extend(values.iter().map(std::convert::AsRef::as_ref));
        }
        ListValue::Pairs { values, .. } => {
            for value in values {
                collect_cell_strings(value.first(), pending);
                collect_cell_strings(value.second(), pending);
            }
        }
        ListValue::ForeignKeys(_)
        | ListValue::Bools(_)
        | ListValue::I8(_)
        | ListValue::I16(_)
        | ListValue::I32(_)
        | ListValue::I64(_)
        | ListValue::U8(_)
        | ListValue::U16(_)
        | ListValue::U32(_)
        | ListValue::U64(_)
        | ListValue::NonZeroI8(_)
        | ListValue::NonZeroI16(_)
        | ListValue::NonZeroI32(_)
        | ListValue::NonZeroI64(_)
        | ListValue::NonZeroU8(_)
        | ListValue::NonZeroU16(_)
        | ListValue::NonZeroU32(_)
        | ListValue::NonZeroU64(_)
        | ListValue::F32(_)
        | ListValue::F64(_)
        | ListValue::LinearRgba(_)
        | ListValue::RangeF32(_)
        | ListValue::RangeInclusiveF32(_)
        | ListValue::RangeU32(_)
        | ListValue::RangeInclusiveU32(_)
        | ListValue::RangeI32(_)
        | ListValue::RangeInclusiveI32(_)
        | ListValue::Crc32(_)
        | ListValue::RowIndexes(_) => {}
    }
}
