//! Material Effects surface-type CSV source import.

use std::{num::ParseIntError, str};

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use csv::{ByteRecord, ReaderBuilder, Trim};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::source_schemas;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTypesSource {
    pub source_path: String,
    pub entries: Vec<SurfaceTypeSourceEntry>,
}

impl SurfaceTypesSource {
    /// Parse a legacy surface-type CSV payload.
    ///
    /// # Errors
    ///
    /// Returns [`SurfaceTypesParseError`] when the CSV is malformed, a header
    /// column is missing, or a surface-type index is not an integer.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, SurfaceTypesParseError> {
        let mut entries = Vec::new();
        visit_surface_types(bytes, |entry| {
            entries.push(SurfaceTypeSourceEntry {
                index: entry.index,
                name: entry.name.to_string(),
            });
            Ok(())
        })?;

        Ok(Self {
            source_path: normalize_source_path(source_path),
            entries,
        })
    }

    /// Serialize this source projection to pretty RON bytes.
    ///
    /// # Errors
    ///
    /// Returns any [`ron::Error`] the RON serializer reports for this value.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTypeSourceEntry {
    pub index: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SurfaceTypesSourceTransform;

impl LegacySourceTransform for SurfaceTypesSourceTransform {
    type Error = SurfaceTypesSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_surface_types_source(&input.source_path) {
            return Err(SurfaceTypesSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = SurfaceTypesSource::from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            surface_types_source_path(&input.source_path),
            source_schemas::SURFACE_TYPES,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn is_legacy_surface_types_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with("libs/materialeffects/surfacetypemapping.csv")
}

#[must_use]
pub fn surface_types_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized.strip_suffix(".csv").unwrap_or(&normalized);
    format!("{stem}.surfacetypes.ron")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfaceTypeRef<'a> {
    index: u32,
    name: &'a str,
}

fn visit_surface_types(
    bytes: &[u8],
    mut visit: impl FnMut(SurfaceTypeRef<'_>) -> Result<(), SurfaceTypesParseError>,
) -> Result<(), SurfaceTypesParseError> {
    visit_typed_records(bytes, &["Index", "SurfaceType"], |row, record| {
        visit(SurfaceTypeRef {
            index: field_u32(record, row, 0, "Index")?,
            name: field_str(record, row, 1, "SurfaceType")?,
        })
    })
}

fn visit_typed_records(
    bytes: &[u8],
    expected_header: &'static [&'static str],
    mut visit_record: impl FnMut(usize, &ByteRecord) -> Result<(), SurfaceTypesParseError>,
) -> Result<(), SurfaceTypesParseError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .trim(Trim::None)
        .from_reader(bytes);
    let mut record = ByteRecord::new();
    if !reader.read_byte_record(&mut record)? {
        return Err(SurfaceTypesParseError::MissingHeader);
    }
    validate_header(1, &record, expected_header)?;

    let mut row = 1usize;
    while reader.read_byte_record(&mut record)? {
        row += 1;
        visit_record(row, &record)?;
    }
    Ok(())
}

fn validate_header(
    row: usize,
    record: &ByteRecord,
    expected: &'static [&'static str],
) -> Result<(), SurfaceTypesParseError> {
    let mut found = Vec::with_capacity(record.len());
    for field in record {
        found.push(trim_utf8(field)?.to_string());
    }

    if found.len() == expected.len()
        && found
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
    {
        return Ok(());
    }

    Err(SurfaceTypesParseError::UnexpectedHeader {
        row,
        expected,
        found,
    })
}

fn field_str<'a>(
    record: &'a ByteRecord,
    row: usize,
    index: usize,
    column: &'static str,
) -> Result<&'a str, SurfaceTypesParseError> {
    let field = record
        .get(index)
        .ok_or(SurfaceTypesParseError::MissingField { row, column })?;
    trim_utf8(field)
}

fn field_u32(
    record: &ByteRecord,
    row: usize,
    index: usize,
    column: &'static str,
) -> Result<u32, SurfaceTypesParseError> {
    let value = field_str(record, row, index, column)?;
    value
        .parse()
        .map_err(|source| SurfaceTypesParseError::InvalidInteger {
            row,
            column,
            value: value.to_string(),
            source,
        })
}

fn trim_utf8(bytes: &[u8]) -> Result<&str, SurfaceTypesParseError> {
    Ok(str::from_utf8(bytes)?.trim())
}

#[derive(Debug, thiserror::Error)]
pub enum SurfaceTypesParseError {
    #[error("surface type CSV is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),
    #[error("CSV parser error: {0}")]
    Csv(#[from] csv::Error),
    #[error("surface type CSV is missing a header row")]
    MissingHeader,
    #[error("row {row} has unexpected header {found:?}; expected {expected:?}")]
    UnexpectedHeader {
        row: usize,
        expected: &'static [&'static str],
        found: Vec<String>,
    },
    #[error("row {row} is missing {column}")]
    MissingField { row: usize, column: &'static str },
    #[error("row {row} has invalid integer in {column}: {value:?}")]
    InvalidInteger {
        row: usize,
        column: &'static str,
        value: String,
        source: ParseIntError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SurfaceTypesSourceTransformError {
    #[error("unsupported surface types CSV path {path}")]
    UnsupportedPath { path: String },
    #[error("parse surface types CSV: {0}")]
    Parse(#[from] SurfaceTypesParseError),
    #[error("serialize surface types source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

    use super::*;

    #[test]
    fn transforms_surface_type_mapping_to_authoring_source() {
        let output = SurfaceTypesSourceTransform
            .transform(LegacySourceInput::new(
                "Libs/MaterialEffects/surfacetypemapping.csv",
                b"Index,SurfaceType\n100,metal\n101,wood\n",
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring artifact");
        assert_eq!(
            artifact.path,
            "libs/materialeffects/surfacetypemapping.surfacetypes.ron"
        );
        assert_eq!(artifact.schema, source_schemas::SURFACE_TYPES);
        let source: SurfaceTypesSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(
            source.source_path,
            "libs/materialeffects/surfacetypemapping.csv"
        );
        assert_eq!(source.entries[0].index, 100);
        assert_eq!(source.entries[0].name, "metal");
        assert_eq!(source.entries[1].index, 101);
        assert_eq!(source.entries[1].name, "wood");
    }

    #[test]
    fn surface_types_source_paths_only_claim_surface_mapping_csv() {
        assert!(is_legacy_surface_types_source(
            "Libs/MaterialEffects/surfacetypemapping.csv"
        ));
        assert_eq!(
            surface_types_source_path("Libs/MaterialEffects/surfacetypemapping.csv"),
            "libs/materialeffects/surfacetypemapping.surfacetypes.ron"
        );
        assert!(!is_legacy_surface_types_source(
            "dictionary/en-us/profanity.csv"
        ));
        assert!(!is_legacy_surface_types_source(
            "sounds/wwise/npc_alligator_events.csv"
        ));
    }

    #[test]
    #[ignore = "requires AZOTH_RELEASE_SOURCE pointing at a local release corpus"]
    fn transforms_configured_surface_types_corpus() {
        let release_source =
            std::env::var("AZOTH_RELEASE_SOURCE").expect("AZOTH_RELEASE_SOURCE must be set");
        let bytes = std::fs::read(
            std::path::Path::new(&release_source)
                .join("libs/materialeffects/surfacetypemapping.csv"),
        )
        .unwrap();

        let output = SurfaceTypesSourceTransform
            .transform(LegacySourceInput::new(
                "libs/materialeffects/surfacetypemapping.csv",
                &bytes,
            ))
            .unwrap();
        let source: SurfaceTypesSource =
            ron::de::from_bytes(&output.artifact().unwrap().bytes).unwrap();
        assert!(!source.entries.is_empty());
        assert_eq!(source.entries[0].index, 0);
        assert_eq!(source.entries[0].name, "default");
    }
}
