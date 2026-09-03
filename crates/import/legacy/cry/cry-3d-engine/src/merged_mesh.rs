//! Merged mesh render manager asset parsers.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/Cry3DEngine/MergedMeshRenderNode.cpp`.

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use crate::ParseError;
use thiserror::Error;

pub const COMPILED_MERGED_MESHES_BASE_NAME: &str = "terrain/merged_meshes_sectors/";
pub const COMPILED_MERGED_MESHES_LIST: &str = "mmrm_used_meshes.lst";

/// `terrain/merged_meshes_sectors/mmrm_used_meshes.lst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedMeshUsedMeshes<'a> {
    text: &'a str,
    len: usize,
}

impl<'a> MergedMeshUsedMeshes<'a> {
    /// Parse a merged-mesh CGF preload list.
    ///
    /// # Errors
    ///
    /// Returns an error when the list is not valid UTF-8.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let text = std::str::from_utf8(bytes).map_err(|source| ParseError::Utf8 {
            field: COMPILED_MERGED_MESHES_LIST,
            source,
        })?;
        Ok(Self {
            text,
            len: count_used_meshes(text),
        })
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline]
    #[must_use]
    pub fn entries(self) -> MergedMeshUsedMeshIter<'a> {
        MergedMeshUsedMeshIter {
            lines: self.text.lines(),
        }
    }

    #[inline]
    #[must_use]
    pub const fn summary(self) -> MergedMeshUsedMeshesSummary {
        MergedMeshUsedMeshesSummary { meshes: self.len }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergedMeshUsedMeshesSummary {
    pub meshes: usize,
}

impl fmt::Display for MergedMeshUsedMeshesSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} mesh refs", self.meshes)
    }
}

/// Count the mesh references in a merged-mesh preload list.
///
/// # Errors
///
/// Returns any error [`MergedMeshUsedMeshes::parse`] returns.
pub fn summarize_used_meshes(bytes: &[u8]) -> Result<MergedMeshUsedMeshesSummary, ParseError> {
    MergedMeshUsedMeshes::parse(bytes).map(MergedMeshUsedMeshes::summary)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergedMeshUsedMeshesTotals {
    pub files: usize,
    pub meshes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedMeshUsedMeshesFileSummary {
    pub source: String,
    pub summary: MergedMeshUsedMeshesSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergedMeshUsedMeshesInspection {
    pub rows: Vec<MergedMeshUsedMeshesFileSummary>,
    pub totals: MergedMeshUsedMeshesTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct MergedMeshUsedMeshesInspectionReport<'a> {
    inspection: &'a MergedMeshUsedMeshesInspection,
    limit: usize,
}

impl MergedMeshUsedMeshesTotals {
    pub const fn add_summary(&mut self, summary: MergedMeshUsedMeshesSummary) {
        self.files += 1;
        self.meshes += summary.meshes;
    }
}

impl MergedMeshUsedMeshesInspection {
    pub fn add_file_summary(&mut self, row: MergedMeshUsedMeshesFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> MergedMeshUsedMeshesInspectionReport<'_> {
        MergedMeshUsedMeshesInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for MergedMeshUsedMeshesTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  mesh refs: {}", self.meshes)
    }
}

impl fmt::Display for MergedMeshUsedMeshesInspectionReport<'_> {
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

/// Summarize `bytes` into a one-row inspection record naming `path`.
///
/// # Errors
///
/// Returns any error [`summarize_used_meshes`] returns.
pub fn inspect_used_meshes_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<MergedMeshUsedMeshesFileSummary, ParseError> {
    Ok(MergedMeshUsedMeshesFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_used_meshes(bytes)?,
    })
}

#[derive(Debug, Error)]
pub enum MergedMeshUsedMeshesInspectionError {
    #[error("read merged mesh used-mesh list {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse merged mesh used-mesh list {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Read `path` from disk and inspect it as a merged-mesh preload list.
///
/// # Errors
///
/// Returns [`MergedMeshUsedMeshesInspectionError::Read`] when `path` cannot be
/// read, or [`MergedMeshUsedMeshesInspectionError::Parse`] when its bytes are
/// not valid UTF-8.
pub fn inspect_used_meshes_path(
    path: impl AsRef<Path>,
) -> Result<MergedMeshUsedMeshesFileSummary, MergedMeshUsedMeshesInspectionError> {
    let path = path.as_ref();
    let bytes =
        std::fs::read(path).map_err(|source| MergedMeshUsedMeshesInspectionError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    inspect_used_meshes_file(path, &bytes).map_err(|source| {
        MergedMeshUsedMeshesInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Inspect every path in `paths`, accumulating file and mesh-ref totals.
///
/// # Errors
///
/// Returns the first error [`inspect_used_meshes_path`] returns; remaining
/// paths are not visited.
pub fn inspect_used_meshes_files(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<MergedMeshUsedMeshesInspection, MergedMeshUsedMeshesInspectionError> {
    let mut inspection = MergedMeshUsedMeshesInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_used_meshes_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub fn is_used_meshes_name(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(COMPILED_MERGED_MESHES_LIST))
}

#[must_use]
pub fn is_used_meshes_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(COMPILED_MERGED_MESHES_LIST))
}

impl<'a> IntoIterator for MergedMeshUsedMeshes<'a> {
    type Item = MergedMeshUsedMesh<'a>;
    type IntoIter = MergedMeshUsedMeshIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries()
    }
}

/// One CGF referenced by `mmrm_used_meshes.lst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MergedMeshUsedMesh<'a> {
    asset_path: &'a str,
}

impl<'a> MergedMeshUsedMesh<'a> {
    #[inline]
    #[must_use]
    pub const fn asset_path(self) -> &'a str {
        self.asset_path
    }
}

/// Borrowed iterator over merged-mesh CGF references.
#[derive(Debug, Clone)]
pub struct MergedMeshUsedMeshIter<'a> {
    lines: std::str::Lines<'a>,
}

impl<'a> Iterator for MergedMeshUsedMeshIter<'a> {
    type Item = MergedMeshUsedMesh<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.lines
            .by_ref()
            .find_map(|line| (!line.is_empty()).then_some(MergedMeshUsedMesh { asset_path: line }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, upper) = self.lines.size_hint();
        (0, upper)
    }
}

fn count_used_meshes(text: &str) -> usize {
    text.lines().filter(|line| !line.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_used_meshes_without_allocating_entries() {
        let bytes = b"objects/a.cgf\n\nobjects/b.cgf\r\n";
        let list = MergedMeshUsedMeshes::parse(bytes).unwrap();
        let entries = list
            .entries()
            .map(MergedMeshUsedMesh::asset_path)
            .collect::<Vec<_>>();

        assert_eq!(list.len(), 2);
        assert_eq!(entries, ["objects/a.cgf", "objects/b.cgf"]);
        assert_eq!(list.summary(), MergedMeshUsedMeshesSummary { meshes: 2 });
        assert_eq!(list.summary().to_string(), "2 mesh refs");
    }

    #[test]
    fn tracks_totals_and_used_mesh_paths() {
        let mut totals = MergedMeshUsedMeshesTotals::default();
        totals.add_summary(MergedMeshUsedMeshesSummary { meshes: 2 });
        totals.add_summary(MergedMeshUsedMeshesSummary { meshes: 3 });

        assert_eq!(totals.files, 2);
        assert_eq!(totals.meshes, 5);
        assert_eq!(totals.to_string(), "  files: 2\n  mesh refs: 5\n");

        let row = inspect_used_meshes_file(
            "terrain/merged_meshes_sectors/mmrm_used_meshes.lst",
            b"objects/a.cgf\nobjects/b.cgf\n",
        )
        .unwrap();
        let mut inspection = MergedMeshUsedMeshesInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "terrain/merged_meshes_sectors/mmrm_used_meshes.lst: 2 mesh refs\n  files: 1\n  mesh refs: 2\n"
        );
        assert!(is_used_meshes_name(
            "terrain/merged_meshes_sectors/mmrm_used_meshes.lst"
        ));
        assert!(is_used_meshes_name("MMRM_USED_MESHES.LST"));
        assert!(!is_used_meshes_name("objects/foo.cgf"));
    }
}
