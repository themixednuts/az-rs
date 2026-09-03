//! Lumberyard-compatible pak archive reader (zip + zlib + Oodle + AZCS).
//!
//! These `.pak` archives are zip files with optional zlib (`Deflated`) or
//! Oodle (custom compression method `15`) compression per entry. After zip
//! decompression, the resulting bytes may also be wrapped in an
//! AZCS-compressed inner layer (Lumberyard's
//! `AzCore::Compression`). This crate handles both layers and gives
//! callers the final decompressed payload.
//!
//! # Scope
//!
//! This crate is a compatibility package backend. Import tooling uses it
//! to read Lumberyard-compatible paks, and `az-framework` can also use its mmap
//! reader when a runtime launch snapshot explicitly selects such a package. It
//! does not decode legacy
//! `ObjectStream`, datasheet, or texture formats by itself; format
//! transforms still belong to import/build crates.
//!
//! # Quick start
//!
//! ```no_run
//! use az_pak::PakFile;
//!
//! let mut pak = PakFile::open("assets/game.pak")?;
//! for entry in pak.entries() {
//!     println!("{}: {} bytes", entry.name(), entry.uncompressed_size());
//! }
//! let bytes = pak.read("textures/foo.dds")?;
//! # Ok::<(), az_pak::PakError>(())
//! ```
//!
//! # Format references
//!
//! The AZCS framing and zlib-specific header follow O3DE's
//! `Code/Framework/AzCore/AzCore/IO/Compressor.h` and
//! `Code/Framework/AzCore/AzCore/IO/CompressorZLib.h`.
//!
//! Format-conversion glue (DDS, Datasheet, `ObjectStream` transforms) is
//! intentionally NOT included — those belong in their own crates.

use humansize::{DECIMAL, format_size};

pub mod archive;
pub mod azcs;
pub mod decompress;
pub mod extract;
pub mod inspection;
pub mod search;

pub use archive::{
    EntryInfo, EntryIter, PakArchive, PakError, PakFile, PakFileMmap, PakMmapReader,
};
pub use azcs::{AZCS_SIGNATURE, AzcsError, AzcsHeader, AzcsId, is_azcs};
pub use decompress::{
    Compression, DecompressError, decompress_bytes_into, decompress_zip_entry,
    decompress_zip_entry_into,
};
pub use extract::{
    PakExtractEntryFailure, PakExtractError, PakExtractFailures, PakExtractOptions,
    PakExtractReport,
};
pub use inspection::{PakInspectionError, PakInspectionReport, inspect_pak_path};
pub use search::{PakSearchReport, PakSearchRow, PakSearchSummary};

/// Render `bytes` in SI units for this crate's `Display` report surfaces.
///
/// The precision is pinned here instead of being inherited from
/// `humansize`'s `DECIMAL` preset. That preset's `decimal_places` is a
/// dependency default, so leaving it implicit lets a `humansize` release
/// silently reword every size column the reports print (and every report
/// snapshot asserted against it). Scale and unit spelling still come from the
/// SI preset.
fn format_report_size(bytes: u64) -> String {
    format_size(bytes, DECIMAL.decimal_places(2).decimal_zeroes(0))
}
