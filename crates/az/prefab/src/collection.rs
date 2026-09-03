//! Canonical authoring model for one source that emits several Prefab assets.

use std::collections::{BTreeMap, BTreeSet};

use bevy_reflect::TypeRegistry;
use ron::{ser::PrettyConfig, value::RawValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{PrefabAssetPath, PrefabCodec, PrefabCodecError, PrefabDocument, codec::UniqueMap};

/// Current source-container version for Prefab collections.
pub const PREFAB_COLLECTION_VERSION: u32 = 1;
/// Logical source type advertised to project workflow and asset-builder routing.
pub const PREFAB_COLLECTION_SOURCE_TYPE: &str = "azoth.prefab.PrefabCollection";

/// One Prefab asset authored inside a [`PrefabCollection`].
#[derive(Debug, Clone, PartialEq)]
pub struct PrefabCollectionEntry {
    source_path: PrefabAssetPath,
    document: PrefabDocument,
}

impl PrefabCollectionEntry {
    /// Creates one collection entry at its logical Prefab source path.
    ///
    /// The logical path determines the canonical product path and remains the
    /// identity used by nested-Prefab source references. It does not name a
    /// second source file.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCollectionError::Document`] when `source_path` is not a
    /// valid Prefab asset path.
    pub fn new(
        source_path: impl Into<String>,
        document: PrefabDocument,
    ) -> Result<Self, PrefabCollectionError> {
        Ok(Self {
            source_path: PrefabAssetPath::new(source_path)?,
            document,
        })
    }

    #[must_use]
    pub const fn source_path(&self) -> &PrefabAssetPath {
        &self.source_path
    }

    #[must_use]
    pub const fn document(&self) -> &PrefabDocument {
        &self.document
    }
}

/// One authored source that emits multiple ordinary Prefab assets.
///
/// The source GUID is owned by normal asset-source metadata. Each entry owns a
/// stable product sub-ID, so `(source GUID, sub-ID)` remains the complete
/// runtime [`az_core::AssetId`] without redirects or lookup fallbacks.
#[derive(Debug, Clone, PartialEq)]
pub struct PrefabCollection {
    entries: BTreeMap<u32, PrefabCollectionEntry>,
}

impl PrefabCollection {
    /// Creates a non-empty collection with unique sub-IDs and logical paths.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCollectionError::Empty`],
    /// [`PrefabCollectionError::DuplicateSubId`], or
    /// [`PrefabCollectionError::DuplicateSourcePath`] when the collection
    /// cannot identify every emitted Prefab unambiguously.
    pub fn try_from_entries(
        entries: impl IntoIterator<Item = (u32, PrefabCollectionEntry)>,
    ) -> Result<Self, PrefabCollectionError> {
        let mut by_sub_id = BTreeMap::new();
        let mut source_paths = BTreeSet::new();
        for (sub_id, entry) in entries {
            if !source_paths.insert(entry.source_path.as_str().to_ascii_lowercase()) {
                return Err(PrefabCollectionError::DuplicateSourcePath(
                    entry.source_path.to_string(),
                ));
            }
            if by_sub_id.insert(sub_id, entry).is_some() {
                return Err(PrefabCollectionError::DuplicateSubId(sub_id));
            }
        }
        if by_sub_id.is_empty() {
            return Err(PrefabCollectionError::Empty);
        }
        Ok(Self { entries: by_sub_id })
    }

    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (u32, &PrefabCollectionEntry)> {
        self.entries.iter().map(|(&sub_id, entry)| (sub_id, entry))
    }

    #[must_use]
    pub fn entry(&self, sub_id: u32) -> Option<&PrefabCollectionEntry> {
        self.entries.get(&sub_id)
    }

    #[must_use]
    pub fn entry_by_source_path(&self, source_path: &str) -> Option<&PrefabCollectionEntry> {
        self.entries
            .values()
            .find(|entry| entry.source_path.as_str().eq_ignore_ascii_case(source_path))
    }
}

/// Registry-bound canonical RON codec for [`PrefabCollection`].
pub struct PrefabCollectionCodec<'a> {
    prefab: PrefabCodec<'a>,
}

impl<'a> PrefabCollectionCodec<'a> {
    /// Binds the collection codec to the same registry used for its embedded
    /// [`PrefabDocument`] values.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCollectionError::PrefabCodec`] when the Prefab codec
    /// cannot initialize from `registry`.
    pub fn new(registry: &'a TypeRegistry) -> Result<Self, PrefabCollectionError> {
        Ok(Self {
            prefab: PrefabCodec::new(registry)?,
        })
    }

    /// Decodes and validates one canonical collection source.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCollectionError`] for malformed RON, an unsupported
    /// collection version, invalid or duplicate entry identity, or an invalid
    /// embedded Prefab document.
    pub fn decode(&self, source: &str) -> Result<PrefabCollection, PrefabCollectionError> {
        let raw: RawPrefabCollection = ron::from_str(source)
            .map_err(|error| PrefabCollectionError::Parse(error.to_string()))?;
        if raw.version != PREFAB_COLLECTION_VERSION {
            return Err(PrefabCollectionError::UnsupportedVersion {
                actual: raw.version,
                supported: PREFAB_COLLECTION_VERSION,
            });
        }
        PrefabCollection::try_from_entries(
            raw.entries
                .0
                .into_iter()
                .map(|(sub_id, entry)| {
                    Ok((
                        sub_id,
                        PrefabCollectionEntry::new(
                            entry.source_path,
                            self.prefab.decode(entry.document.get_ron())?,
                        )?,
                    ))
                })
                .collect::<Result<Vec<_>, PrefabCollectionError>>()?,
        )
    }

    /// Writes deterministic canonical collection source.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCollectionError`] when an embedded Prefab is not
    /// canonical or RON serialization fails.
    pub fn encode(&self, collection: &PrefabCollection) -> Result<String, PrefabCollectionError> {
        let entries = collection
            .iter()
            .map(|(sub_id, entry)| {
                let source = self.prefab.encode(entry.document())?;
                let document = RawValue::from_boxed_ron(source.into_boxed_str())
                    .map_err(|error| PrefabCollectionError::Parse(error.to_string()))?;
                Ok((
                    sub_id,
                    RawPrefabCollectionEntry {
                        source_path: entry.source_path().as_str().to_owned(),
                        document,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, PrefabCollectionError>>()?;
        let raw = RawPrefabCollection {
            version: PREFAB_COLLECTION_VERSION,
            entries: UniqueMap(entries),
        };
        let mut source = ron::ser::to_string_pretty(&raw, PrettyConfig::default())
            .map_err(|error| PrefabCollectionError::Serialize(error.to_string()))?;
        source.push('\n');
        Ok(source)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrefabCollection {
    version: u32,
    entries: UniqueMap<u32, RawPrefabCollectionEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrefabCollectionEntry {
    source_path: String,
    document: Box<RawValue>,
}

#[derive(Debug, Error)]
pub enum PrefabCollectionError {
    #[error("Prefab collection must contain at least one entry")]
    Empty,
    #[error("Prefab collection contains duplicate sub-ID {0}")]
    DuplicateSubId(u32),
    #[error("Prefab collection contains duplicate logical source path `{0}`")]
    DuplicateSourcePath(String),
    #[error("Prefab collection version {actual} is unsupported; expected {supported}")]
    UnsupportedVersion { actual: u32, supported: u32 },
    #[error("failed to parse Prefab collection RON: {0}")]
    Parse(String),
    #[error("failed to serialize Prefab collection RON: {0}")]
    Serialize(String),
    #[error(transparent)]
    PrefabCodec(#[from] PrefabCodecError),
    #[error(transparent)]
    Document(#[from] crate::PrefabDocumentError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_round_trips_entries_in_sub_id_order() {
        let registry = TypeRegistry::default();
        let codec = PrefabCollectionCodec::new(&registry).unwrap();
        let collection = PrefabCollection::try_from_entries([
            (
                9,
                PrefabCollectionEntry::new("prefabs/nine.prefab.ron", PrefabDocument::default())
                    .unwrap(),
            ),
            (
                2,
                PrefabCollectionEntry::new("prefabs/two.prefab.ron", PrefabDocument::default())
                    .unwrap(),
            ),
        ])
        .unwrap();

        let source = codec.encode(&collection).unwrap();
        let decoded = codec.decode(&source).unwrap();

        assert_eq!(decoded, collection);
        assert_eq!(
            decoded.iter().map(|(sub_id, _)| sub_id).collect::<Vec<_>>(),
            [2, 9]
        );
    }

    #[test]
    fn collection_rejects_case_folded_duplicate_paths() {
        let error = PrefabCollection::try_from_entries([
            (
                1,
                PrefabCollectionEntry::new("Prefabs/Door.prefab.ron", PrefabDocument::default())
                    .unwrap(),
            ),
            (
                2,
                PrefabCollectionEntry::new("prefabs/door.prefab.ron", PrefabDocument::default())
                    .unwrap(),
            ),
        ])
        .unwrap_err();

        assert!(matches!(
            error,
            PrefabCollectionError::DuplicateSourcePath(_)
        ));
    }

    #[test]
    fn collection_decode_rejects_duplicate_sub_ids() {
        let registry = TypeRegistry::default();
        let codec = PrefabCollectionCodec::new(&registry).unwrap();
        let source = r#"(
            version: 1,
            entries: {
                7: (source_path: "prefabs/a.prefab.ron", document: (version:1,type_versions:{},entities:{},instances:{})),
                7: (source_path: "prefabs/b.prefab.ron", document: (version:1,type_versions:{},entities:{},instances:{})),
            },
        )"#;

        assert!(matches!(
            codec.decode(source),
            Err(PrefabCollectionError::Parse(message)) if message.contains("duplicate map key")
        ));
    }
}
