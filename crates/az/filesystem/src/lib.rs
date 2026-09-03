//! Shared filesystem and virtual asset-path handling.
//!
//! Azoth content moves through three path domains:
//!
//! - local filesystem paths (`Path` / `PathBuf`)
//! - pak entry paths, which are always virtual relative paths
//! - normalized source asset paths, which use lowercase forward slashes
//!
//! Keeping those rules here prevents every importer/tool from
//! reimplementing normalization slightly differently.

use std::borrow::{Borrow, Cow};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Write;
use std::io::{Read, Write as IoWrite};
use std::ops::Deref;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use thiserror::Error;

mod data_home;
mod host_tool;

pub use data_home::*;
pub use host_tool::*;

static FILE_TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(1);
static FILE_TRANSACTION_PROCESS_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<ProcessLockState>>>> =
    OnceLock::new();

/// Stable identity for one filesystem object within its filesystem.
///
/// Paths, lengths, and timestamps do not identify a publication: atomic save
/// replaces one object with another at the same path, and coarse timestamp
/// resolution can make those states otherwise indistinguishable. The pair is
/// only meaningful as an equality token; its platform-specific numbers have
/// no portable ordering or lifetime guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub file_system_id: u64,
    pub file_id: u64,
}

/// Read the platform identity corresponding to `metadata` at `path`.
///
/// Returns `None` on platforms without a stable object identity or when the
/// object cannot be opened long enough to query it. Callers using identity for
/// cache correctness must disable that cache when this returns `None`.
#[cfg(unix)]
#[must_use]
pub fn file_identity(_path: &Path, metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(FileIdentity {
        file_system_id: metadata.dev(),
        file_id: metadata.ino(),
    })
}

#[cfg(windows)]
#[must_use]
pub fn file_identity(path: &Path, _metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    use std::os::windows::io::AsRawHandle as _;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    // `std::os::windows::fs::MetadataExt::{volume_serial_number, file_index}`
    // are still unstable, so read the identical fields straight off the Win32
    // structure that backs them.
    let file = std::fs::File::open(path).ok()?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle stays owned by `file` for the duration of the call and
    // `information` is a live, correctly sized output buffer.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &raw mut information) }
        .ok()?;

    Some(FileIdentity {
        file_system_id: u64::from(information.dwVolumeSerialNumber),
        file_id: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn file_identity(_path: &Path, _metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    None
}

/// Normalized virtual source asset path.
///
/// Source paths are virtual content paths, not native filesystem
/// paths. They are lowercase, slash-separated, and relative.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePath(String);

impl SourcePath {
    #[must_use]
    pub fn new(path: impl AsRef<str>) -> Self {
        Self(normalize_source_path(path.as_ref()))
    }

    #[must_use]
    pub fn non_empty(path: &str) -> Option<Self> {
        let path = path.trim();
        (!path.is_empty()).then(|| Self::new(path))
    }

    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    #[must_use]
    pub fn extension_key(&self) -> Option<String> {
        source_extension_key(self.as_str())
    }
}

impl fmt::Display for SourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for SourcePath {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for SourcePath {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Borrow<str> for SourcePath {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for SourcePath {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<SourcePath> for &str {
    fn eq(&self, other: &SourcePath) -> bool {
        *self == other.as_str()
    }
}

impl From<&str> for SourcePath {
    #[inline]
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<String> for SourcePath {
    #[inline]
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

/// Normalize a virtual source asset path.
///
/// This is *the* canonicalization for the asset-path domain. Every asset-path
/// key in the engine — [`SourcePath`], `az_asset::AssetTreePath`, catalog and
/// pack lookups, builder patterns — is produced by this function, so a path has
/// exactly one byte spelling everywhere it is stored, hashed, or compared. See
/// `az_asset::AssetTreePath` for the invariant that canonical form carries.
///
/// The rule matches the path normalization the Lumberyard/O3DE catalog applies
/// before hashing a path into an `AssetId` with `AZ::Uuid::CreateName`:
///
/// 1. convert `\` separators to `/`
/// 2. trim surrounding whitespace
/// 3. fold ASCII case to lowercase
/// 4. strip leading `./` segments
/// 5. drop leading and trailing slashes
/// 6. collapse repeated internal slashes
///
/// Two spellings that differ only in case, separator flavor, or redundant and
/// trailing separators therefore normalize identically and hash to the same
/// GUID. The transform is idempotent: its own output is a fixed point, which is
/// what makes it safe for lookup surfaces to re-normalize an already-canonical
/// argument.
#[must_use]
pub fn normalize_source_path(path: impl AsRef<str>) -> String {
    let mut normalized = path.as_ref().replace('\\', "/").trim().to_ascii_lowercase();
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    let trimmed = normalized.trim_matches('/');
    // Collapse repeated internal slashes so `a//b` and `a/b` are identical.
    let mut collapsed = String::with_capacity(trimmed.len());
    let mut previous_was_slash = false;
    for ch in trimmed.chars() {
        if ch == '/' {
            if previous_was_slash {
                continue;
            }
            previous_was_slash = true;
        } else {
            previous_was_slash = false;
        }
        collapsed.push(ch);
    }
    collapsed
}

/// Return the final file extension key used by generic asset routing.
///
/// Project or compatibility formats that need compound extensions or
/// grouped split-file suffixes should wrap this with their own classifier
/// and then call [`engine_path_with_extension_key`] when constructing
/// products.
#[must_use]
pub fn source_extension_key(path: &str) -> Option<String> {
    let normalized = normalize_source_path(path);
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);

    file_name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty())
        .map(str::to_string)
}

/// Resolve a product path from a source path and desired output extension.
///
/// If the requested extension is the source's own normalized extension, the
/// source identity path is preserved. Synthetic/derived products are placed
/// under `prefix` and replace the source extension with `extension`.
#[must_use]
pub fn engine_path(source_path: &str, prefix: &str, extension: &str) -> String {
    let source_path = normalize_source_path(source_path);
    let source_extension_key = source_extension_key(&source_path);
    engine_path_from_normalized_source(
        &source_path,
        prefix,
        extension,
        source_extension_key.as_deref(),
    )
}

/// Resolve a product path with a caller-supplied source extension key.
///
/// This is for compatibility adapters that classify source files with
/// compound keys such as `loc.xml` or grouped split keys. The shared engine
/// crate stays format-agnostic while those adapters can still strip their
/// own source suffix correctly.
#[must_use]
pub fn engine_path_with_extension_key(
    source_path: &str,
    prefix: &str,
    extension: &str,
    source_extension_key: Option<&str>,
) -> String {
    let source_path = normalize_source_path(source_path);
    engine_path_from_normalized_source(&source_path, prefix, extension, source_extension_key)
}

fn engine_path_from_normalized_source(
    source_path: &str,
    prefix: &str,
    extension: &str,
    source_extension_key: Option<&str>,
) -> String {
    let extension = extension.trim_start_matches('.').to_ascii_lowercase();
    if source_extension_key == Some(extension.as_str()) {
        return source_path.to_string();
    }

    let stem =
        source_stem_without_extension_key(source_path, source_extension_key).unwrap_or(source_path);
    let prefix = normalize_source_path(prefix)
        .trim_end_matches('/')
        .to_string();
    if prefix.is_empty() {
        format!("{stem}.{extension}")
    } else {
        format!("{prefix}/{stem}.{extension}")
    }
}

fn source_stem_without_extension_key<'a>(
    path: &'a str,
    extension_key: Option<&str>,
) -> Option<&'a str> {
    let extension = extension_key?;
    let suffix = format!(".{extension}");
    path.strip_suffix(&suffix)
}

#[must_use]
pub fn material_source_asset_path(path: &str) -> Option<String> {
    let mut path = SourcePath::non_empty(path)?.into_string();
    if !Path::new(&path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mtl"))
    {
        path.push_str(".mtl");
    }
    Some(path)
}

#[must_use]
pub fn texture_source_asset_path(path: &str) -> String {
    let path = normalize_source_path(path);
    path.strip_suffix(".tif")
        .or_else(|| path.strip_suffix(".tiff"))
        .map(|stem| format!("{stem}.dds"))
        .unwrap_or(path)
}

/// Region metadata parsed from a path containing `regions/r_+XX_+YY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionPathMeta {
    /// Level or world folder directly above `regions`.
    pub level: String,
    /// Region X coordinate.
    pub x: i32,
    /// Region Y coordinate.
    pub y: i32,
}

impl RegionPathMeta {
    /// Parse region coordinates from a normalized or Windows-style path.
    #[must_use]
    pub fn parse(path: impl AsRef<str>) -> Option<Self> {
        let path = path.as_ref();
        let normalized = if path.contains('\\') {
            Cow::Owned(path.replace('\\', "/"))
        } else {
            Cow::Borrowed(path)
        };

        let mut previous_level: Option<&str> = None;
        let mut previous: Option<&str> = None;
        for part in normalized.split('/').filter(|part| !part.is_empty()) {
            if previous == Some("regions") {
                let (x, y) = parse_region_segment(part)?;
                return Some(Self {
                    level: previous_level?.to_string(),
                    x,
                    y,
                });
            }
            previous_level = previous;
            previous = Some(part);
        }
        None
    }
}

/// Display `path` relative to `root` when possible, using forward
/// slashes so output matches pak/source paths on Windows too.
#[must_use]
pub fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[derive(Debug, Error)]
pub enum SafeJoinError {
    #[error("archive path is empty")]
    EmptyArchivePath,

    #[error("archive path must be relative: {0}")]
    AbsoluteArchivePath(String),

    #[error("archive path contains a parent-directory component: {0}")]
    ParentDir(String),

    #[error("archive path escapes root {root:?} to {path:?}")]
    EscapesRoot { root: PathBuf, path: PathBuf },
}

/// Join a pak/archive entry path under `dest_root` without allowing
/// zip-slip traversal outside the destination.
///
/// # Errors
///
/// Returns [`SafeJoinError`] when the entry path is empty, absolute, contains
/// `..`, or resolves outside `dest_root`.
pub fn safe_join(dest_root: &Path, entry_name: &str) -> Result<PathBuf, SafeJoinError> {
    if entry_name.trim().is_empty() {
        return Err(SafeJoinError::EmptyArchivePath);
    }
    if is_archive_path_absolute(entry_name) {
        return Err(SafeJoinError::AbsoluteArchivePath(entry_name.to_string()));
    }

    let root = normalize_lexical(dest_root);
    let mut candidate = root.clone();
    let mut saw_component = false;

    for component in entry_name.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => return Err(SafeJoinError::ParentDir(entry_name.to_string())),
            part => {
                saw_component = true;
                candidate.push(part);
            }
        }
    }

    if !saw_component {
        return Err(SafeJoinError::EmptyArchivePath);
    }

    let normalized = normalize_lexical(&candidate);
    if !normalized.starts_with(&root) {
        return Err(SafeJoinError::EscapesRoot {
            root,
            path: normalized,
        });
    }
    Ok(normalized)
}

/// Lexically normalize a local filesystem path without touching the
/// filesystem. This resolves `.` and `..` segments for paths that may
/// not exist yet.
#[must_use]
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            other => stack.push(other),
        }
    }
    stack.iter().collect()
}

/// Canonicalize an existing path into the one shape Azoth stores.
///
/// This is the single normalization boundary for paths that outlive the call
/// producing them: workspace roots, asset source roots, project data roots,
/// service records, and anything else another process later compares. Two
/// spellings of the same directory converge here — symlinks resolved, real
/// on-disk case applied — so a root the daemon recorded still matches the root
/// the asset processor resolved.
///
/// Windows [`std::fs::canonicalize`] alone is not enough: it returns a
/// verbatim `\\?\` path, which most programs reject as an argument, which
/// refuses forward slashes, and which compares unequal to the same directory
/// named any other way. [`dunce`] downgrades those to the legacy form only
/// where that is provably lossless, keeping verbatim for the cases where it is
/// not — over 260 characters, reserved DOS device names, components ending in
/// a dot or space, network shares, and non-Unicode names. A hand-rolled prefix
/// strip gets every one of those wrong.
///
/// # Errors
///
/// Returns the [`std::io::Error`] from canonicalization; use [`normalize`] for
/// paths that need not exist yet.
#[inline]
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Best-effort [`canonical`] for a path that need not exist on disk yet.
///
/// Paths that cannot be canonicalized still come back absolute and lexically
/// normalized, in the same legacy-compatible shape, so a root recorded before
/// its directory exists still matches once it does. [`normalize_lexical`] is
/// the no-filesystem sibling for callers that must not touch the disk at all.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    if let Ok(canonical) = canonical(path) {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        Cow::Borrowed(path)
    } else {
        std::env::current_dir().map_or(Cow::Borrowed(path), |current| {
            Cow::Owned(current.join(path))
        })
    };
    normalize_lexical(dunce::simplified(&absolute))
}

/// Widen a path for one syscall when its legacy form would hit `MAX_PATH`.
///
/// [`canonical`] deliberately stores the legacy form, because that is what
/// compares equal and what other programs accept as an argument. Win32 caps
/// that form at 260 characters (248 for directory creation), and Azoth nests
/// well past it — product caches and transaction staging descend a dozen
/// components below a project root the user chose. The verbatim prefix lifts
/// the cap, so it goes on exactly where a path stops being data and becomes a
/// syscall argument, and never reaches anything Azoth stores, compares, logs,
/// or hands to a child process.
#[cfg(windows)]
fn wide(path: &Path) -> Cow<'_, Path> {
    use std::ffi::OsString;
    use std::path::Prefix;

    // Widen slightly below the directory limit so creating the parent of a
    // long file path is covered by the same threshold.
    const LIMIT: usize = 240;

    if path.as_os_str().len() < LIMIT {
        return Cow::Borrowed(path);
    }

    // The verbatim namespace skips path parsing entirely: it takes `.`, `..`
    // and `/` literally. Only a rooted drive path converts faithfully — a UNC
    // share needs the `\\?\UNC\` spelling, which is not a concatenation — so
    // anything else keeps the legacy form and its limit.
    let mut components = path.components();
    let rooted = matches!(
        components.next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    ) && matches!(components.next(), Some(Component::RootDir));
    if !rooted || !components.all(|component| matches!(component, Component::Normal(_))) {
        return Cow::Borrowed(path);
    }

    // Rebuilding from components rewrites any `/` separator as `\`, which the
    // verbatim namespace requires, and keeps non-Unicode names intact.
    let normalized = path.components().collect::<PathBuf>();
    let mut widened = OsString::with_capacity(normalized.as_os_str().len() + 4);
    widened.push(r"\\?\");
    widened.push(&normalized);
    Cow::Owned(PathBuf::from(widened))
}

#[cfg(not(windows))]
fn wide(path: &Path) -> Cow<'_, Path> {
    Cow::Borrowed(path)
}

/// One file written by [`FileTransaction::commit`].
///
/// [`Self::new`] keeps the original in-memory byte API. [`Self::from_path`] and
/// [`Self::from_reader`] defer source I/O until the transaction has acquired
/// its lock, then copy directly into the transaction staging file. This keeps
/// large products out of the caller's batch allocation.
pub struct FileWrite {
    pub target: PathBuf,
    source: FileWriteSource,
}

enum FileWriteSource {
    Bytes(Vec<u8>),
    Path(PathBuf),
    Reader {
        label: PathBuf,
        reader: Box<dyn Read + Send>,
    },
}

impl fmt::Debug for FileWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileWrite")
            .field("target", &self.target)
            .field("source", &self.source)
            .finish()
    }
}

impl fmt::Debug for FileWriteSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => formatter
                .debug_struct("Bytes")
                .field("len", &bytes.len())
                .finish(),
            Self::Path(path) => formatter.debug_tuple("Path").field(path).finish(),
            Self::Reader { label, .. } => formatter.debug_tuple("Reader").field(label).finish(),
        }
    }
}

impl FileWrite {
    /// Construct an in-memory file write.
    ///
    /// This is the compatibility constructor for callers that already own the
    /// complete product bytes.
    #[must_use]
    pub fn new(target: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            target: target.into(),
            source: FileWriteSource::Bytes(bytes.into()),
        }
    }

    /// Construct a file write whose source is copied during commit.
    ///
    /// The source is opened only after the transaction lock is held. A source
    /// read or open failure is returned as [`FileTransactionError::ReadSource`].
    #[must_use]
    pub fn from_path(target: impl Into<PathBuf>, source: impl Into<PathBuf>) -> Self {
        Self {
            target: target.into(),
            source: FileWriteSource::Path(source.into()),
        }
    }

    /// Construct a file write from a stream copied during commit.
    ///
    /// The reader is consumed directly into transaction staging, so a batch of
    /// large products does not need to retain their complete contents in
    /// memory. The reader must be owned because commit consumes each write.
    #[must_use]
    pub fn from_reader<R>(target: impl Into<PathBuf>, reader: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self {
            target: target.into(),
            source: FileWriteSource::Reader {
                label: PathBuf::from("<stream>"),
                reader: Box::new(reader),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransactionReceipt {
    pub id: String,
    pub write_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecoveryReport {
    pub id: String,
    pub recovered_write_count: usize,
}

#[derive(Debug, Clone)]
pub struct FileTransaction {
    root: PathBuf,
}

impl FileTransaction {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// # Errors
    ///
    /// Returns [`FileTransactionError`] when staging, backup, marker writes, or
    /// final apply/rename steps fail.
    pub fn commit(
        &self,
        writes: impl IntoIterator<Item = FileWrite>,
    ) -> Result<FileTransactionReceipt, FileTransactionError> {
        let writes = writes.into_iter().collect::<Vec<_>>();
        if writes.is_empty() {
            return Err(FileTransactionError::EmptyTransaction);
        }

        let _lock = acquire_file_transaction_lock(&self.root, true)?;
        let id = next_file_transaction_id();
        let layout = TransactionLayout::new(&self.root, &id);
        let mut unprepared_guard = UnpreparedTransactionGuard::new(&layout);
        std::fs::create_dir_all(&*wide(&layout.staging_dir)).map_err(|source| {
            FileTransactionError::CreateDir {
                path: layout.staging_dir.clone(),
                source,
            }
        })?;
        std::fs::create_dir_all(&*wide(&layout.backup_dir)).map_err(|source| {
            FileTransactionError::CreateDir {
                path: layout.backup_dir.clone(),
                source,
            }
        })?;

        let mut entries = Vec::with_capacity(writes.len());
        for (index, write) in writes.into_iter().enumerate() {
            let FileWrite { target, source } = write;
            if target.as_os_str().is_empty() {
                return Err(FileTransactionError::EmptyTarget);
            }
            let staged = layout.staging_path(index);
            stage_file_sync(source, &staged)?;
            entries.push(TransactionEntry {
                target,
                staged,
                backup: layout.backup_path(index),
            });
        }

        sync_directory_for_transaction(&layout.staging_dir)?;
        write_marker(&layout.marker, MarkerStatus::Prepared, &entries)?;
        unprepared_guard.disarm();
        sync_directory_for_transaction(&layout.dir)?;
        sync_directory_for_transaction(&self.root)?;
        write_marker(&layout.marker, MarkerStatus::Applying, &entries)?;
        apply_entries_forward(&entries)?;
        write_marker(&layout.marker, MarkerStatus::Committed, &entries)?;
        remove_dir_all_if_exists(&layout.dir)?;

        Ok(FileTransactionReceipt {
            id,
            write_count: entries.len(),
        })
    }

    /// # Errors
    ///
    /// Returns [`FileTransactionError`] when pending transaction directories
    /// cannot be read or recovered.
    pub fn recover_pending(&self) -> Result<Vec<FileRecoveryReport>, FileTransactionError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let _lock = acquire_file_transaction_lock(&self.root, false)?;
        let mut reports = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(reports),
            Err(source) => {
                return Err(FileTransactionError::ReadDir {
                    path: self.root.clone(),
                    source,
                });
            }
        };

        for entry in entries {
            let entry = entry.map_err(|source| FileTransactionError::ReadDir {
                path: self.root.clone(),
                source,
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let marker = path.join(FILE_TRANSACTION_MARKER);
            if !marker.is_file() {
                continue;
            }

            let parsed = read_marker(&marker)?;
            let recovered_write_count = match parsed.status {
                MarkerStatus::Prepared => 0,
                MarkerStatus::Applying | MarkerStatus::Committed => {
                    apply_entries_forward(&parsed.entries)?;
                    parsed.entries.len()
                }
            };
            remove_dir_all_if_exists(&path)?;
            reports.push(FileRecoveryReport {
                id: parsed.id,
                recovered_write_count,
            });
        }

        Ok(reports)
    }
}

const FILE_TRANSACTION_MARKER: &str = "transaction.txt";
const FILE_TRANSACTION_LOCK_SUFFIX: &str = ".lock";

struct ProcessLockState {
    held: Mutex<bool>,
    released: Condvar,
}

struct ProcessRootGuard {
    state: Arc<ProcessLockState>,
}

impl ProcessRootGuard {
    fn acquire(state: Arc<ProcessLockState>) -> Self {
        let mut held = state
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *held {
            held = state
                .released
                .wait(held)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *held = true;
        drop(held);
        Self { state }
    }
}

impl Drop for ProcessRootGuard {
    fn drop(&mut self) {
        let mut held = self
            .state
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *held = false;
        drop(held);
        self.state.released.notify_one();
    }
}

struct FileTransactionLock {
    file: std::fs::File,
    _process_guard: ProcessRootGuard,
}

impl Drop for FileTransactionLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn acquire_file_transaction_lock(
    root: &Path,
    create_root: bool,
) -> Result<FileTransactionLock, FileTransactionError> {
    if create_root {
        std::fs::create_dir_all(&*wide(root)).map_err(|source| {
            FileTransactionError::CreateDir {
                path: root.to_path_buf(),
                source,
            }
        })?;
        sync_parent_directory_for_transaction(root)?;
    }
    let path = transaction_lock_path(root)?;
    let process_guard = ProcessRootGuard::acquire(process_lock_state(&path));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&*wide(&path))
        .map_err(|source| FileTransactionError::Lock {
            path: path.clone(),
            source,
        })?;
    file.lock_exclusive()
        .map_err(|source| FileTransactionError::Lock { path, source })?;
    Ok(FileTransactionLock {
        file,
        _process_guard: process_guard,
    })
}

fn process_lock_state(path: &Path) -> Arc<ProcessLockState> {
    let locks = FILE_TRANSACTION_PROCESS_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, state| state.strong_count() > 0);
    let key = normalize_lexical(path);
    if let Some(state) = locks.get(&key).and_then(Weak::upgrade) {
        return state;
    }
    let state = Arc::new(ProcessLockState {
        held: Mutex::new(false),
        released: Condvar::new(),
    });
    locks.insert(key, Arc::downgrade(&state));
    state
}

fn transaction_lock_path(root: &Path) -> Result<PathBuf, FileTransactionError> {
    let Some(input_name) = root.file_name().filter(|name| !name.is_empty()) else {
        return Err(FileTransactionError::InvalidRoot {
            path: root.to_path_buf(),
            reason: "transaction root must have a file name".to_string(),
        });
    };
    let input_parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let (parent, name) = match std::fs::canonicalize(root) {
        Ok(canonical_root) => {
            let Some(name) = canonical_root.file_name().filter(|name| !name.is_empty()) else {
                return Err(FileTransactionError::InvalidRoot {
                    path: root.to_path_buf(),
                    reason: "canonical transaction root must have a file name".to_string(),
                });
            };
            let parent = canonical_root
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            (parent.to_path_buf(), name.to_os_string())
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let parent = std::fs::canonicalize(input_parent).map_err(|source| {
                FileTransactionError::Lock {
                    path: input_parent.to_path_buf(),
                    source,
                }
            })?;
            (parent, input_name.to_os_string())
        }
        Err(source) => {
            return Err(FileTransactionError::Lock {
                path: root.to_path_buf(),
                source,
            });
        }
    };
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(name);
    lock_name.push(FILE_TRANSACTION_LOCK_SUFFIX);
    Ok(parent.join(lock_name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerStatus {
    Prepared,
    Applying,
    Committed,
}

impl MarkerStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Applying => "applying",
            Self::Committed => "committed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "prepared" => Some(Self::Prepared),
            "applying" => Some(Self::Applying),
            "committed" => Some(Self::Committed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransactionEntry {
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
}

#[derive(Debug, Clone)]
struct ParsedMarker {
    id: String,
    status: MarkerStatus,
    entries: Vec<TransactionEntry>,
}

#[derive(Debug, Clone)]
struct TransactionLayout {
    dir: PathBuf,
    staging_dir: PathBuf,
    backup_dir: PathBuf,
    marker: PathBuf,
}

impl TransactionLayout {
    fn new(root: &Path, id: &str) -> Self {
        let dir = root.join(id);
        Self {
            staging_dir: dir.join("staged"),
            backup_dir: dir.join("backup"),
            marker: dir.join(FILE_TRANSACTION_MARKER),
            dir,
        }
    }

    fn staging_path(&self, index: usize) -> PathBuf {
        self.staging_dir.join(format!("{index:04}.write"))
    }

    fn backup_path(&self, index: usize) -> PathBuf {
        self.backup_dir.join(format!("{index:04}.backup"))
    }
}

struct UnpreparedTransactionGuard {
    dir: PathBuf,
    marker: PathBuf,
    armed: bool,
}

impl UnpreparedTransactionGuard {
    fn new(layout: &TransactionLayout) -> Self {
        Self {
            dir: layout.dir.clone(),
            marker: layout.marker.clone(),
            armed: true,
        }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UnpreparedTransactionGuard {
    fn drop(&mut self) {
        if self.armed && !self.marker.is_file() {
            let _ = std::fs::remove_dir_all(&*wide(&self.dir));
        }
    }
}

#[derive(Debug, Error)]
pub enum FileTransactionError {
    #[error("file transaction must contain at least one write")]
    EmptyTransaction,

    #[error("file transaction target path cannot be empty")]
    EmptyTarget,

    #[error("invalid file transaction root {path}: {reason}")]
    InvalidRoot { path: PathBuf, reason: String },

    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to acquire file transaction lock {path}: {source}")]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read file transaction source {path}: {source}")]
    ReadSource {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to sync file transaction directory {path}: {source}")]
    SyncDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read file transaction root {path}: {source}")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read file transaction marker {path}: {source}")]
    ReadMarker {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse file transaction marker {path}: {reason}")]
    ParseMarker { path: PathBuf, reason: String },

    #[error("failed to rename {from} to {to}: {source}")]
    Rename {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to remove file {path}: {source}")]
    RemoveFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to remove directory {path}: {source}")]
    RemoveDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn next_file_transaction_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let counter = FILE_TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("tx-{millis}-{}-{counter}", std::process::id())
}

fn write_file_sync(path: &Path, bytes: &[u8]) -> Result<(), FileTransactionError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(&*wide(parent)).map_err(|source| {
            FileTransactionError::CreateDir {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let mut file =
        std::fs::File::create(&*wide(path)).map_err(|source| FileTransactionError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    IoWrite::write_all(&mut file, bytes).map_err(|source| FileTransactionError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all()
        .map_err(|source| FileTransactionError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn stage_file_sync(source: FileWriteSource, path: &Path) -> Result<(), FileTransactionError> {
    match source {
        FileWriteSource::Bytes(bytes) => write_file_sync(path, &bytes),
        FileWriteSource::Path(source) => {
            let mut reader = std::fs::File::open(&*wide(&source)).map_err(|source_error| {
                FileTransactionError::ReadSource {
                    path: source.clone(),
                    source: source_error,
                }
            })?;
            copy_reader_sync(&mut reader, &source, path)
        }
        FileWriteSource::Reader { label, mut reader } => {
            copy_reader_sync(reader.as_mut(), &label, path)
        }
    }
}

fn copy_reader_sync(
    reader: &mut dyn Read,
    source_path: &Path,
    destination: &Path,
) -> Result<(), FileTransactionError> {
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(&*wide(parent)).map_err(|source| {
            FileTransactionError::CreateDir {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let mut file = std::fs::File::create(&*wide(destination)).map_err(|source| {
        FileTransactionError::Write {
            path: destination.to_path_buf(),
            source,
        }
    })?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| FileTransactionError::ReadSource {
                path: source_path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        IoWrite::write_all(&mut file, &buffer[..read]).map_err(|source| {
            FileTransactionError::Write {
                path: destination.to_path_buf(),
                source,
            }
        })?;
    }
    file.sync_all()
        .map_err(|source| FileTransactionError::Write {
            path: destination.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn write_marker(
    path: &Path,
    status: MarkerStatus,
    entries: &[TransactionEntry],
) -> Result<(), FileTransactionError> {
    let mut text = String::new();
    let id = path
        .parent()
        .and_then(Path::file_name)
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    let _ = writeln!(text, "format=azoth.file-transaction/v1");
    let _ = writeln!(text, "id={id}");
    let _ = writeln!(text, "status={}", status.as_str());
    let _ = writeln!(text, "entries={}", entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let _ = writeln!(
            text,
            "entry.{index}.target={}",
            encode_marker_path(&entry.target)
        );
        let _ = writeln!(
            text,
            "entry.{index}.staged={}",
            encode_marker_path(&entry.staged)
        );
        let _ = writeln!(
            text,
            "entry.{index}.backup={}",
            encode_marker_path(&entry.backup)
        );
    }
    let temporary = marker_temporary_path(path);
    write_file_sync(&temporary, text.as_bytes())?;
    replace_file_atomic_and_sync(&temporary, path)
}

fn marker_temporary_path(path: &Path) -> PathBuf {
    let token = next_file_transaction_id();
    let name = path.file_name().map_or_else(
        || std::borrow::Cow::Borrowed("marker"),
        |value| value.to_string_lossy(),
    );
    path.with_file_name(format!(".{name}.tmp-{token}"))
}

fn replace_file_atomic_and_sync(from: &Path, to: &Path) -> Result<(), FileTransactionError> {
    atomic_replace_path(from, to).map_err(|source| FileTransactionError::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })?;
    sync_parent_directory(to).map_err(|source| FileTransactionError::SyncDir {
        path: to
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(&*wide(path))?.sync_all()
}

#[cfg(not(unix))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    sync_directory(parent)
}

fn sync_directory_for_transaction(path: &Path) -> Result<(), FileTransactionError> {
    sync_directory(path).map_err(|source| FileTransactionError::SyncDir {
        path: path.to_path_buf(),
        source,
    })
}

fn sync_parent_directory_for_transaction(path: &Path) -> Result<(), FileTransactionError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    sync_directory_for_transaction(parent)
}

fn read_marker(path: &Path) -> Result<ParsedMarker, FileTransactionError> {
    let text = std::fs::read_to_string(&*wide(path)).map_err(|source| {
        FileTransactionError::ReadMarker {
            path: path.to_path_buf(),
            source,
        }
    })?;
    parse_marker(path, &text)
}

fn parse_marker(path: &Path, text: &str) -> Result<ParsedMarker, FileTransactionError> {
    let mut format = None;
    let mut id = None;
    let mut status = None;
    let mut entry_count = None;

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "format" => format = Some(value),
            "id" => id = Some(value.to_string()),
            "status" => status = MarkerStatus::parse(value),
            "entries" => {
                entry_count = Some(value.parse::<usize>().map_err(|error| {
                    FileTransactionError::ParseMarker {
                        path: path.to_path_buf(),
                        reason: format!("invalid entries count `{value}`: {error}"),
                    }
                })?);
            }
            _ => {}
        }
    }

    if format != Some("azoth.file-transaction/v1") {
        return Err(FileTransactionError::ParseMarker {
            path: path.to_path_buf(),
            reason: "unsupported marker format".to_string(),
        });
    }
    let id = id.ok_or_else(|| FileTransactionError::ParseMarker {
        path: path.to_path_buf(),
        reason: "missing transaction id".to_string(),
    })?;
    let status = status.ok_or_else(|| FileTransactionError::ParseMarker {
        path: path.to_path_buf(),
        reason: "missing or invalid transaction status".to_string(),
    })?;
    let entry_count = entry_count.ok_or_else(|| FileTransactionError::ParseMarker {
        path: path.to_path_buf(),
        reason: "missing entries count".to_string(),
    })?;

    let mut entries = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        entries.push(TransactionEntry {
            target: marker_path_value(path, text, index, "target")?,
            staged: marker_path_value(path, text, index, "staged")?,
            backup: marker_path_value(path, text, index, "backup")?,
        });
    }

    Ok(ParsedMarker {
        id,
        status,
        entries,
    })
}

fn marker_path_value(
    marker: &Path,
    text: &str,
    index: usize,
    field: &str,
) -> Result<PathBuf, FileTransactionError> {
    let key = format!("entry.{index}.{field}");
    let value = text
        .lines()
        .find_map(|line| {
            let (candidate_key, value) = line.split_once('=')?;
            (candidate_key == key).then_some(value)
        })
        .ok_or_else(|| FileTransactionError::ParseMarker {
            path: marker.to_path_buf(),
            reason: format!("missing `{key}`"),
        })?;
    Ok(PathBuf::from(decode_marker_path(value)))
}

fn apply_entries_forward(entries: &[TransactionEntry]) -> Result<(), FileTransactionError> {
    for entry in entries {
        if let Some(parent) = entry.target.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(&*wide(parent)).map_err(|source| {
                FileTransactionError::CreateDir {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }

        if entry.staged.exists() {
            // Replacement must remain continuously readable. The previous
            // rename-to-backup sequence exposed a window where `target` did
            // not exist, so concurrent SecretArtifact/config readers could
            // fail during an otherwise successful publication. The staged
            // file and destination share a filesystem; replace them with one
            // atomic rename/MoveFileEx operation. Recovery is forward-only,
            // so a new backup is unnecessary. The legacy backup branch below
            // remains for transactions prepared by older engine versions.
            replace_file_atomic_and_sync(&entry.staged, &entry.target)?;
        } else if !entry.target.exists() && entry.backup.exists() {
            rename_path(&entry.backup, &entry.target)?;
        }

        if entry.backup.exists() {
            std::fs::remove_file(&*wide(&entry.backup)).map_err(|source| {
                FileTransactionError::RemoveFile {
                    path: entry.backup.clone(),
                    source,
                }
            })?;
        }
    }

    Ok(())
}

/// Replace `to` with `from` in a single atomic operation.
///
/// A reader of `to` observes either the previous file or the new one in full,
/// never a `NotFound` gap. `remove_file` then `rename` opens exactly that gap,
/// so every publication path that replaces a file a concurrent reader may hold
/// open routes through here instead of reimplementing the sequence.
///
/// Both paths must live on the same filesystem.
///
/// # Errors
///
/// Returns the underlying I/O error when the replacement cannot be performed.
#[cfg(not(windows))]
pub fn atomic_replace_path(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

/// Replace `to` with `from` in a single atomic operation.
///
/// A reader of `to` observes either the previous file or the new one in full,
/// never a `NotFound` gap. `remove_file` then `rename` opens exactly that gap,
/// so every publication path that replaces a file a concurrent reader may hold
/// open routes through here instead of reimplementing the sequence.
///
/// Both paths must live on the same volume. Transient sharing violations from
/// scanners holding the destination open are retried briefly before failing.
///
/// # Errors
///
/// Returns the underlying I/O error when the replacement cannot be performed.
#[cfg(windows)]
pub fn atomic_replace_path(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::time::Duration;

    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    const ATTEMPTS: usize = 8;
    const RETRY_DELAY: Duration = Duration::from_millis(10);

    let from_wide = wide(from)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to_wide = wide(to)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut last_error = None;
    for attempt in 0..ATTEMPTS {
        // SAFETY: both owned buffers are NUL-terminated UTF-16 paths and
        // remain alive for the duration of the Win32 call.
        match unsafe {
            MoveFileExW(
                PCWSTR(from_wide.as_ptr()),
                PCWSTR(to_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } {
            Ok(()) => return Ok(()),
            Err(_error) => {
                // Capture the Win32 failure before the next operation can
                // overwrite the thread-local last-error value.
                last_error = Some(std::io::Error::last_os_error());
            }
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("atomic replacement failed")))
}

fn rename_path(from: &Path, to: &Path) -> Result<(), FileTransactionError> {
    if let Some(parent) = to.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(&*wide(parent)).map_err(|source| {
            FileTransactionError::CreateDir {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    std::fs::rename(&*wide(from), &*wide(to)).map_err(|source| FileTransactionError::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })?;
    sync_parent_directory(to).map_err(|source| FileTransactionError::SyncDir {
        path: to
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        source,
    })
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), FileTransactionError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&*wide(path)).map_err(|source| FileTransactionError::RemoveDir {
        path: path.to_path_buf(),
        source,
    })
}

fn encode_marker_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('%', "%25")
        .replace('\n', "%0A")
}

fn decode_marker_path(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let first = chars.next();
            let second = chars.next();
            match (first, second) {
                (Some('2'), Some('5')) | (None, _) => decoded.push('%'),
                (Some('0'), Some('A')) => decoded.push('\n'),
                (Some(first), Some(second)) => {
                    decoded.push('%');
                    decoded.push(first);
                    decoded.push(second);
                }
                (Some(first), None) => {
                    decoded.push('%');
                    decoded.push(first);
                }
            }
        } else {
            decoded.push(ch);
        }
    }
    decoded
}

fn is_archive_path_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn parse_region_segment(segment: &str) -> Option<(i32, i32)> {
    let coords = segment.strip_prefix("r_")?;
    let mut parts = coords.split('_');
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_are_normalized() {
        assert_eq!(
            normalize_source_path(r".\Objects\Foo\Bar.CGF"),
            "objects/foo/bar.cgf"
        );
        assert_eq!(
            normalize_source_path("/Textures/Foo.DDS"),
            "textures/foo.dds"
        );
    }

    #[test]
    fn source_paths_collapse_redundant_and_trailing_separators() {
        // Repeated internal slashes and a trailing slash must not change the
        // normalized identity, so the two spellings hash to the same GUID.
        assert_eq!(normalize_source_path("a//b/"), normalize_source_path("a/b"));
        assert_eq!(normalize_source_path("a//b/"), "a/b");
        assert_eq!(
            normalize_source_path(r"Objects\\Foo\\Bar.CGF\"),
            "objects/foo/bar.cgf"
        );
        assert_eq!(normalize_source_path("///a///b///"), "a/b");
    }

    #[test]
    fn extension_key_uses_final_extension_only() {
        assert_eq!(
            source_extension_key("objects/foo/textures/bar.dds.a").as_deref(),
            Some("a")
        );
        assert_eq!(
            source_extension_key("localization/en-us/main.loc.xml").as_deref(),
            Some("xml")
        );
    }

    #[test]
    fn engine_path_preserves_source_extension_products() {
        assert_eq!(
            engine_path("Regions/R1/Region.heightmap", "terrain", "heightmap"),
            "regions/r1/region.heightmap"
        );
        assert_eq!(
            engine_path("Textures/Foo.DDS", "textures", "dds"),
            "textures/foo.dds"
        );
    }

    #[test]
    fn engine_path_can_use_caller_supplied_compound_extension_keys() {
        assert_eq!(
            engine_path_with_extension_key(
                "Localization/en-us/main.loc.xml",
                "localization",
                "loc.xml",
                Some("loc.xml"),
            ),
            "localization/en-us/main.loc.xml"
        );
        assert_eq!(
            engine_path_with_extension_key(
                "Localization/en-us/main.loc.xml",
                "localization",
                "loc.bin",
                Some("loc.xml"),
            ),
            "localization/localization/en-us/main.loc.bin"
        );
    }

    #[test]
    fn engine_path_places_derived_products_under_prefix() {
        assert_eq!(
            engine_path("Objects/Foo/Bar.cgf", "models", "static-model.bin"),
            "models/objects/foo/bar.static-model.bin"
        );
        assert_eq!(
            engine_path("Materials/Foo/Armor.mtl", "materials", "material.bin"),
            "materials/materials/foo/armor.material.bin"
        );
    }

    #[test]
    fn material_paths_are_normalized_with_extension() {
        assert_eq!(
            material_source_asset_path("Objects/Foo/Bar").as_deref(),
            Some("objects/foo/bar.mtl")
        );
        assert_eq!(material_source_asset_path("  ").as_deref(), None);
    }

    #[test]
    fn region_paths_parse_level_and_coordinates() {
        let meta = RegionPathMeta::parse(
            r"sharedassets\example\overworld\regions\r_+02_-03\region.distribution",
        )
        .unwrap();

        assert_eq!(meta.level, "overworld");
        assert_eq!(meta.x, 2);
        assert_eq!(meta.y, -3);
    }

    #[test]
    fn authoring_texture_paths_resolve_to_dds_sources() {
        assert_eq!(
            texture_source_asset_path("Objects/Foo/Diff.tif"),
            "objects/foo/diff.dds"
        );
        assert_eq!(
            texture_source_asset_path("Objects/Foo/Normal.dds"),
            "objects/foo/normal.dds"
        );
    }

    #[test]
    fn safe_join_blocks_traversal() {
        let root = std::env::temp_dir().join("az-filesystem-safe-join-test");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        assert!(safe_join(&root, "subdir/file.dds").is_ok());
        assert!(safe_join(&root, "../etc/passwd").is_err());
        assert!(safe_join(&root, "subdir/../../escape").is_err());

        #[cfg(windows)]
        {
            let absolute = std::env::temp_dir().join("outside/foo");
            assert!(safe_join(&root, &absolute.to_string_lossy()).is_err());
        }
    }

    #[test]
    fn atomic_replace_publishes_over_an_existing_destination() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("product.bin");
        let staged = temp.path().join("staged.bin");
        std::fs::write(&destination, "generation-0").unwrap();
        std::fs::write(&staged, "generation-1").unwrap();

        atomic_replace_path(&staged, &destination).unwrap();

        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "generation-1"
        );
        assert!(!staged.exists(), "the staged file is consumed by the move");
    }

    #[test]
    fn atomic_replace_leaves_the_destination_intact_when_the_source_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("product.bin");
        std::fs::write(&destination, "published").unwrap();
        let missing = temp.path().join("staged.bin");

        let error = atomic_replace_path(&missing, &destination)
            .expect_err("a missing source cannot be published");

        #[cfg(windows)]
        assert!(
            error.raw_os_error().is_some(),
            "Windows replacement errors must retain the OS error code"
        );

        // This is the property the reader-visible gap costs you. `remove_file`
        // then `rename` deletes the destination before it discovers the source
        // is gone, so the failure destroys a file that was valid before the
        // call. A single atomic replace either succeeds or changes nothing.
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "published");
    }

    #[test]
    fn marker_replacement_keeps_previous_valid_marker_when_source_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("tx").join(FILE_TRANSACTION_MARKER);
        let previous = "format=azoth.file-transaction/v1\nid=tx\nstatus=prepared\nentries=0\n";
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, previous).unwrap();
        let missing = temp.path().join("missing-marker.tmp");

        let error = replace_file_atomic_and_sync(&missing, &marker)
            .expect_err("a missing temporary marker must not replace the prior marker");

        assert!(matches!(error, FileTransactionError::Rename { .. }));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), previous);
    }

    #[test]
    fn normalize_lexical_resolves_dots() {
        let p = Path::new("a/b/./c/../d");
        assert_eq!(normalize_lexical(p), Path::new("a/b/d"));
    }

    #[test]
    fn product_cache_dir_uses_o3de_project_platform_shape() {
        assert_eq!(PRODUCT_CACHE_DIR_NAME, "Cache");
        assert_eq!(DEFAULT_ASSET_PLATFORM, "pc");
        assert_eq!(
            default_platform_product_cache_dir(Path::new("project")),
            Path::new("project").join("Cache").join("pc")
        );
        assert_eq!(
            platform_product_cache_dir(Path::new("project"), "ios"),
            Path::new("project").join("Cache").join("ios")
        );
    }

    #[test]
    fn project_state_paths_use_name_and_instance_layout() {
        let root = Path::new("projects/sample-game");
        let state = project_state_dir("Sample Game", root);
        let key = ProjectMachineKey::derive("Sample Game", root);

        assert_eq!(
            state,
            azoth_home_dir()
                .join("projects")
                .join(key.project_name())
                .join(key.instance_id())
        );
        assert_eq!(key.project_name(), "sample-game");
        assert!(key.instance_id().starts_with("unlinked-"));
        assert_eq!(
            project_asset_db_path("Sample Game", root),
            state.join("assetdb").join("assetdb.sqlite")
        );
        assert_eq!(
            project_platform_product_cache_dir("Sample Game", root, "pc"),
            state.join("Cache").join("pc")
        );
    }

    #[test]
    fn file_transaction_stages_replaces_and_cleans_marker() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp
            .path()
            .join(".azoth")
            .join("transactions")
            .join("files");
        let target = temp.path().join("assets").join("main.scene.ron");

        let receipt = FileTransaction::new(&tx_root)
            .commit([FileWrite::new(&target, b"new document".to_vec())])
            .unwrap();

        assert_eq!(receipt.write_count, 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new document");
        assert_eq!(
            transaction_entry_count(&tx_root),
            0,
            "successful transactions should not leave recovery markers"
        );
    }

    #[test]
    fn file_transaction_commits_past_the_windows_legacy_path_limit() {
        let temp = tempfile::tempdir().unwrap();
        // A product cache under a project the user nested a few levels deep
        // clears 260 characters without trying. Storing roots in the legacy
        // form is what makes that a limit at all, so the transaction
        // primitives have to widen at the syscall.
        let deep = (0..6).fold(temp.path().to_path_buf(), |path, index| {
            path.join(format!("{index}-{}", "d".repeat(30)))
        });
        let tx_root = deep.join("transactions");
        let target = deep.join("assets").join("main.scene.ron");
        assert!(
            target.as_os_str().len() > 260,
            "fixture must exceed MAX_PATH, got {}",
            target.as_os_str().len()
        );

        let receipt = FileTransaction::new(&tx_root)
            .commit([FileWrite::new(&target, b"deep document".to_vec())])
            .unwrap();

        assert_eq!(receipt.write_count, 1);
        assert_eq!(
            std::fs::read_to_string(&*wide(&target)).unwrap(),
            "deep document"
        );
    }

    #[test]
    fn file_transaction_replaces_existing_file_with_backup_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp
            .path()
            .join(".azoth")
            .join("transactions")
            .join("files");
        let target = temp.path().join("assets").join("main.scene.ron");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "old document").unwrap();

        FileTransaction::new(&tx_root)
            .commit([FileWrite::new(&target, b"new document".to_vec())])
            .unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new document");
        assert_eq!(transaction_entry_count(&tx_root), 0);
    }

    #[test]
    fn file_transaction_recovers_interrupted_apply_forward() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp
            .path()
            .join(".azoth")
            .join("transactions")
            .join("files");
        let target = temp.path().join("assets").join("main.scene.ron");
        let layout = TransactionLayout::new(&tx_root, "tx-recover");
        let entry = TransactionEntry {
            target: target.clone(),
            staged: layout.staging_path(0),
            backup: layout.backup_path(0),
        };

        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "old document").unwrap();
        write_file_sync(&entry.staged, b"new document").unwrap();
        write_marker(
            &layout.marker,
            MarkerStatus::Applying,
            std::slice::from_ref(&entry),
        )
        .unwrap();
        rename_path(&target, &entry.backup).unwrap();

        let reports = FileTransaction::new(&tx_root).recover_pending().unwrap();

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, "tx-recover");
        assert_eq!(reports[0].recovered_write_count, 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new document");
        assert!(!entry.staged.exists());
        assert!(!entry.backup.exists());
        assert!(!layout.dir.exists());
    }

    #[test]
    fn file_transaction_discards_prepared_writes_without_reporting_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let target = temp.path().join("assets").join("main.scene.ron");
        let layout = TransactionLayout::new(&tx_root, "tx-prepared");
        let entry = TransactionEntry {
            target: target.clone(),
            staged: layout.staging_path(0),
            backup: layout.backup_path(0),
        };

        std::fs::create_dir_all(&tx_root).unwrap();
        write_file_sync(&entry.staged, b"discarded document").unwrap();
        write_marker(
            &layout.marker,
            MarkerStatus::Prepared,
            std::slice::from_ref(&entry),
        )
        .unwrap();

        let reports = FileTransaction::new(&tx_root).recover_pending().unwrap();

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, "tx-prepared");
        assert_eq!(reports[0].recovered_write_count, 0);
        assert!(!target.exists());
        assert!(!layout.dir.exists());
    }

    #[test]
    fn file_transaction_stages_path_and_reader_sources() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let path_source = temp.path().join("source.bin");
        let path_target = temp.path().join("path-target.bin");
        let reader_target = temp.path().join("reader-target.bin");
        std::fs::write(&path_source, b"path source").unwrap();

        FileTransaction::new(&tx_root)
            .commit([
                FileWrite::from_path(&path_target, &path_source),
                FileWrite::from_reader(&reader_target, std::io::Cursor::new(b"reader source")),
            ])
            .unwrap();

        assert_eq!(std::fs::read(&path_target).unwrap(), b"path source");
        assert_eq!(std::fs::read(&reader_target).unwrap(), b"reader source");
    }

    #[test]
    fn file_transaction_recovery_does_not_create_a_missing_root_or_lock() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let lock_path = transaction_lock_path(&tx_root).unwrap();

        assert!(
            FileTransaction::new(&tx_root)
                .recover_pending()
                .unwrap()
                .is_empty()
        );
        assert!(!tx_root.exists());
        assert!(!lock_path.exists());
    }

    #[test]
    fn file_transaction_reports_path_source_read_errors() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let source = temp.path().join("missing.bin");
        let target = temp.path().join("target.bin");

        let error = FileTransaction::new(&tx_root)
            .commit([FileWrite::from_path(&target, &source)])
            .expect_err("a missing source must fail before marker publication");

        assert!(matches!(
            error,
            FileTransactionError::ReadSource { path, .. } if path == source
        ));
        assert!(!target.exists());
        assert_eq!(transaction_marker_count(&tx_root), 0);
        assert_eq!(transaction_entry_count(&tx_root), 0);
    }

    #[test]
    fn file_transaction_reports_reader_source_errors() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let target = temp.path().join("target.bin");

        let error = FileTransaction::new(&tx_root)
            .commit([FileWrite::from_reader(&target, FailingReader)])
            .expect_err("a reader failure must abort before marker publication");

        assert!(matches!(
            error,
            FileTransactionError::ReadSource { path, .. } if path == Path::new("<stream>")
        ));
        assert!(!target.exists());
        assert_eq!(transaction_entry_count(&tx_root), 0);
    }

    #[test]
    fn file_transaction_serializes_concurrent_commits_per_root() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let first_target = temp.path().join("first.bin");
        let second_target = temp.path().join("second.bin");
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(0);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(0);

        let first = std::thread::spawn({
            let tx_root = tx_root.clone();
            let first_target = first_target.clone();
            move || {
                FileTransaction::new(tx_root).commit([FileWrite::from_reader(
                    first_target,
                    BlockingReader {
                        started: Some(started_sender),
                        release: release_receiver,
                        released: false,
                    },
                )])
            }
        });
        started_receiver
            .recv()
            .expect("first commit must reach its locked staging read");

        let (attempted_sender, attempted_receiver) = std::sync::mpsc::sync_channel(0);
        let second = std::thread::spawn({
            let second_target = second_target.clone();
            move || {
                attempted_sender
                    .send(())
                    .expect("test receiver remains alive");
                FileTransaction::new(tx_root)
                    .commit([FileWrite::new(second_target, b"second".to_vec())])
            }
        });
        attempted_receiver
            .recv()
            .expect("second commit must start before it waits for the lock");
        assert!(
            !second_target.exists(),
            "the second commit must not publish while the first holds the root lock"
        );

        release_sender
            .send(())
            .expect("first staging reader remains blocked until release");
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert_eq!(std::fs::read(&first_target).unwrap(), b"first\0");
        assert_eq!(std::fs::read(&second_target).unwrap(), b"second");
    }

    #[test]
    fn file_transaction_lock_excludes_competing_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        std::fs::create_dir_all(&tx_root).unwrap();
        let lock = acquire_file_transaction_lock(&tx_root, false).unwrap();
        let lock_path = transaction_lock_path(&tx_root).unwrap();
        let contender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();

        assert!(
            contender.try_lock_exclusive().is_err(),
            "a recovery process must not acquire the root lock during a commit"
        );
        drop(lock);
        contender
            .try_lock_exclusive()
            .expect("recovery can acquire the lock after commit releases it");
        contender.unlock().unwrap();
        assert!(
            FileTransaction::new(&tx_root)
                .recover_pending()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn file_transaction_lock_aliases_share_a_canonical_process_key() {
        let temp = tempfile::tempdir().unwrap();
        let tx_root = temp.path().join("transactions");
        let alias_root = tx_root.join("..").join("transactions");
        std::fs::create_dir_all(&tx_root).unwrap();

        let root_lock_path = transaction_lock_path(&tx_root).unwrap();
        let alias_lock_path = transaction_lock_path(&alias_root).unwrap();
        assert_eq!(root_lock_path, alias_lock_path);
        #[cfg(windows)]
        {
            let case_alias = PathBuf::from(tx_root.to_string_lossy().to_ascii_uppercase());
            assert_eq!(transaction_lock_path(&case_alias).unwrap(), root_lock_path);
        }
        assert!(Arc::ptr_eq(
            &process_lock_state(&root_lock_path),
            &process_lock_state(&alias_lock_path)
        ));

        let lock = acquire_file_transaction_lock(&tx_root, false).unwrap();
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(0);
        let (acquired_sender, acquired_receiver) = std::sync::mpsc::sync_channel(0);
        let contender = std::thread::spawn(move || {
            started_sender.send(()).unwrap();
            let _lock = acquire_file_transaction_lock(&alias_root, false).unwrap();
            acquired_sender.send(()).unwrap();
        });
        started_receiver.recv().unwrap();
        assert!(
            acquired_receiver
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "an alias path must wait for the same process lock"
        );
        drop(lock);
        acquired_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("the alias lock should be acquired after release");
        contender.join().unwrap();
    }

    fn transaction_entry_count(root: &Path) -> usize {
        std::fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .count()
    }

    fn transaction_marker_count(root: &Path) -> usize {
        std::fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(FILE_TRANSACTION_MARKER).is_file())
            .count()
    }

    struct BlockingReader {
        started: Option<std::sync::mpsc::SyncSender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        released: bool,
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "test source failure",
            ))
        }
    }

    impl Read for BlockingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.released {
                if let Some(started) = self.started.take() {
                    started.send(()).map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test receiver closed")
                    })?;
                }
                self.release.recv().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test release closed")
                })?;
                self.released = true;
                buffer[..6].copy_from_slice(b"first\0");
                return Ok(6);
            }
            Ok(0)
        }
    }
}
