//! Deterministic pak metadata reports.

use std::fmt;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::archive::{PakArchive, PakError, PakFile};
use crate::format_report_size;

/// Borrowed view for printing a pak entry-table inspection.
#[derive(Clone, Copy)]
pub struct PakInspectionReport<'a, R: Read + Seek> {
    archive: &'a PakArchive<R>,
    source: &'a Path,
    filter: Option<&'a str>,
    limit: usize,
}

impl<R: Read + Seek> PakArchive<R> {
    /// Build a displayable entry-table inspection report.
    #[must_use]
    pub const fn inspection_report<'a>(
        &'a self,
        source: &'a Path,
        filter: Option<&'a str>,
        limit: usize,
    ) -> PakInspectionReport<'a, R> {
        PakInspectionReport {
            archive: self,
            source,
            filter,
            limit,
        }
    }
}

impl<R: Read + Seek> fmt::Display for PakInspectionReport<'_, R> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "the compression ratio is a human-readable summary printed to two \
                  decimals; f64's 53-bit mantissa is exact for byte totals up to 8 PiB, \
                  well beyond any pak, and rounding beyond that cannot change the \
                  printed value"
    )]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{}: {} entries",
            self.source.display(),
            self.archive.len()
        )?;

        let mut shown = 0usize;
        let mut total_uncompressed: u64 = 0;
        let mut total_compressed: u64 = 0;
        for entry in self.archive.entries() {
            total_uncompressed += entry.uncompressed_size();
            total_compressed += entry.compressed_size();
            if let Some(filter) = self.filter
                && !entry.name().contains(filter)
            {
                continue;
            }
            if shown < self.limit {
                writeln!(
                    f,
                    "  {:>10} -> {:>10}  {}  {}",
                    format_report_size(entry.compressed_size()),
                    format_report_size(entry.uncompressed_size()),
                    entry.compression(),
                    entry.name(),
                )?;
                shown += 1;
            }
        }

        writeln!(f)?;
        writeln!(
            f,
            "  totals: {} compressed -> {} uncompressed (ratio {:.2})",
            format_report_size(total_compressed),
            format_report_size(total_uncompressed),
            if total_compressed == 0 {
                0.0
            } else {
                total_uncompressed as f64 / total_compressed as f64
            }
        )
    }
}

#[derive(Debug, Error)]
pub enum PakInspectionError {
    #[error("open pak {path:?}")]
    Open {
        path: PathBuf,
        #[source]
        source: PakError,
    },
}

/// Open the pak at `path` and render its entry-table inspection report.
///
/// # Errors
///
/// Returns [`PakInspectionError::Open`] when the pak cannot be opened — the
/// wrapped [`PakError`] distinguishes an I/O failure from an unparseable zip
/// central directory. Rendering the report itself cannot fail.
pub fn inspect_pak_path(
    path: impl AsRef<Path>,
    filter: Option<&str>,
    limit: usize,
) -> Result<String, PakInspectionError> {
    let path = path.as_ref();
    let archive = PakFile::open(path).map_err(|source| PakInspectionError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(archive.inspection_report(path, filter, limit).to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::path::Path;

    use zip::CompressionMethod;

    use crate::PakArchive;

    #[test]
    fn inspection_report_is_deterministic() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<'_, '_, ()> =
                zip::write::FileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("textures/a.dds", opts).unwrap();
            zip.write_all(&[1, 2, 3, 4]).unwrap();
            zip.start_file("levels/main.slice", opts).unwrap();
            zip.write_all(&[5, 6]).unwrap();
            zip.finish().unwrap();
        }

        let archive = PakArchive::from_reader(Cursor::new(buf.into_inner())).unwrap();
        let report = archive
            .inspection_report(Path::new("assets/test.pak"), Some("textures/"), 10)
            .to_string();

        assert!(report.contains("assets/test.pak: 2 entries"));
        assert!(report.contains("4 B ->        4 B  stored  textures/a.dds"));
        assert!(report.contains("totals: 6 B compressed -> 6 B uncompressed (ratio 1.00)"));
    }
}
