//! Shared local Turso connection policy for Azoth's embedded stores.
//!
//! This crate owns the engine-wide storage primitive: every local Turso database
//! uses ordinary `SQLite` B-tree indexes and plain WAL journaling. Turso owns the
//! online PASSIVE auto-checkpoint cadence; Azoth issues explicit TRUNCATE only
//! at offline maintenance boundaries. File databases share one Turso database
//! owner per resolved process-local path.
//!
//! One OS process owns each path-backed database (ADR 0039 §7). This crate
//! enforces both halves of that: the process-local [`OpenSlot`] hands every
//! opener of a path the same database owner, and the [`Deed`] beside the
//! database refuses a second process outright.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use thiserror::Error;

pub const WAL_JOURNAL_MODE: &str = "wal";

/// A local Turso database and its configured connection.
///
/// Keep this value alive for at least as long as any clones of [`Self::connection`].
#[derive(Clone)]
pub struct LocalDatabase {
    owner: Arc<DatabaseOwner>,
    connection: turso::Connection,
}

struct DatabaseOwner {
    // Declared first so the engine releases its own handles on this path
    // before the slot, and with it the deed, is dropped.
    database: turso::Database,
    vacuum_enabled: bool,
    // Keeps the per-path slot alive exactly as long as its Turso database.
    _slot: Arc<OpenSlot>,
}

#[derive(Default)]
struct OpenSlot {
    owner: async_lock::Mutex<Weak<DatabaseOwner>>,
    // The cross-process half of single ownership, held for exactly as long as
    // this process owns the path. `None` only for `:memory:`, which is private
    // to its opener and never registered in `OPEN_SLOTS`.
    _deed: Option<Deed>,
}

static OPEN_SLOTS: OnceLock<Mutex<HashMap<PathBuf, Weak<OpenSlot>>>> = OnceLock::new();

impl std::fmt::Debug for LocalDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalDatabase")
            .finish_non_exhaustive()
    }
}

impl LocalDatabase {
    #[must_use]
    pub fn database(&self) -> &turso::Database {
        &self.owner.database
    }

    #[must_use]
    pub const fn connection(&self) -> &turso::Connection {
        &self.connection
    }

    /// Open another configured connection against the same local database.
    ///
    /// Stores use this after schema migration/bootstrap work so their long-lived
    /// runtime connection starts without an initialization cursor or snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError::Connect`] if the database refuses a new connection,
    /// or [`OpenError::BusyTimeout`] if `busy_timeout` cannot be applied to it.
    pub fn new_connection(&self, busy_timeout: Duration) -> Result<turso::Connection, OpenError> {
        configured_connection(&self.owner.database, busy_timeout)
    }

    /// Create another configured handle backed by the same process-local
    /// database owner.
    ///
    /// This is the service-facing connection factory: callers get an
    /// independent connection without reopening the file, re-running storage
    /// initialization, or constructing a second pager.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::new_connection`] returns for `busy_timeout`.
    pub fn new_handle(&self, busy_timeout: Duration) -> Result<Self, OpenError> {
        Ok(Self {
            owner: Arc::clone(&self.owner),
            connection: configured_connection(&self.owner.database, busy_timeout)?,
        })
    }

    /// Read the journal mode reported by the configured connection.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError::ReadJournalMode`] if the `journal_mode` pragma
    /// cannot be run or returns no row.
    pub async fn journal_mode(&self) -> Result<String, OpenError> {
        read_journal_mode(&self.connection).await
    }

    /// Flush dirty pages held by Turso's page cache to durable storage.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError::Flush`] if the engine's cache flush fails.
    pub fn flush_to_disk(&self) -> Result<(), OpenError> {
        self.connection.cacheflush().map_err(OpenError::Flush)
    }

    /// Consolidate committed WAL state into the main database.
    ///
    /// A refused checkpoint is reported in the pragma's **first** result column
    /// rather than as an error, so callers must decode that column rather than
    /// treating a successfully executed pragma as a successful checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError::CheckpointBusy`] if the engine reports the
    /// checkpoint as busy — either as a `Busy` error or in the pragma's first
    /// result column — [`OpenError::CheckpointUnreported`] if the pragma yields
    /// no row, and [`OpenError::CheckpointWal`] for any other engine failure.
    pub async fn checkpoint(&self) -> Result<(), OpenError> {
        let mut rows = self
            .connection
            .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await
            .map_err(checkpoint_error)?;
        let busy = rows
            .next()
            .await
            .map_err(checkpoint_error)?
            .ok_or(OpenError::CheckpointUnreported)?
            .get::<i64>(0)
            .map_err(checkpoint_error)?;
        while rows.next().await.map_err(checkpoint_error)?.is_some() {}
        if busy == 0 {
            Ok(())
        } else {
            Err(OpenError::CheckpointBusy)
        }
    }

    /// Rebuild the database file and reclaim free pages.
    ///
    /// This must be an explicit offline-maintenance action. Normal database
    /// opens avoid Turso's experimental vacuum feature so startup never pays
    /// for shrink-specific storage behavior.
    ///
    /// # Errors
    ///
    /// Returns [`OpenError::Vacuum`] if the `VACUUM` statement cannot be run or
    /// fails while its rows are drained.
    pub async fn vacuum(&self) -> Result<(), OpenError> {
        drain_rows(&self.connection, "VACUUM", OpenError::Vacuum).await
    }
}

fn checkpoint_error(error: turso::Error) -> OpenError {
    match error {
        turso::Error::Busy(_) | turso::Error::BusySnapshot(_) => OpenError::CheckpointBusy,
        source => OpenError::CheckpointWal(source),
    }
}

#[derive(Debug, Error)]
pub enum OpenError {
    #[error("resolve local Turso database path {path}: {source}")]
    ResolvePath {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("open local Turso database: {0}")]
    Open(#[source] turso::Error),
    #[error("connect to local Turso database: {0}")]
    Connect(#[source] turso::Error),
    #[error("configure local Turso busy timeout: {0}")]
    BusyTimeout(#[source] turso::Error),
    #[error("read local Turso journal mode: {0}")]
    ReadJournalMode(#[source] turso::Error),
    #[error("checkpoint Turso WAL: {0}")]
    CheckpointWal(#[source] turso::Error),
    #[error("checkpoint Turso WAL: the engine refused it as busy")]
    CheckpointBusy,
    #[error("checkpoint Turso WAL: the engine returned no checkpoint result")]
    CheckpointUnreported,
    #[error(
        "database at {path} is owned by another process: {holder}. One process owns this database (ADR 0039 §7) — reach it through the owning service's API instead of opening the file"
    )]
    Held { path: String, holder: String },
    #[error(
        "refusing to open the database at {path}: {ENGINE_FILE_LOCK_DISABLE} is set in this process's environment, which disables the storage engine's own cross-process file lock"
    )]
    Disabled { path: String },
    #[error("claim the deed for the local Turso database at {path}: {source}")]
    Deed {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "open local Turso database with vacuum requested while an existing process-local owner was opened without vacuum"
    )]
    MissingVacuumFeature,
    #[error("set Turso WAL journal mode: {0}")]
    SetJournalMode(#[source] turso::Error),
    #[error("Turso returned journal mode `{actual}` after WAL was requested")]
    UnexpectedJournalMode { actual: String },
    #[error(
        "database uses retired Turso MVCC storage and must be exported and reset before opening"
    )]
    LegacyMvccResetRequired,
    #[error("flush local Turso database page cache: {0}")]
    Flush(#[source] turso::Error),
    #[error("vacuum local Turso database: {0}")]
    Vacuum(#[source] turso::Error),
    #[error("refusing to reset local Turso database at {path}: this process still owns it")]
    ResetActive { path: String },
    #[error("remove local Turso database file {path}: {source}")]
    ResetFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Remove one offline local database and its engine-owned journal files.
///
/// The database path is resolved before any mutation. The same cross-process
/// deed used by normal open is held across the complete reset, and a live
/// process-local owner is rejected before the deed is acquired. Only the
/// fixed file set owned by Turso and Azoth is removed; the deed itself remains
/// as harmless holder metadata and is reused by the next opener.
///
/// # Errors
///
/// Returns [`OpenError::ResolvePath`] if `path` cannot be made absolute,
/// [`OpenError::ResetActive`] if this process still holds a live owner for it,
/// [`OpenError::Deed`] or [`OpenError::Held`] if the cross-process deed cannot
/// be acquired, and [`OpenError::ResetFile`] if one of the engine-owned files
/// exists but cannot be removed.
pub fn reset_local_database(path: impl AsRef<Path>) -> Result<Vec<PathBuf>, OpenError> {
    let path = path.as_ref();
    let resolved = std::path::absolute(path).map_err(|source| OpenError::ResolvePath {
        path: path.display().to_string(),
        source,
    })?;

    let slots = OPEN_SLOTS.get_or_init(|| Mutex::new(HashMap::new()));
    let owner_is_live = {
        let mut slots = slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots.retain(|_, slot| slot.strong_count() > 0);
        let live = slots.get(&resolved).and_then(Weak::upgrade).is_some();
        drop(slots);
        live
    };
    if owner_is_live {
        return Err(OpenError::ResetActive {
            path: resolved.display().to_string(),
        });
    }

    // Acquisition races safely with a concurrent opener: exactly one side
    // obtains the kernel-held deed and the other fails closed.
    let _deed = Deed::acquire(&resolved)?;
    let mut removed = Vec::new();
    for owned_path in local_database_files(&resolved) {
        match std::fs::remove_file(&owned_path) {
            Ok(()) => removed.push(owned_path),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(OpenError::ResetFile {
                    path: owned_path.display().to_string(),
                    source,
                });
            }
        }
    }
    Ok(removed)
}

fn local_database_files(database: &Path) -> [PathBuf; 6] {
    let append = |suffix: &str| {
        let mut name = database
            .file_name()
            .map_or_else(OsString::new, std::ffi::OsStr::to_os_string);
        name.push(suffix);
        database.with_file_name(name)
    };
    [
        database.to_owned(),
        database.with_extension("db-log"),
        append("-wal"),
        append("-shm"),
        append("-journal"),
        append(".azoth-mvcc-v1"),
    ]
}

/// Open a local Turso database with Azoth's plain-WAL policy.
///
/// # Errors
///
/// Returns [`OpenError::ResolvePath`] if `path` cannot be made absolute,
/// [`OpenError::Held`] if another process owns the database,
/// [`OpenError::Disabled`] if the engine's cross-process file lock is disabled
/// in this environment, [`OpenError::LegacyMvccResetRequired`] if the file is a
/// retired MVCC image, and the [`OpenError::Open`], [`OpenError::Connect`],
/// [`OpenError::BusyTimeout`], [`OpenError::SetJournalMode`],
/// [`OpenError::ReadJournalMode`] or [`OpenError::UnexpectedJournalMode`]
/// variants if the engine refuses one of the steps that establish plain WAL.
pub async fn open_local(path: &str, busy_timeout: Duration) -> Result<LocalDatabase, OpenError> {
    open_local_with_policy(path, busy_timeout, false, JournalPolicy::Wal).await
}

/// Open a local Turso database with Azoth's plain-WAL policy and the
/// experimental vacuum feature enabled.
///
/// This is intended for explicit offline maintenance tools only. If a database
/// owner for `path` already exists in the process and was opened without
/// vacuum, this returns an error instead of silently reusing an incompatible
/// owner.
///
/// # Errors
///
/// Returns [`OpenError::MissingVacuumFeature`] if a process-local owner for
/// `path` already exists and was opened without vacuum, plus any error
/// [`open_local`] returns.
pub async fn open_local_with_vacuum(
    path: &str,
    busy_timeout: Duration,
) -> Result<LocalDatabase, OpenError> {
    open_local_with_policy(path, busy_timeout, true, JournalPolicy::Wal).await
}

/// Open a local database for exclusive offline export or maintenance.
///
/// This boundary deliberately preserves the existing journal mode so recovery
/// can export a retired MVCC image before reset. Runtime callers use
/// [`open_local`] and therefore cannot accidentally adopt that image.
///
/// # Errors
///
/// Returns the same failures as [`open_local`] except the journal-mode ones:
/// this boundary preserves the existing mode instead of setting WAL, so it
/// never reports [`OpenError::ReadJournalMode`], [`OpenError::SetJournalMode`],
/// [`OpenError::UnexpectedJournalMode`] or
/// [`OpenError::LegacyMvccResetRequired`]. With `vacuum_enabled` it can also
/// return [`OpenError::MissingVacuumFeature`] when a process-local owner for
/// `path` was already opened without vacuum.
pub async fn open_local_for_offline_maintenance(
    path: &str,
    busy_timeout: Duration,
    vacuum_enabled: bool,
) -> Result<LocalDatabase, OpenError> {
    open_local_with_policy(path, busy_timeout, vacuum_enabled, JournalPolicy::Preserve).await
}

#[derive(Clone, Copy)]
enum JournalPolicy {
    Wal,
    Preserve,
}

async fn open_local_with_policy(
    path: &str,
    busy_timeout: Duration,
    vacuum_enabled: bool,
    journal_policy: JournalPolicy,
) -> Result<LocalDatabase, OpenError> {
    if path == ":memory:" {
        return open_with_owner(path, busy_timeout, None, vacuum_enabled, journal_policy).await;
    }

    let key = std::path::absolute(Path::new(path)).map_err(|source| OpenError::ResolvePath {
        path: path.to_owned(),
        source,
    })?;
    let slot = open_slot(key)?;
    let mut current_owner = slot.owner.lock().await;
    if let Some(owner) = current_owner.upgrade() {
        return connect_with_policy(owner, busy_timeout, vacuum_enabled, journal_policy).await;
    }

    let local = open_with_owner(
        path,
        busy_timeout,
        Some(Arc::clone(&slot)),
        vacuum_enabled,
        journal_policy,
    )
    .await?;
    *current_owner = Arc::downgrade(&local.owner);
    Ok(local)
}

fn open_slot(path: PathBuf) -> Result<Arc<OpenSlot>, OpenError> {
    let slots = OPEN_SLOTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut slots = slots
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    slots.retain(|_, slot| slot.strong_count() > 0);
    if let Some(slot) = slots.get(&path).and_then(Weak::upgrade) {
        return Ok(slot);
    }

    // One gate, two conditions, one call site. The deed is taken first and
    // unconditionally — it is the enforcement, and it never consults the
    // environment for its own decision. Refusing an environment that has
    // disabled the *engine's* own cross-process lock is belt-and-braces
    // hygiene on a layer az does not own, applied only once the deed is held.
    let deed = Deed::acquire(&path)?;
    if std::env::var_os(ENGINE_FILE_LOCK_DISABLE).is_some() {
        return Err(OpenError::Disabled {
            path: path.display().to_string(),
        });
    }

    let slot = Arc::new(OpenSlot {
        owner: async_lock::Mutex::default(),
        _deed: Some(deed),
    });
    slots.insert(path, Arc::downgrade(&slot));
    Ok(slot)
}

/// The environment variable that removes Turso's own cross-process file lock.
/// Azoth never sets or unsets it; it only refuses to open when it is set,
/// because a database opened under it has lost the engine's guard.
const ENGINE_FILE_LOCK_DISABLE: &str = "LIMBO_DISABLE_FILE_LOCK";

/// Fixed-shape leading token of a deed. Bump it when the line shape changes.
const DEED_VERSION: &str = "az-deed-v1";

/// This process's exclusive claim on one path-backed local database.
///
/// **The lock decides, the text explains.** The claim is a kernel-held
/// exclusion on a `<database>.deed` sidecar, so a crashed, killed, or
/// `SIGKILL`ed owner has it released by the kernel on process teardown: there
/// are no stale holders and therefore no reclamation policy. The line written
/// into the sidecar is only ever read to compose a refusal message; nothing
/// reads it to make a decision, so a stale or torn line degrades the message
/// and never the verdict. The sidecar is deliberately not deleted on drop —
/// an unlocked deed is not a held one. On Unix, `flock` is attached to the
/// sidecar inode and cannot prevent an external actor from unlinking and
/// recreating the path; cache removal is therefore an offline operation that
/// must happen after the owner exits.
struct Deed {
    // Dropping the handle releases the claim.
    _file: File,
}

impl Deed {
    /// Claim `database` for this process for as long as the returned value lives.
    fn acquire(database: &Path) -> Result<Self, OpenError> {
        let path = deed(database);
        let file = match claim(&path) {
            Ok(file) => file,
            Err(source) if held(&source) => {
                return Err(OpenError::Held {
                    path: database.display().to_string(),
                    holder: holder(&path),
                });
            }
            Err(source) => {
                return Err(OpenError::Deed {
                    path: path.display().to_string(),
                    source,
                });
            }
        };
        inscribe(&file).map_err(|source| OpenError::Deed {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self { _file: file })
    }
}

fn deed(database: &Path) -> PathBuf {
    let mut name = database
        .file_name()
        .map_or_else(OsString::new, std::ffi::OsStr::to_os_string);
    name.push(".deed");
    database.with_file_name(name)
}

/// Take the platform's exclusive claim on the deed sidecar.
///
/// Both platforms use a primitive the kernel releases on process teardown and
/// that leaves the deed's text readable to a refused opener — the refusal has
/// to be able to name the holder. Neither is a POSIX `fcntl` record lock, so
/// another descriptor on the path closing in this process cannot drop it.
#[cfg(windows)]
fn claim(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    // Share the deed for reading so a refused opener can read the holder line,
    // but never for writing or deletion. Deleting and recreating a live sidecar
    // would create a new name that no longer carries this owner's exclusion.
    // A `LockFileEx` byte range would deny the refused opener the read as well,
    // and the message is the point.
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

#[cfg(unix)]
fn claim(path: &Path) -> std::io::Result<File> {
    use fs2::FileExt;

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    // `flock(2)`, not `fcntl` record locking, and advisory only to other
    // `flock` callers — so `read_to_string` of the holder line still works.
    file.try_lock_exclusive()?;
    Ok(file)
}

#[cfg(windows)]
fn held(error: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    matches!(error.raw_os_error(), Some(32 | 33))
}

#[cfg(unix)]
fn held(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
}

fn inscribe(file: &File) -> std::io::Result<()> {
    // A previous holder's line may be longer than this one.
    file.set_len(0)?;
    let acquired_unix_s = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let line = format!(
        "{DEED_VERSION} pid={} acquired_unix_s={} process={}\n",
        std::process::id(),
        acquired_unix_s,
        process()
    );
    (&mut &*file).write_all(line.as_bytes())
}

fn holder(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .as_deref()
        .and_then(parse)
        .unwrap_or_else(|| "deed unreadable".to_owned())
}

/// Read a deed's fixed-shape line back into a holder description.
///
/// Anything that is not exactly the shape [`inscribe`] writes yields `None`,
/// which degrades the message rather than the decision.
fn parse(text: &str) -> Option<String> {
    let rest = text.lines().next()?.strip_prefix(DEED_VERSION)?;
    let (pid, rest) = rest.strip_prefix(" pid=")?.split_once(' ')?;
    let (acquired_unix_s, process) = rest.strip_prefix("acquired_unix_s=")?.split_once(' ')?;
    let process = process.strip_prefix("process=")?;
    if pid.is_empty() || acquired_unix_s.parse::<u64>().is_err() || process.is_empty() {
        return None;
    }
    Some(format!(
        "{process} (pid {pid}), held since unix second {acquired_unix_s}"
    ))
}

/// The holder is observed from the OS, never declared by a caller, so it cannot
/// drift from the process that actually holds the deed.
fn process() -> String {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::file_stem)
        .map_or_else(
            || "unknown process".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
}

async fn open_with_owner(
    path: &str,
    busy_timeout: Duration,
    slot: Option<Arc<OpenSlot>>,
    vacuum_enabled: bool,
    journal_policy: JournalPolicy,
) -> Result<LocalDatabase, OpenError> {
    let database = build_local_database(path, vacuum_enabled).await?;
    let slot = slot.unwrap_or_default();
    let owner = Arc::new(DatabaseOwner {
        database,
        vacuum_enabled,
        _slot: slot,
    });
    connect_with_policy(owner, busy_timeout, vacuum_enabled, journal_policy).await
}

async fn connect_with_policy(
    owner: Arc<DatabaseOwner>,
    busy_timeout: Duration,
    vacuum_requested: bool,
    journal_policy: JournalPolicy,
) -> Result<LocalDatabase, OpenError> {
    if vacuum_requested && !owner.vacuum_enabled {
        return Err(OpenError::MissingVacuumFeature);
    }
    let connection = configured_connection(&owner.database, busy_timeout)?;
    if matches!(journal_policy, JournalPolicy::Wal) {
        ensure_wal(&connection).await?;
    }
    Ok(LocalDatabase { owner, connection })
}

/// The sole production construction point for a path-backed Turso database.
///
/// File-lock requirements and storage-engine builder policy belong here so a
/// storage upgrade cannot leave one open path with weaker ownership rules.
async fn build_local_database(
    path: &str,
    vacuum_enabled: bool,
) -> Result<turso::Database, OpenError> {
    turso::Builder::new_local(path)
        .experimental_vacuum(vacuum_enabled)
        .build()
        .await
        .map_err(OpenError::Open)
}

fn configured_connection(
    database: &turso::Database,
    busy_timeout: Duration,
) -> Result<turso::Connection, OpenError> {
    let connection = database.connect().map_err(OpenError::Connect)?;
    connection
        .busy_timeout(busy_timeout)
        .map_err(OpenError::BusyTimeout)?;
    Ok(connection)
}

async fn ensure_wal(connection: &turso::Connection) -> Result<(), OpenError> {
    let previous_mode = read_journal_mode(connection).await?;
    if previous_mode.eq_ignore_ascii_case("mvcc") {
        return Err(OpenError::LegacyMvccResetRequired);
    }
    if !previous_mode.eq_ignore_ascii_case(WAL_JOURNAL_MODE) {
        connection
            .pragma_update("journal_mode", "'wal'")
            .await
            .map_err(OpenError::SetJournalMode)?;
    }

    let actual = read_journal_mode(connection).await?;
    if !actual.eq_ignore_ascii_case(WAL_JOURNAL_MODE) {
        return Err(OpenError::UnexpectedJournalMode { actual });
    }
    Ok(())
}

async fn read_journal_mode(connection: &turso::Connection) -> Result<String, OpenError> {
    let mut rows = connection
        .query("PRAGMA journal_mode", ())
        .await
        .map_err(OpenError::ReadJournalMode)?;
    let row = rows
        .next()
        .await
        .map_err(OpenError::ReadJournalMode)?
        .ok_or_else(|| OpenError::UnexpectedJournalMode {
            actual: "<missing>".to_owned(),
        })?;
    let mode = row.get::<String>(0).map_err(OpenError::ReadJournalMode)?;
    while rows
        .next()
        .await
        .map_err(OpenError::ReadJournalMode)?
        .is_some()
    {}
    Ok(mode)
}

async fn drain_rows(
    connection: &turso::Connection,
    sql: &str,
    map_error: fn(turso::Error) -> OpenError,
) -> Result<(), OpenError> {
    let mut rows = connection.query(sql, ()).await.map_err(map_error)?;
    while rows.next().await.map_err(map_error)?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEED_CHILD_DATABASE: &str = "AZ_TURSO_DEED_CHILD_DATABASE";

    #[test]
    #[ignore = "subprocess entry point for cross-process deed tests"]
    fn deed_subprocess_entry() {
        let path = std::env::var(DEED_CHILD_DATABASE).expect("child database path");
        let Err(error) = futures_lite::future::block_on(open_local(&path, Duration::from_secs(1)))
        else {
            panic!("the parent process owns the database deed")
        };
        assert!(matches!(error, OpenError::Held { .. }), "{error}");
    }

    #[test]
    fn a_second_process_is_refused_even_when_the_storage_lock_is_disabled() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("owned.sqlite");
            let path_text = path.to_string_lossy().into_owned();
            let _owner = open_local(&path_text, Duration::from_secs(1))
                .await
                .expect("parent owns the database");

            for disable_storage_lock in [false, true] {
                let mut child = std::process::Command::new(
                    std::env::current_exe().expect("current test binary"),
                );
                child
                    .args([
                        "--ignored",
                        "--exact",
                        "tests::deed_subprocess_entry",
                        "--nocapture",
                    ])
                    .env(DEED_CHILD_DATABASE, &path_text);
                if disable_storage_lock {
                    child.env(ENGINE_FILE_LOCK_DISABLE, "1");
                }
                let status = child.status().expect("run refused child opener");
                assert!(
                    status.success(),
                    "child must observe OpenError::Held with storage lock disabled={disable_storage_lock}"
                );
            }
        });
    }

    #[cfg(windows)]
    #[test]
    fn a_live_deed_refuses_deletion_until_its_owner_exits() {
        let temp = tempfile::tempdir().expect("tempdir");
        let database = temp.path().join("owned.sqlite");
        let deed_path = deed(&database);
        let owner = Deed::acquire(&database).expect("claim deed");

        assert!(
            std::fs::remove_file(&deed_path).is_err(),
            "a cache reset must not unlink and recreate a live ownership deed"
        );

        drop(owner);
        std::fs::remove_file(&deed_path).expect("released deed may be removed offline");
    }

    #[test]
    fn offline_reset_refuses_a_live_owner_and_removes_only_owned_files() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let database = temp.path().join("asset.sqlite");
            let path = database.to_string_lossy().into_owned();
            let owner = open_local(&path, Duration::from_secs(1))
                .await
                .expect("open database");
            owner
                .connection()
                .execute("CREATE TABLE records(id INTEGER PRIMARY KEY)", ())
                .await
                .expect("create table");
            owner.flush_to_disk().expect("flush database");

            let error = reset_local_database(&database).expect_err("live owner must fence reset");
            assert!(matches!(error, OpenError::ResetActive { .. }), "{error}");
            assert!(database.is_file());

            drop(owner);
            let unrelated = temp.path().join("asset.sqlite.notes");
            std::fs::write(&unrelated, b"keep").expect("write unrelated file");
            let removed = reset_local_database(&database).expect("reset offline database");
            assert!(removed.contains(&database));
            assert!(!database.exists());
            assert!(!database.with_extension("db-log").exists());
            assert!(
                !database
                    .with_file_name("asset.sqlite.azoth-mvcc-v1")
                    .exists()
            );
            assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
            assert!(
                deed(&database).exists(),
                "deed metadata is intentionally retained"
            );
        });
    }

    #[test]
    fn reset_file_set_is_exact_for_extensionless_and_extension_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        assert_eq!(
            local_database_files(&root.join("asset.sqlite")),
            [
                root.join("asset.sqlite"),
                root.join("asset.db-log"),
                root.join("asset.sqlite-wal"),
                root.join("asset.sqlite-shm"),
                root.join("asset.sqlite-journal"),
                root.join("asset.sqlite.azoth-mvcc-v1"),
            ]
        );
        assert_eq!(
            local_database_files(&root.join("assetdb"))[1],
            root.join("assetdb.db-log")
        );
    }

    #[test]
    fn opens_file_and_memory_databases_in_wal_mode() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            for path in [
                temp.path()
                    .join("store.sqlite")
                    .to_string_lossy()
                    .into_owned(),
                ":memory:".to_owned(),
            ] {
                let local = open_local(&path, Duration::from_secs(1))
                    .await
                    .expect("open Turso WAL database");
                let journal_mode = local.journal_mode().await.unwrap();
                drop(local);
                assert_eq!(journal_mode, WAL_JOURNAL_MODE);
            }
        });
    }

    #[test]
    fn ordinary_btree_indexes_do_not_enable_custom_index_methods() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("ordinary-index.sqlite");
            let path = path.to_string_lossy().into_owned();
            {
                let local = open_local(&path, Duration::from_secs(1))
                    .await
                    .expect("open Turso WAL database");
                local
                    .connection()
                    .execute("CREATE TABLE records(value TEXT NOT NULL)", ())
                    .await
                    .expect("create table");
                local
                    .connection()
                    .execute("CREATE INDEX records_value ON records(value)", ())
                    .await
                    .expect("create ordinary B-tree index");
                local
                    .connection()
                    .execute("INSERT INTO records VALUES ('preserved')", ())
                    .await
                    .expect("insert indexed row");
                local.flush_to_disk().expect("flush indexed database");
            }

            let reopened = open_local(&path, Duration::from_secs(1))
                .await
                .expect("reopen indexed Turso WAL database");
            let indexed_value = {
                let mut rows = reopened
                    .connection()
                    .query("SELECT value FROM records WHERE value = 'preserved'", ())
                    .await
                    .expect("query reopened ordinary index");
                rows.next()
                    .await
                    .unwrap()
                    .unwrap()
                    .get::<String>(0)
                    .unwrap()
            };
            drop(reopened);
            assert_eq!(indexed_value, "preserved");
        });
    }

    #[test]
    fn offline_maintenance_preserves_the_existing_wal_mode() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("offline-maintenance.sqlite");
            let path = path.to_string_lossy().into_owned();
            let online = open_local(&path, Duration::from_secs(1))
                .await
                .expect("create WAL database before maintenance");
            drop(online);

            let local = open_local_for_offline_maintenance(&path, Duration::from_secs(1), false)
                .await
                .expect("open database for offline maintenance");
            let journal_mode = local.journal_mode().await.unwrap();
            local
                .connection()
                .execute("CREATE TABLE records(value TEXT NOT NULL)", ())
                .await
                .expect("write through offline maintenance connection");
            drop(local);
            assert_eq!(journal_mode, WAL_JOURNAL_MODE);
        });
    }

    #[test]
    fn checkpoints_committed_wal_work() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("checkpoint.sqlite");
            let local = open_local(&path.to_string_lossy(), Duration::from_secs(1))
                .await
                .expect("open Turso WAL database");
            local
                .connection()
                .execute("CREATE TABLE records(value TEXT NOT NULL)", ())
                .await
                .expect("create table");
            local
                .connection()
                .execute("INSERT INTO records VALUES ('preserved')", ())
                .await
                .expect("insert row");

            local.checkpoint().await.expect("checkpoint WAL");

            let checkpointed_value = {
                let mut rows = local
                    .connection()
                    .query("SELECT value FROM records", ())
                    .await
                    .expect("query checkpointed row");
                rows.next()
                    .await
                    .unwrap()
                    .unwrap()
                    .get::<String>(0)
                    .unwrap()
            };
            drop(local);
            assert_eq!(checkpointed_value, "preserved");
        });
    }

    #[test]
    fn a_contended_checkpoint_reports_busy_instead_of_success() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("checkpoint-busy.sqlite");
            let local = open_local(&path.to_string_lossy(), Duration::from_secs(1))
                .await
                .expect("open Turso WAL database");
            local
                .connection()
                .execute("CREATE TABLE records(value TEXT NOT NULL)", ())
                .await
                .expect("create table");
            let blocker = local
                .new_connection(Duration::from_secs(1))
                .expect("open blocking connection");
            blocker.execute("BEGIN", ()).await.expect("begin blocker");
            blocker
                .execute("INSERT INTO records VALUES ('pending')", ())
                .await
                .expect("hold an uncommitted writer");

            let error = local
                .checkpoint()
                .await
                .expect_err("a contended truncate checkpoint is not success");
            drop(local);
            assert!(matches!(error, OpenError::CheckpointBusy), "{error}");

            blocker
                .execute("ROLLBACK", ())
                .await
                .expect("release blocker");
        });
    }

    #[test]
    fn shares_one_database_owner_for_the_same_file_path() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("shared.sqlite");
            let first = open_local(&path.to_string_lossy(), Duration::from_secs(1))
                .await
                .expect("open first handle");
            let second = open_local(&path.to_string_lossy(), Duration::from_secs(1))
                .await
                .expect("open second handle");

            assert!(Arc::ptr_eq(&first.owner, &second.owner));
            first
                .connection()
                .execute("CREATE TABLE records(value TEXT NOT NULL)", ())
                .await
                .expect("create through first handle");
            let mut statement = second
                .connection()
                .prepare_cached("SELECT COUNT(*) FROM records")
                .await
                .expect("prepare empty-table count");
            let mut rows = statement
                .query(())
                .await
                .expect("read empty table through second handle");
            assert_eq!(
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
                0
            );
            while rows.next().await.unwrap().is_some() {}
            first
                .connection()
                .execute("INSERT INTO records VALUES ('first')", ())
                .await
                .expect("write through first handle");
            let mut statement = second
                .connection()
                .prepare_cached("SELECT COUNT(*) FROM records")
                .await
                .expect("prepare refreshed count");
            let mut rows = statement
                .query(())
                .await
                .expect("refresh second handle after first writes");
            assert_eq!(
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
                1
            );
            while rows.next().await.unwrap().is_some() {}
            second
                .connection()
                .execute("INSERT INTO records VALUES ('shared')", ())
                .await
                .expect("write through second handle");
            first
                .connection()
                .execute("INSERT INTO records VALUES ('returned')", ())
                .await
                .expect("write through first handle after second handle");
            let mut rows = second
                .connection()
                .query("SELECT COUNT(*) FROM records", ())
                .await
                .expect("read writes from both handles");
            assert_eq!(
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
                3
            );
            while rows.next().await.unwrap().is_some() {}

            let mut first_connection = first.connection().clone();
            drop(first);
            let transaction = first_connection
                .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
                .await
                .expect("begin transaction through first handle");
            transaction
                .execute("INSERT INTO records VALUES ('transaction')", ())
                .await
                .expect("write transaction through first handle");
            transaction.commit().await.expect("commit transaction");

            let committed_count = {
                let mut rows = second
                    .connection()
                    .query("SELECT COUNT(*) FROM records", ())
                    .await
                    .expect("read committed transaction from second handle");
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
            };
            drop(second);
            assert_eq!(committed_count, 4);
        });
    }

    #[test]
    fn opens_existing_wal_database_without_losing_rows() {
        futures_lite::future::block_on(async {
            let temp = tempfile::tempdir().expect("tempdir");
            let path = temp.path().join("legacy.sqlite");
            let path_text = path.to_string_lossy();
            let legacy = turso::Builder::new_local(&path_text)
                .build()
                .await
                .expect("open legacy database");
            let connection = legacy.connect().expect("connect legacy database");
            connection
                .pragma_update("journal_mode", "'wal'")
                .await
                .expect("enable WAL");
            connection
                .execute("CREATE TABLE records(value TEXT NOT NULL)", ())
                .await
                .expect("create table");
            connection
                .execute("INSERT INTO records VALUES ('preserved')", ())
                .await
                .expect("insert row");
            drop(connection);
            drop(legacy);

            let local = open_local(&path_text, Duration::from_secs(1))
                .await
                .expect("open existing WAL database");
            let journal_mode = local.journal_mode().await.unwrap();
            let preserved_value = {
                let mut rows = local
                    .connection()
                    .query("SELECT value FROM records", ())
                    .await
                    .expect("query preserved row");
                rows.next()
                    .await
                    .unwrap()
                    .unwrap()
                    .get::<String>(0)
                    .unwrap()
            };
            drop(local);
            assert_eq!(journal_mode, WAL_JOURNAL_MODE);
            assert_eq!(preserved_value, "preserved");
        });
    }
}
