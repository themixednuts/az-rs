//! Pak archive wrapper around `zip::ZipArchive`.
//!
//! One generic [`PakArchive<R>`] over any seekable reader, with a
//! [`PakFile`] type alias for the file-backed common case.
//!
//! # Performance
//!
//! - [`PakFileMmap::open_mmap`] memory-maps the archive — best
//!   for repeated / interactive scans where the OS file cache
//!   stays warm. ~10× faster than buffered I/O on warm cache,
//!   ~3× slower cold (Windows mmap fault-in is more expensive
//!   per page than batched `read()`).
//! - [`PakFile::open`] uses a 16 KiB [`BufReader`]. See
//!   [`PAK_READ_BUFFER_BYTES`] for why larger buffers can hurt this access
//!   pattern.
//! - Entry metadata (name, sizes, compression) is collected once at
//!   open time into a `Vec<EntryInfo>` so [`PakArchive::entries`]
//!   yields fully-populated metadata without taking a mutable
//!   reference back to the underlying zip state.
//! - [`PakFile::extract_parallel`] decompresses N entries in
//!   parallel via rayon — each worker opens its own `PakFile`
//!   (fresh `File` + `ZipArchive`) so the slow oodle/zlib step
//!   runs concurrently with no contention. **Don't** wrap a
//!   single `PakArchive` in a `Mutex` and call from multiple
//!   threads: the mutex serializes the decompress step and you
//!   get zero actual parallelism.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek};
use std::path::Path;

use rayon::prelude::*;
use thiserror::Error;
use zip::ZipArchive;
use zip::result::ZipError;

use crate::decompress::{Compression, DecompressError, decompress_bytes_into};

/// LFH (Local File Header) signature: `PK\x03\x04`.
const LFH_SIGNATURE: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
/// LFH fixed-size prefix length, before the variable-length name +
/// extra fields.
const LFH_FIXED_LEN: usize = 30;

/// `BufReader` capacity used by [`PakFile::open`].
///
/// `zip::ZipArchive` does many seek-and-small-read
/// cycles while parsing the central directory. Every seek
/// invalidates `BufReader`'s buffer; every refill reads up to
/// `capacity` bytes regardless of the request size. Large buffers create read
/// amplification; 16 KiB amortizes central-directory reads without excessive
/// read-ahead.
pub const PAK_READ_BUFFER_BYTES: usize = 16 * 1024;

#[derive(Debug, Error)]
pub enum PakError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] ZipError),

    #[error("decompress error: {0}")]
    Decompress(#[from] DecompressError),

    #[error("pak entry not found: {0}")]
    NotFound(String),

    #[error("pak entry index out of bounds: {0}")]
    IndexOutOfBounds(usize),

    #[error("pak entry {entry:?} size {size} does not fit in this target's address space")]
    EntryTooLarge { entry: String, size: u64 },

    #[error(
        "corrupt local-file-header for entry {entry:?} at offset {offset:#x}: bad PK signature"
    )]
    CorruptHeader { entry: String, offset: u64 },
}

/// File-backed pak archive — the common case.
pub type PakFile = PakArchive<BufReader<File>>;

/// Mmap-backed pak archive (open via [`PakFile::open_mmap`]).
pub type PakFileMmap = PakArchive<Cursor<memmap2::Mmap>>;

/// Shared mmap-backed pak reader for repeated runtime lookups.
///
/// Unlike [`PakArchive`], reads take `&self`: the central directory is
/// parsed once at open time, then each lookup indexes directly into the
/// mmap and decompresses the entry bytes from a slice. That makes the
/// reader cheap to share behind an `Arc` without serialising unrelated
/// asset loads on a mutex.
pub struct PakMmapReader {
    mmap: memmap2::Mmap,
    metadata: Vec<EntryMetadata>,
    by_name: HashMap<String, usize>,
}

impl PakMmapReader {
    /// Open a pak from a filesystem path via memory-mapped I/O.
    ///
    /// # Safety
    ///
    /// `memmap2::Mmap` is internally `unsafe`; if the file is
    /// concurrently truncated or modified, reads may observe torn data.
    /// Azoth package outputs are immutable once promoted into a launch
    /// snapshot, so runtime use satisfies that constraint.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::Io`] when `path` cannot be opened or mapped, and
    /// [`PakError::Zip`] when the zip central directory cannot be parsed or
    /// an entry name is not decodable.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PakError> {
        let file = File::open(path.as_ref())?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let bytes: &[u8] = &mmap;
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        let metadata = collect_metadata(&mut archive)?;
        drop(archive);

        let mut by_name = HashMap::with_capacity(metadata.len());
        for (index, entry) in metadata.iter().enumerate() {
            by_name.entry(entry.name.clone()).or_insert(index);
        }

        Ok(Self {
            mmap,
            metadata,
            by_name,
        })
    }

    /// Open a pak for repeated reads of a known subset of entries.
    ///
    /// The central directory is parsed once, but only metadata for `names` is
    /// retained after open. This keeps shared cross-package import readers
    /// proportional to their actual working set rather than to every entry in
    /// every mounted archive.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::Io`] when `path` cannot be opened or mapped,
    /// [`PakError::Zip`] when the zip central directory cannot be parsed, and
    /// [`PakError::NotFound`] when any requested name is absent from the
    /// archive.
    pub fn open_selected<'a>(
        path: impl AsRef<Path>,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, PakError> {
        let selected = names.into_iter().collect::<HashSet<_>>();
        let file = File::open(path.as_ref())?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let bytes: &[u8] = &mmap;
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        let metadata = collect_selected_metadata(&mut archive, &selected)?;
        drop(archive);

        let mut by_name = HashMap::with_capacity(metadata.len());
        for (index, entry) in metadata.iter().enumerate() {
            by_name.entry(entry.name.clone()).or_insert(index);
        }
        if let Some(missing) = selected
            .into_iter()
            .find(|name| !by_name.contains_key(*name))
        {
            return Err(PakError::NotFound(missing.to_string()));
        }

        Ok(Self {
            mmap,
            metadata,
            by_name,
        })
    }

    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.metadata.len()
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.metadata.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn entries(&self) -> EntryIter<'_> {
        EntryIter {
            inner: self.metadata.iter(),
        }
    }

    #[inline]
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<EntryInfo<'_>> {
        self.by_name
            .get(name)
            .and_then(|index| self.metadata.get(*index))
            .map(EntryMetadata::view)
    }

    #[inline]
    #[must_use]
    pub fn entry_by_index(&self, idx: usize) -> Option<EntryInfo<'_>> {
        self.metadata.get(idx).map(EntryMetadata::view)
    }

    /// Read + decompress a retained entry by name.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::NotFound`] when `name` is not among the retained
    /// entries, and otherwise propagates [`PakMmapReader::read_by_index`]:
    /// [`PakError::CorruptHeader`], [`PakError::EntryTooLarge`], or
    /// [`PakError::Decompress`].
    pub fn read(&self, name: &str) -> Result<Vec<u8>, PakError> {
        let index = self
            .by_name
            .get(name)
            .copied()
            .ok_or_else(|| PakError::NotFound(name.to_string()))?;
        self.read_by_index(index)
    }

    /// Read + decompress a retained entry by its index in [`Self::entries`].
    ///
    /// # Errors
    ///
    /// Returns [`PakError::IndexOutOfBounds`] when `idx` is past the retained
    /// entry table, [`PakError::CorruptHeader`] when the entry's local file
    /// header falls outside the mapping or lacks the `PK\x03\x04` signature,
    /// [`PakError::EntryTooLarge`] when a recorded size exceeds this target's
    /// address space, and [`PakError::Decompress`] when the payload fails to
    /// decode.
    pub fn read_by_index(&self, idx: usize) -> Result<Vec<u8>, PakError> {
        let entry = self
            .metadata
            .get(idx)
            .ok_or(PakError::IndexOutOfBounds(idx))?;
        decompress_entry_from_slice(&self.mmap, entry)
    }
}

/// Pak archive over any seekable reader.
///
/// Entry metadata is pre-collected at open time so [`PakArchive::entries`]
/// (and any other metadata-only operation) takes `&self` and is
/// cheap to share. Reading entry *bytes* requires `&mut self`
/// because zip's underlying reader needs a mutable position.
///
/// For parallel decompression, **don't** wrap a single `PakArchive`
/// in a `Mutex` — that serializes the slow decompress step. Instead
/// open one [`PakFile`] per rayon worker via
/// [`PakFile::extract_parallel`] (or use [`map_init`][1] to roll
/// your own).
///
/// [1]: https://docs.rs/rayon/latest/rayon/iter/trait.IndexedParallelIterator.html#method.map_init
pub struct PakArchive<R: Read + Seek> {
    inner: ZipArchive<R>,
    metadata: Vec<EntryMetadata>,
}

impl PakFile {
    /// Open a pak from a filesystem path, buffered.
    ///
    /// Uses a [`PAK_READ_BUFFER_BYTES`]-sized (16 KiB) [`BufReader`].
    /// See the constant's docs for the bisect table that picked
    /// this size — bigger is dramatically worse here because
    /// `zip::ZipArchive` does small seek+read cycles that pay
    /// the full `BufReader` capacity in I/O per refill.
    ///
    /// For interactive workflows that re-scan paks repeatedly,
    /// prefer [`PakFileMmap::open_mmap`] — it's ~10× faster on
    /// warm cache, at the cost of being ~3× slower on the first
    /// cold run.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::Io`] when `path` cannot be opened, and
    /// [`PakError::Zip`] when the zip central directory cannot be parsed or
    /// an entry name is not decodable.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PakError> {
        let file = File::open(path.as_ref())?;
        Self::from_reader(BufReader::with_capacity(PAK_READ_BUFFER_BYTES, file))
    }
}

impl PakFileMmap {
    /// Open a pak from a filesystem path via memory-mapped I/O.
    ///
    /// # Safety
    ///
    /// `memmap2::Mmap` is internally `unsafe`; if the file is concurrently
    /// truncated or modified, reads may observe torn data. The caller must
    /// keep the archive immutable for the lifetime of the mapping.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::Io`] when `path` cannot be opened or mapped, and
    /// [`PakError::Zip`] when the zip central directory cannot be parsed or
    /// an entry name is not decodable.
    pub fn open_mmap(path: impl AsRef<Path>) -> Result<Self, PakError> {
        let file = File::open(path.as_ref())?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Self::from_reader(Cursor::new(mmap))
    }
}

impl<R: Read + Seek> PakArchive<R> {
    /// Wrap an arbitrary seekable reader (in-memory pak, mmap, etc.).
    ///
    /// # Errors
    ///
    /// Returns [`PakError::Zip`] when `reader` does not yield a parseable zip
    /// central directory, when reading it fails, or when an entry name is not
    /// decodable.
    pub fn from_reader(reader: R) -> Result<Self, PakError> {
        let mut zip = ZipArchive::new(reader)?;
        let metadata = collect_metadata(&mut zip)?;
        Ok(Self {
            inner: zip,
            metadata,
        })
    }

    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.metadata.len()
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.metadata.is_empty()
    }

    /// Cheap, borrow-free metadata for every entry in the pak.
    #[inline]
    #[must_use]
    pub fn entries(&self) -> EntryIter<'_> {
        EntryIter {
            inner: self.metadata.iter(),
        }
    }

    /// Look up entry metadata by name.
    #[inline]
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<EntryInfo<'_>> {
        self.metadata
            .iter()
            .find(|m| m.name == name)
            .map(EntryMetadata::view)
    }

    /// Look up entry metadata by index.
    #[inline]
    #[must_use]
    pub fn entry_by_index(&self, idx: usize) -> Option<EntryInfo<'_>> {
        self.metadata.get(idx).map(EntryMetadata::view)
    }

    /// Read + decompress a named entry into a fresh `Vec<u8>`.
    ///
    /// Takes `&mut self` because the zip crate's underlying reader
    /// position mutates per call. For parallel reads across many
    /// entries, see [`PakFile::extract_parallel`].
    ///
    /// # Errors
    ///
    /// Returns [`PakError::NotFound`] when no entry matches `name`, and
    /// otherwise propagates [`PakArchive::read_by_index`]: [`PakError::Zip`]
    /// or [`PakError::Io`] when the raw entry bytes cannot be read,
    /// [`PakError::EntryTooLarge`] when a recorded size exceeds this target's
    /// address space, and [`PakError::Decompress`] when the payload fails to
    /// decode.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, PakError> {
        let idx = self
            .metadata
            .iter()
            .position(|m| m.name == name)
            .ok_or_else(|| PakError::NotFound(name.to_string()))?;
        self.read_by_index(idx)
    }

    /// Read + decompress a named entry into a caller-supplied
    /// buffer (cleared first). Returns the resulting length.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::NotFound`] when no entry matches `name`, and
    /// otherwise propagates [`PakArchive::read_by_index_into`]:
    /// [`PakError::Zip`] or [`PakError::Io`] when the raw entry bytes cannot
    /// be read, [`PakError::EntryTooLarge`] when a recorded size exceeds this
    /// target's address space, and [`PakError::Decompress`] when the payload
    /// fails to decode.
    pub fn read_into(&mut self, name: &str, buf: &mut Vec<u8>) -> Result<usize, PakError> {
        let idx = self
            .metadata
            .iter()
            .position(|m| m.name == name)
            .ok_or_else(|| PakError::NotFound(name.to_string()))?;
        self.read_by_index_into(idx, buf)
    }

    /// Read + decompress an entry by index.
    ///
    /// Uses the same raw-read + `decompress_bytes_into` path as
    /// [`PakFile::extract_parallel`] — `by_index_raw` gives us the
    /// compressed bytes without zip's compression-method
    /// validation gate, which rejects the optional Oodle method 15 extension.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::IndexOutOfBounds`] when `idx` is past the entry
    /// table, [`PakError::Zip`] when the local file header cannot be read,
    /// [`PakError::Io`] when the compressed bytes are truncated,
    /// [`PakError::EntryTooLarge`] when a recorded size exceeds this target's
    /// address space, and [`PakError::Decompress`] when the payload fails to
    /// decode.
    pub fn read_by_index(&mut self, idx: usize) -> Result<Vec<u8>, PakError> {
        let meta = self
            .metadata
            .get(idx)
            .ok_or(PakError::IndexOutOfBounds(idx))?
            .clone();
        let mut out = Vec::with_capacity(meta.uncompressed_len()?);
        self.read_by_index_into_inner(&meta, &mut out)?;
        Ok(out)
    }

    /// Read + decompress an entry by index into a caller-supplied
    /// buffer (cleared first). Returns the resulting length.
    ///
    /// # Errors
    ///
    /// Returns [`PakError::IndexOutOfBounds`] when `idx` is past the entry
    /// table, [`PakError::Zip`] when the local file header cannot be read,
    /// [`PakError::Io`] when the compressed bytes are truncated,
    /// [`PakError::EntryTooLarge`] when a recorded size exceeds this target's
    /// address space, and [`PakError::Decompress`] when the payload fails to
    /// decode.
    pub fn read_by_index_into(&mut self, idx: usize, buf: &mut Vec<u8>) -> Result<usize, PakError> {
        let meta = self
            .metadata
            .get(idx)
            .ok_or(PakError::IndexOutOfBounds(idx))?
            .clone();
        self.read_by_index_into_inner(&meta, buf)?;
        Ok(buf.len())
    }

    /// Internal: do the raw read + decompress for a metadata entry.
    fn read_by_index_into_inner(
        &mut self,
        meta: &EntryMetadata,
        buf: &mut Vec<u8>,
    ) -> Result<(), PakError> {
        // `by_index_raw` returns a ZipFile that yields the
        // **compressed** bytes verbatim, with no decompression
        // attempted — so zip's "method not supported" check
        // doesn't fire for our Oodle entries.
        let mut entry = self.inner.by_index_raw(meta.index)?;
        let mut compressed = Vec::with_capacity(meta.compressed_len()?);
        entry.read_to_end(&mut compressed)?;
        decompress_bytes_into(meta.compression, &compressed, meta.uncompressed_len()?, buf)?;
        Ok(())
    }
}

impl PakFile {
    /// Read + decompress many named entries in parallel.
    ///
    /// **Architecture (zero per-worker syscalls):**
    /// 1. mmap the whole pak file once on the calling thread.
    /// 2. Parse the central directory once (single `ZipArchive::new`
    ///    over the mmap'd bytes) to capture each entry's
    ///    `(header_offset, compressed_size, compression)`.
    /// 3. Each rayon worker receives a `&[u8]` slice into the
    ///    mmap — *no* file handle, *no* seek state, *no* syscalls.
    ///    The worker:
    ///    - reads the 30-byte LFH directly from the slice +
    ///      validates the `PK\x03\x04` signature
    ///    - slices out `compressed_size` bytes at `data_start`
    ///    - hands them to [`decompress_bytes_into`] (same vetted
    ///      `flate2` / `oodle-safe` / AZCS path as the single-
    ///      thread reads)
    ///
    /// vs. the per-worker `File::open` approach this also saves:
    /// - per-worker file-descriptor allocation
    /// - per-entry seek+read syscalls (replaced by direct slice
    ///   indexing into the kernel-paged mmap)
    ///
    /// On warm cache the inner read+decompress loop is essentially
    /// memcpy-bound rather than syscall-bound.
    ///
    /// **Safety:** offsets and sizes come from the zip crate's own
    /// vetted central-directory parse done in step 2. The mmap
    /// stays alive for the entire `par_iter` so worker slices
    /// remain valid. We validate the LFH signature on every read
    /// before trusting the variable-length fields.
    pub fn extract_parallel(
        path: impl AsRef<Path>,
        names: &[&str],
    ) -> Vec<Result<Vec<u8>, PakError>> {
        match extract_parallel_inner(path.as_ref(), names) {
            Ok(results) => results,
            Err(e) => names.iter().map(|_| Err(clone_error(&e))).collect(),
        }
    }
}

fn extract_parallel_inner(
    path: &Path,
    names: &[&str],
) -> Result<Vec<Result<Vec<u8>, PakError>>, PakError> {
    // Step 1: mmap the file. SAFETY: the caller-facing operation borrows the
    // mapping only for this read and requires the archive to remain immutable,
    // matching `PakFileMmap::open_mmap`.
    let file = File::open(path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let bytes: &[u8] = &mmap;

    // Step 2: parse CD once.
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let metadata = collect_metadata(&mut archive)?;
    drop(archive);

    // Resolve user-supplied names -> metadata snapshot.
    let resolved: Vec<Result<EntryMetadata, PakError>> = names
        .iter()
        .map(|&name| {
            metadata
                .iter()
                .find(|m| m.name == name)
                .cloned()
                .ok_or_else(|| PakError::NotFound(name.to_string()))
        })
        .collect();

    // Step 3: parallel decompress, workers index directly into
    // the mmap. The closure captures `bytes` (a `&[u8]` borrowed
    // from `mmap`); the mmap stays alive on this stack frame for
    // the duration of the par_iter.
    let results: Vec<Result<Vec<u8>, PakError>> = resolved
        .into_par_iter()
        .map(|entry_result| {
            let entry = entry_result?;
            decompress_entry_from_slice(bytes, &entry)
        })
        .collect();

    // Explicit drop to make the lifetime relationship loud.
    drop(mmap);
    Ok(results)
}

/// Locate + decompress one entry directly from a mmap'd byte
/// slice. Pure memory access — no syscalls in this function.
fn decompress_entry_from_slice(bytes: &[u8], entry: &EntryMetadata) -> Result<Vec<u8>, PakError> {
    // A header offset past `usize` can't be addressed in this slice at all,
    // which is the same failure the `bytes.get` below reports.
    let header_off = usize::try_from(entry.header_offset).map_err(|_| PakError::CorruptHeader {
        entry: entry.name.clone(),
        offset: entry.header_offset,
    })?;
    let lfh = bytes
        .get(header_off..header_off + LFH_FIXED_LEN)
        .ok_or_else(|| PakError::CorruptHeader {
            entry: entry.name.clone(),
            offset: entry.header_offset,
        })?;

    if lfh[0..4] != LFH_SIGNATURE {
        return Err(PakError::CorruptHeader {
            entry: entry.name.clone(),
            offset: entry.header_offset,
        });
    }

    let name_len = u16::from_le_bytes([lfh[26], lfh[27]]) as usize;
    let extra_len = u16::from_le_bytes([lfh[28], lfh[29]]) as usize;
    let data_start = header_off + LFH_FIXED_LEN + name_len + extra_len;
    let data_end = data_start
        .checked_add(entry.compressed_len()?)
        .ok_or_else(|| PakError::CorruptHeader {
            entry: entry.name.clone(),
            offset: entry.header_offset,
        })?;

    let compressed = bytes
        .get(data_start..data_end)
        .ok_or_else(|| PakError::CorruptHeader {
            entry: entry.name.clone(),
            offset: entry.header_offset,
        })?;

    let uncompressed_len = entry.uncompressed_len()?;
    let mut out = Vec::with_capacity(uncompressed_len);
    decompress_bytes_into(entry.compression, compressed, uncompressed_len, &mut out)?;
    Ok(out)
}

/// Best-effort clone of a `PakError` for fan-out reporting.
/// `io::Error` isn't `Clone`, so we project to a string-cloned
/// equivalent. Loses the original `kind()`, but only used when
/// the open itself failed — every entry in the result vec gets
/// the same surface error.
fn clone_error(err: &PakError) -> PakError {
    PakError::Io(io::Error::other(err.to_string()))
}

// === entry metadata ===

#[derive(Debug, Clone)]
struct EntryMetadata {
    name: String,
    index: usize,
    uncompressed_size: u64,
    compressed_size: u64,
    compression: Compression,
    /// Byte offset of this entry's Local File Header in the
    /// underlying file. `data_start = header_offset + 30 +
    /// LFH.name_len + LFH.extra_len`.
    header_offset: u64,
}

impl EntryMetadata {
    #[inline]
    fn view(&self) -> EntryInfo<'_> {
        EntryInfo {
            name: &self.name,
            index: self.index,
            uncompressed_size: self.uncompressed_size,
            compressed_size: self.compressed_size,
            compression: self.compression,
        }
    }

    /// The central-directory uncompressed size as a `usize`.
    ///
    /// The value feeds both the output allocation and Oodle's pre-sized
    /// destination buffer, so a truncating cast would silently produce a
    /// short read. Infallible on 64-bit targets.
    #[inline]
    fn uncompressed_len(&self) -> Result<usize, PakError> {
        usize::try_from(self.uncompressed_size).map_err(|_| PakError::EntryTooLarge {
            entry: self.name.clone(),
            size: self.uncompressed_size,
        })
    }

    /// The central-directory compressed size as a `usize`.
    ///
    /// Used to bound the compressed payload slice, so a truncating cast
    /// would hand the decoder the wrong byte range. Infallible on 64-bit
    /// targets.
    #[inline]
    fn compressed_len(&self) -> Result<usize, PakError> {
        usize::try_from(self.compressed_size).map_err(|_| PakError::EntryTooLarge {
            entry: self.name.clone(),
            size: self.compressed_size,
        })
    }
}

fn collect_metadata<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<Vec<EntryMetadata>, PakError> {
    let len = zip.len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let entry = zip.by_index_raw(i)?;
        out.push(EntryMetadata {
            name: entry.name()?.into_owned(),
            index: i,
            uncompressed_size: entry.size(),
            compressed_size: entry.compressed_size(),
            compression: Compression::from_zip_method(entry.compression()),
            // header_start() comes from the central-directory record
            // — no extra I/O required to read it.
            header_offset: entry.header_start(),
        });
    }
    Ok(out)
}

fn collect_selected_metadata<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    selected: &HashSet<&str>,
) -> Result<Vec<EntryMetadata>, PakError> {
    let mut out = Vec::with_capacity(selected.len());
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i)?;
        let name = entry.name()?.into_owned();
        if !selected.contains(name.as_str()) {
            continue;
        }
        out.push(EntryMetadata {
            name,
            index: i,
            uncompressed_size: entry.size(),
            compressed_size: entry.compressed_size(),
            compression: Compression::from_zip_method(entry.compression()),
            header_offset: entry.header_start(),
        });
    }
    Ok(out)
}

/// Borrowed view into a single pak entry's metadata.
///
/// Cheap to copy; doesn't hold any open zip handle.
#[derive(Debug, Clone, Copy)]
pub struct EntryInfo<'a> {
    name: &'a str,
    index: usize,
    uncompressed_size: u64,
    compressed_size: u64,
    compression: Compression,
}

impl<'a> EntryInfo<'a> {
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &'a str {
        self.name
    }

    #[inline]
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[inline]
    #[must_use]
    pub const fn uncompressed_size(&self) -> u64 {
        self.uncompressed_size
    }

    #[inline]
    #[must_use]
    pub const fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    #[inline]
    #[must_use]
    pub const fn compression(&self) -> Compression {
        self.compression
    }
}

/// Iterator yielded by [`PakArchive::entries`].
pub struct EntryIter<'a> {
    inner: std::slice::Iter<'a, EntryMetadata>,
}

impl<'a> Iterator for EntryIter<'a> {
    type Item = EntryInfo<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(EntryMetadata::view)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for EntryIter<'_> {}
impl DoubleEndedIterator for EntryIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(EntryMetadata::view)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn selected_mmap_reader_retains_only_requested_entries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "az-pak-selected-reader-{}-{nonce}.zip",
            std::process::id()
        ));
        let file = File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("ignored.txt", options).unwrap();
        writer.write_all(b"ignored").unwrap();
        writer.start_file("selected.bin", options).unwrap();
        writer.write_all(b"selected").unwrap();
        writer.finish().unwrap();

        let reader = PakMmapReader::open_selected(&path, ["selected.bin"]).unwrap();
        assert_eq!(reader.len(), 1);
        assert!(reader.entry("ignored.txt").is_none());
        assert_eq!(reader.read("selected.bin").unwrap(), b"selected");

        let Err(missing) = PakMmapReader::open_selected(&path, ["missing.bin"]) else {
            panic!("missing selected entry unexpectedly opened")
        };
        assert!(matches!(missing, PakError::NotFound(name) if name == "missing.bin"));
        std::fs::remove_file(path).unwrap();
    }
}
