//! Optional lookup tables that pretty-print element names + fields.
//!
//! `ObjectStream` payloads carry only UUIDs (type) and CRC-32 hashes
//! (field name). Resolving them back to human-readable strings
//! requires the AZ source-tree hash dump (`AZ_CRC` / `AZ_TYPE_INFO`
//! macros parsed out of Lumberyard sources).
//!
//! Callers that don't have the dump pass `None` to
//! [`crate::ObjectStream::from_reader`] and the read still works — names just stay
//! empty / unresolved.
//!
//! # Why `ArcStr`?
//!
//! `ObjectStream` payloads have ~500 unique type names and ~1000
//! unique field names duplicated across millions of elements.
//! Storing them as [`arcstr::ArcStr`] in this lookup means every
//! [`Element`](crate::Element) clone is an atomic refcount bump
//! instead of a heap allocation + memcpy — measured ~10× memory
//! reduction on a 1 M-element parse.

use std::collections::HashMap;

use arcstr::ArcStr;
use az_core::rtti::TypeEntry;
use az_core::uuid::type_ids;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// ObjectStream-specific type names and legacy wire aliases.
///
/// These are not Rust mappings. They are parser-facing names for legacy XML or
/// binary `ObjectStream` ids that either predate the canonical folded id or use a
/// friendlier stream name than the generic AZ type formula.
pub const OBJECT_STREAM_TYPES: &[TypeEntry] = &[
    TypeEntry::alias(
        "AZStd::string",
        type_ids::AZSTD_STRING_LEGACY_XML,
        type_ids::AZSTD_STRING,
    ),
    TypeEntry::alias(
        "AZStd::vector",
        type_ids::AZSTD_VECTOR_LEGACY_XML,
        type_ids::AZSTD_VECTOR,
    ),
    TypeEntry::new("ByteStream", type_ids::BYTE_STREAM),
    TypeEntry::new("SliceComponent", crate::types::SLICE_COMPONENT),
];

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LumberyardHashes {
    /// `AZ_TYPE_INFO` UUID → C++ class name.
    pub uuids: HashMap<Uuid, ArcStr>,
    /// `AZ_CRC` field-name CRC-32 → original field-name string.
    pub crcs: HashMap<u32, ArcStr>,
}

impl LumberyardHashes {
    /// Construct an empty table. Equivalent to [`Default::default`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-allocate space for `uuid_capacity` UUID entries and
    /// `crc_capacity` CRC entries. Useful when building from a
    /// known-size dump.
    #[inline]
    #[must_use]
    pub fn with_capacity(uuid_capacity: usize, crc_capacity: usize) -> Self {
        Self {
            uuids: HashMap::with_capacity(uuid_capacity),
            crcs: HashMap::with_capacity(crc_capacity),
        }
    }

    #[inline]
    #[must_use]
    pub fn with_uuids(mut self, uuids: HashMap<Uuid, ArcStr>) -> Self {
        self.uuids = uuids;
        self
    }

    #[inline]
    #[must_use]
    pub fn with_crcs(mut self, crcs: HashMap<u32, ArcStr>) -> Self {
        self.crcs = crcs;
        self
    }

    /// Insert or replace one type-name mapping.
    #[inline]
    pub fn insert_type_name(&mut self, uuid: Uuid, name: impl AsRef<str>) -> Option<ArcStr> {
        self.uuids.insert(uuid, ArcStr::from(name.as_ref()))
    }

    /// Insert or replace one static AZ type entry.
    #[inline]
    pub fn insert_type(&mut self, entry: TypeEntry) -> Option<ArcStr> {
        self.insert_type_name(entry.type_id(), entry.name())
    }

    /// Add fallback type-name mappings without replacing names already loaded
    /// from a runtime reflection dump.
    pub fn extend_type_names<'a>(&mut self, type_names: impl IntoIterator<Item = (Uuid, &'a str)>) {
        for (uuid, name) in type_names {
            self.uuids.entry(uuid).or_insert_with(|| ArcStr::from(name));
        }
    }

    /// Add fallback AZ type entries without replacing names already loaded from
    /// a runtime reflection dump.
    pub fn extend_types<'a>(&mut self, type_entries: impl IntoIterator<Item = &'a TypeEntry>) {
        for entry in type_entries {
            self.uuids
                .entry(entry.type_id())
                .or_insert_with(|| ArcStr::from(entry.name()));
        }
    }

    /// Add fallback field-name mappings (CRC computed from each name) without
    /// replacing names already loaded from a runtime reflection dump. Symmetric
    /// with [`extend_type_names`](Self::extend_type_names) for recipe-seeded
    /// classes whose field names the captured registry no longer carries.
    pub fn extend_field_names<'a>(&mut self, field_names: impl IntoIterator<Item = &'a str>) {
        for name in field_names {
            self.crcs
                .entry(crate::field_name_crc(name))
                .or_insert_with(|| ArcStr::from(name));
        }
    }

    #[inline]
    #[must_use]
    pub fn uuid_count(&self) -> usize {
        self.uuids.len()
    }

    #[inline]
    #[must_use]
    pub fn crc_count(&self) -> usize {
        self.crcs.len()
    }

    /// Look up a class name by AZ type UUID. The returned
    /// [`ArcStr`] is cheap to clone (refcount bump).
    #[inline]
    #[must_use]
    pub fn type_name(&self, uuid: &Uuid) -> Option<&ArcStr> {
        self.uuids.get(uuid)
    }

    /// Look up a field name by CRC-32 hash.
    #[inline]
    #[must_use]
    pub fn field_name(&self, crc: u32) -> Option<&ArcStr> {
        self.crcs.get(&crc)
    }
}

impl FromIterator<(Uuid, ArcStr)> for LumberyardHashes {
    fn from_iter<I: IntoIterator<Item = (Uuid, ArcStr)>>(iter: I) -> Self {
        Self {
            uuids: iter.into_iter().collect(),
            crcs: HashMap::new(),
        }
    }
}

impl FromIterator<(u32, ArcStr)> for LumberyardHashes {
    fn from_iter<I: IntoIterator<Item = (u32, ArcStr)>>(iter: I) -> Self {
        Self {
            uuids: HashMap::new(),
            crcs: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objectstream_type_aliases_seed_lookup_names() {
        let mut hashes = LumberyardHashes::new();
        hashes.extend_types(OBJECT_STREAM_TYPES);

        assert_eq!(
            hashes
                .type_name(&type_ids::AZSTD_STRING_LEGACY_XML)
                .map(arcstr::ArcStr::as_str),
            Some("AZStd::string")
        );
        assert_eq!(
            hashes
                .type_name(&type_ids::AZSTD_VECTOR_LEGACY_XML)
                .map(arcstr::ArcStr::as_str),
            Some("AZStd::vector")
        );
        assert_eq!(
            hashes
                .type_name(&type_ids::BYTE_STREAM)
                .map(arcstr::ArcStr::as_str),
            Some("ByteStream")
        );
    }

    #[test]
    fn type_entries_do_not_replace_runtime_names() {
        let mut hashes = LumberyardHashes::new();
        hashes.insert_type_name(type_ids::AZSTD_STRING_LEGACY_XML, "RuntimeName");
        hashes.extend_types(OBJECT_STREAM_TYPES);

        assert_eq!(
            hashes
                .type_name(&type_ids::AZSTD_STRING_LEGACY_XML)
                .map(arcstr::ArcStr::as_str),
            Some("RuntimeName")
        );
    }
}
