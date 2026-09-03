//! Wwise asset inspection summaries.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use super::asset::{WwiseMediaAsset, WwiseSoundBankAsset};
use super::bank::WwiseHierarchyObjectKind;
use super::error::WwiseAssetLoadError;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WwiseAssetKind {
    SoundBank,
    Media,
}

impl WwiseAssetKind {
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("bnk") => Some(Self::SoundBank),
            Some("wem") => Some(Self::Media),
            _ => None,
        }
    }
}

#[must_use]
pub fn is_wwise_asset_name(path: &str) -> bool {
    WwiseAssetKind::from_path(Path::new(path)).is_some()
}

#[must_use]
pub fn is_wwise_asset_path(path: &Path) -> bool {
    WwiseAssetKind::from_path(path).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WwiseAssetSummary {
    Bank(WwiseBankSummary),
    Media(WwiseMediaSummary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WwiseBankSummary {
    pub bank_id: Option<u32>,
    pub sections: usize,
    pub media_entries: usize,
    pub hierarchy_objects: usize,
    pub event_objects: usize,
    pub event_actions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WwiseMediaSummary {
    pub bytes: usize,
    pub chunks: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WwiseCollectionSummary {
    pub files: usize,
    pub banks: usize,
    pub media: usize,
    pub sections: usize,
    pub media_entries: usize,
    pub hierarchy_objects: usize,
    pub event_objects: usize,
    pub event_actions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WwiseAssetFileSummary {
    pub source: String,
    pub summary: WwiseAssetSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WwiseInspection {
    pub rows: Vec<WwiseAssetFileSummary>,
    pub totals: WwiseCollectionSummary,
}

#[derive(Debug, Clone, Copy)]
pub struct WwiseInspectionReport<'a> {
    inspection: &'a WwiseInspection,
    limit: usize,
}

#[derive(Debug, Error)]
pub enum WwiseAssetInspectionError {
    #[error("unknown Wwise asset path {path}")]
    UnknownPath { path: String },
    #[error("read {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("load Wwise asset {path:?}")]
    Load {
        path: PathBuf,
        #[source]
        source: WwiseAssetLoadError,
    },
}

/// Parse `bytes` as `kind` and reduce it to a counts-only summary.
///
/// # Errors
///
/// Returns [`WwiseAssetLoadError::SoundBank`] or
/// [`WwiseAssetLoadError::Media`] if `bytes` is not a well-formed container of
/// the requested kind.
pub fn summarize_wwise_asset(
    kind: WwiseAssetKind,
    bytes: &[u8],
) -> Result<WwiseAssetSummary, WwiseAssetLoadError> {
    match kind {
        WwiseAssetKind::SoundBank => {
            let asset = WwiseSoundBankAsset::from_bytes(bytes)?;
            Ok(WwiseAssetSummary::Bank(WwiseBankSummary::from_asset(
                &asset,
            )))
        }
        WwiseAssetKind::Media => {
            let asset = WwiseMediaAsset::from_bytes(bytes)?;
            Ok(WwiseAssetSummary::Media(WwiseMediaSummary::from_asset(
                &asset,
            )))
        }
    }
}

/// Summarize already-read bytes, taking the asset kind from `path`'s extension.
///
/// # Errors
///
/// Returns [`WwiseAssetInspectionError::UnknownPath`] if `path`'s extension is
/// not a known Wwise asset extension, or [`WwiseAssetInspectionError::Load`] if
/// `bytes` fails to parse.
pub fn inspect_wwise_asset_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<WwiseAssetFileSummary, WwiseAssetInspectionError> {
    let path = path.as_ref();
    let kind =
        WwiseAssetKind::from_path(path).ok_or_else(|| WwiseAssetInspectionError::UnknownPath {
            path: path.display().to_string(),
        })?;
    Ok(WwiseAssetFileSummary {
        source: path.display().to_string(),
        summary: summarize_wwise_asset(kind, bytes).map_err(|source| {
            WwiseAssetInspectionError::Load {
                path: path.to_path_buf(),
                source,
            }
        })?,
    })
}

/// Read a Wwise asset off disk and summarize it.
///
/// # Errors
///
/// Returns [`WwiseAssetInspectionError::Read`] if `path` cannot be read, plus
/// any error [`inspect_wwise_asset_file`] returns.
pub fn inspect_wwise_asset_path(
    path: impl AsRef<Path>,
) -> Result<WwiseAssetFileSummary, WwiseAssetInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| WwiseAssetInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_wwise_asset_file(path, &bytes)
}

/// Read and summarize every path in `paths`, stopping at the first failure.
///
/// # Errors
///
/// Returns the first error [`inspect_wwise_asset_path`] returns for any path.
pub fn inspect_wwise_asset_files<I, P>(
    paths: I,
) -> Result<WwiseInspection, WwiseAssetInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = WwiseInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_wwise_asset_path(path)?);
    }
    Ok(inspection)
}

impl WwiseBankSummary {
    #[must_use]
    pub fn from_asset(asset: &WwiseSoundBankAsset) -> Self {
        let bank = &asset.bank;
        let event_objects = bank
            .hierarchy
            .iter()
            .filter(|object| object.kind == WwiseHierarchyObjectKind::EVENT)
            .count();
        let event_actions = bank
            .hierarchy
            .iter()
            .filter_map(|object| object.event_action_count)
            .sum();

        Self {
            bank_id: bank.header.map(|header| header.bank_id.0),
            sections: bank.sections.len(),
            media_entries: bank.media.len(),
            hierarchy_objects: bank.hierarchy.len(),
            event_objects,
            event_actions,
        }
    }
}

impl WwiseMediaSummary {
    #[must_use]
    pub fn from_asset(asset: &WwiseMediaAsset) -> Self {
        Self {
            bytes: asset.bytes().len(),
            chunks: asset.info.chunks.len(),
        }
    }
}

impl WwiseCollectionSummary {
    pub fn add_summary(&mut self, summary: WwiseAssetSummary) {
        self.files += 1;
        match summary {
            WwiseAssetSummary::Bank(summary) => {
                self.banks += 1;
                self.sections += summary.sections;
                self.media_entries += summary.media_entries;
                self.hierarchy_objects += summary.hierarchy_objects;
                self.event_objects += summary.event_objects;
                self.event_actions += u64::from(summary.event_actions);
            }
            WwiseAssetSummary::Media(_) => {
                self.media += 1;
            }
        }
    }
}

impl WwiseInspection {
    pub fn add_file_summary(&mut self, row: WwiseAssetFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> WwiseInspectionReport<'_> {
        WwiseInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for WwiseCollectionSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  banks: {}", self.banks)?;
        writeln!(f, "  media: {}", self.media)?;
        writeln!(f, "  bank sections: {}", self.sections)?;
        writeln!(f, "  embedded media refs: {}", self.media_entries)?;
        writeln!(f, "  HIRC objects: {}", self.hierarchy_objects)?;
        writeln!(f, "  Event objects: {}", self.event_objects)?;
        writeln!(f, "  Event actions: {}", self.event_actions)
    }
}

impl fmt::Display for WwiseInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.summary)?;
            }

            if self.inspection.rows.len() > self.limit {
                writeln!(
                    f,
                    "... {} more files",
                    self.inspection.rows.len() - self.limit
                )?;
            }
        }

        write!(f, "{}", self.inspection.totals)
    }
}

impl fmt::Display for WwiseAssetSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bank(summary) => fmt::Display::fmt(summary, f),
            Self::Media(summary) => fmt::Display::fmt(summary, f),
        }
    }
}

impl fmt::Display for WwiseBankSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bank_id = self
            .bank_id
            .map_or_else(|| "unknown".to_string(), |id| id.to_string());
        write!(
            f,
            "bank id={}, {} sections, {} media refs, {} HIRC objects, {} events, {} actions",
            bank_id,
            self.sections,
            self.media_entries,
            self.hierarchy_objects,
            self.event_objects,
            self.event_actions
        )
    }
}

impl fmt::Display for WwiseMediaSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "encoded media, {} bytes, {} chunks",
            self.bytes, self.chunks
        )
    }
}
