//! Display surfaces for pak entry-table searches.

use std::fmt;
use std::path::Path;

use az_filesystem::display_relative;

use crate::format_report_size;

/// One pak entry-table search hit.
#[derive(Debug, Clone, Copy)]
pub struct PakSearchRow<'a> {
    pub pak_path: &'a Path,
    pub name: &'a str,
    pub uncompressed_size: u64,
    /// Fuzzy score. Ignored when the search mode is not fuzzy.
    pub score: u32,
}

/// Display report for pak entry-table search hits.
#[derive(Debug, Clone, Copy)]
pub struct PakSearchReport<'rows, 'data> {
    root: &'data Path,
    rows: &'rows [PakSearchRow<'data>],
    fuzzy: bool,
    paths_only: bool,
}

impl<'rows, 'data> PakSearchReport<'rows, 'data> {
    #[must_use]
    pub const fn new(
        root: &'data Path,
        rows: &'rows [PakSearchRow<'data>],
        fuzzy: bool,
        paths_only: bool,
    ) -> Self {
        Self {
            root,
            rows,
            fuzzy,
            paths_only,
        }
    }
}

impl fmt::Display for PakSearchReport<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in self.rows {
            if self.paths_only {
                writeln!(f, "{}", row.name)?;
                continue;
            }

            let pak_rel = display_relative(self.root, row.pak_path);
            if self.fuzzy {
                writeln!(
                    f,
                    "[{:>5}] {:>10}  {} :: {}",
                    row.score,
                    format_report_size(row.uncompressed_size),
                    pak_rel,
                    row.name
                )?;
            } else {
                writeln!(
                    f,
                    "{:>10}  {} :: {}",
                    format_report_size(row.uncompressed_size),
                    pak_rel,
                    row.name
                )?;
            }
        }
        Ok(())
    }
}

/// Summary line for a pak entry-table search.
#[derive(Debug, Clone, Copy)]
pub struct PakSearchSummary {
    total_hits: usize,
    shown: usize,
    pak_count: usize,
}

impl PakSearchSummary {
    #[must_use]
    pub const fn new(total_hits: usize, shown: usize, pak_count: usize) -> Self {
        Self {
            total_hits,
            shown,
            pak_count,
        }
    }
}

impl fmt::Display for PakSearchSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} matches across {} paks{}",
            self.total_hits,
            self.pak_count,
            if self.shown < self.total_hits {
                format!(" (showing top {})", self.shown)
            } else {
                String::new()
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_report_formats_rows() {
        let root = Path::new("install");
        let rows = vec![PakSearchRow {
            pak_path: Path::new("install/assets/data.pak"),
            name: "objects/foo.cgf",
            uncompressed_size: 1234,
            score: 42,
        }];

        // The size column's two-decimal precision is pinned by
        // `crate::format_report_size`, not by the `humansize` preset default.
        assert_eq!(
            PakSearchReport::new(root, &rows, true, false).to_string(),
            "[   42]    1.23 kB  assets/data.pak :: objects/foo.cgf\n"
        );
        assert_eq!(
            PakSearchReport::new(root, &rows, false, true).to_string(),
            "objects/foo.cgf\n"
        );
        assert_eq!(
            PakSearchSummary::new(10, 3, 2).to_string(),
            "10 matches across 2 paks (showing top 3)\n"
        );
    }
}
