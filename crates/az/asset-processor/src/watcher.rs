//! Continuous filesystem watching for registered asset source roots.
//!
//! The `reconcileAssetSources` RPC scans every registered source root once on
//! demand. This module keeps the registry current between those calls by
//! translating sound file events into exact [`SweepScope::Paths`] deltas.
//! Missing exact files become removals; create or modify events read only the
//! named file and metadata sidecar. Overflow, re-arm, directory-shaped, and
//! otherwise unsound observations escalate to [`SweepScope::Root`].
//!
//! # Architecture
//!
//! `spawn_source_watcher` starts one dedicated OS thread per RPC server. That
//! thread:
//!
//! 1. Receives its own `AssetDb` runtime handle from the RPC service's
//!    already-open database owner.
//! 2. Takes one immutable registered-root snapshot at startup, arms each root
//!    (or its nearest existing ancestor when the root is absent), and re-arms
//!    the relevant OS watch after filesystem events or an overflow/rescan.
//!    Builder-catalog and reconcile-completion coordination wakes do not
//!    re-query topology: they only release already-recorded work.
//! 3. Coalesces bursts of raw filesystem events per source root with a
//!    [`SourceChangeDebouncer`] quiet window, then dispatches one scoped
//!    `reconcile_registered_source_assets` call per batch of roots that went
//!    quiet.
//!
//! The debounce/coalescing logic (`SourceChangeDebouncer`) and the
//! path-to-root matching (`match_root`, `is_ignored_path`) are pure and unit
//! tested with injected events/timestamps; no real filesystem or sleep is
//! required to exercise them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use az_filesystem::{
    ASSET_DB_DIR_NAME, AZOTH_STATE_DIR_NAME, PRODUCT_CACHE_DIR_NAME, SourcePath, normalize_lexical,
};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, error, info, instrument, warn};

use crate::RegisteredSourceRoot;
use crate::sweep::{SweepHandle, SweepProvenance, SweepRequest, SweepRoot, SweepScope};

/// Quiet window: editors typically emit several write/flush events per save,
/// so bursts inside this window collapse into a single reconcile.
const SOURCE_WATCH_DEBOUNCE_WINDOW: Duration = Duration::from_millis(400);

/// Handle to a running source watcher thread. Mirrors
/// `AssetProcessorRpcServer`'s shutdown shape: both `Drop` and the explicit
/// `stop()` signal the thread to stop and join it, so the thread never
/// outlives this handle.
pub struct SourceWatcherHandle {
    stop: crossbeam_channel::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SourceWatcherHandle {
    pub(crate) fn stop(mut self) -> thread::Result<()> {
        let _ = self.stop.try_send(());
        self.thread.take().map_or(Ok(()), thread::JoinHandle::join)
    }
}

impl Drop for SourceWatcherHandle {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the project workspace source watcher with read access derived from
/// the RPC service's already-open database owner and its composition-owned
/// writer handle.
///
/// Returns an `io::Error` only if the OS thread itself fails to start;
/// per-root arm failures (for example an inaccessible source root) are logged
/// without preventing other roots from being watched. Existing ancestors are
/// watched for absent roots, so recreate events re-arm the root without a
/// retry cadence.
#[instrument(skip(source_roots, sweeps), fields(workspace_id))]
pub fn spawn_source_watcher(
    workspace_id: i64,
    source_roots: Vec<RegisteredSourceRoot>,
    sweeps: SweepHandle,
) -> std::io::Result<SourceWatcherHandle> {
    let (stop, stop_events) = crossbeam_channel::bounded(1);
    let thread = thread::Builder::new()
        .name("asset-source-watcher".to_string())
        .spawn(move || run_source_watcher(workspace_id, source_roots, sweeps, stop_events))?;
    Ok(SourceWatcherHandle {
        stop,
        thread: Some(thread),
    })
}

// Thread entry point: every argument is moved into the spawned source-watcher
// thread and must be owned, so it cannot take these by reference.
#[allow(clippy::needless_pass_by_value)]
fn run_source_watcher(
    workspace_id: i64,
    source_roots: Vec<RegisteredSourceRoot>,
    sweeps: SweepHandle,
    stop_events: crossbeam_channel::Receiver<()>,
) {
    let (event_tx, event_rx) = crossbeam_channel::unbounded::<notify::Result<Event>>();
    let mut watcher = match notify::recommended_watcher(move |result| {
        // This is an unbounded handoff from notify's backend callback into the
        // one watcher thread. `send` cannot block on capacity, and preserving
        // every backend event lets the debouncer perform the only coalescing.
        let _ = event_tx.send(result);
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            error!(
                workspace_id,
                error = %error,
                "asset source watcher failed to create filesystem watcher"
            );
            return;
        }
    };

    let mut watched_roots: Vec<WatchedRoot> = Vec::new();
    // Roots that were armed and are not any more. An arm that clears an entry
    // here is a *re*-arm, and events were lost while the root was disarmed, so
    // the root is marked dirty. A root that was never armed has no such gap:
    // whoever registered it owns its initial state.
    let mut disarmed: BTreeSet<i64> = BTreeSet::new();
    let mut debouncer = SourceChangeDebouncer::new(SOURCE_WATCH_DEBOUNCE_WINDOW);
    register_initial_watched_roots(
        workspace_id,
        &mut watcher,
        &mut watched_roots,
        &source_roots,
        &mut disarmed,
        &mut debouncer,
    );
    info!(workspace_id, "asset source watcher started");

    loop {
        let mut select = crossbeam_channel::Select::new();
        let fs_index = select.recv(&event_rx);
        let stop_index = select.recv(&stop_events);
        let mut dispatch_due = false;
        let operation = match debouncer.next_deadline() {
            Some(deadline) => {
                let selected = select.select_deadline(deadline).ok();
                dispatch_due = selected.is_none();
                selected
            }
            None => Some(select.select()),
        };
        if let Some(operation) = operation {
            if operation.index() == stop_index {
                let _ = operation.recv(&stop_events);
                break;
            }
            debug_assert_eq!(operation.index(), fs_index);
            match operation.recv(&event_rx) {
                Ok(result) => {
                    record_fs_event(result, workspace_id, &watched_roots, &mut debouncer);
                    // A deleted root may make notify drop its direct watch.
                    // Re-evaluate anchors from this filesystem wake, not a
                    // periodic retry; overflow follows the same path.
                    rearm_watched_roots(
                        workspace_id,
                        &mut watcher,
                        &mut watched_roots,
                        &mut disarmed,
                        &source_roots,
                        &BTreeSet::new(),
                        &mut debouncer,
                    );
                }
                Err(_) => break,
            }
        }

        if !dispatch_due && !debouncer.has_ready(Instant::now()) {
            continue;
        }
        flush_debounced_changes(
            workspace_id,
            &mut watcher,
            &mut watched_roots,
            &mut disarmed,
            &source_roots,
            &mut debouncer,
            &sweeps,
        );
    }

    info!(workspace_id, "asset source watcher stopped");
}

/// Drains whatever the debouncer has ready, re-arms the watches that burst may
/// have killed, and submits one scoped sweep per affected source root.
///
/// A ready set whose source roots are not registered yet is parked rather than
/// dropped, so the signal survives until registration becomes visible.
fn flush_debounced_changes(
    workspace_id: i64,
    watcher: &mut RecommendedWatcher,
    watched_roots: &mut Vec<WatchedRoot>,
    disarmed: &mut BTreeSet<i64>,
    source_roots: &[RegisteredSourceRoot],
    debouncer: &mut SourceChangeDebouncer,
    sweeps: &SweepHandle,
) {
    let ready = debouncer.drain_ready(Instant::now());
    if ready.is_empty() {
        return;
    }
    let affected: Vec<RegisteredSourceRoot> = source_roots
        .iter()
        .filter(|root| ready.contains_key(&root.workspace_root_pk))
        .cloned()
        .collect();
    if affected.is_empty() {
        debug!(
            workspace_id,
            source_root_count = ready.len(),
            "asset source watcher deferred a change signal until its source-root registration is visible"
        );
        debouncer.park_ready(&ready);
        return;
    }
    let forced = forced_watch_renewals(&affected, &ready);
    rearm_watched_roots(
        workspace_id,
        watcher,
        watched_roots,
        disarmed,
        source_roots,
        &forced,
        debouncer,
    );
    let dispatch = dispatch_watch_sweeps(sweeps, &affected, &ready);
    if let Err(error) = retain_ready_on_dispatch_error(debouncer, &ready, dispatch) {
        error!(workspace_id, %error, "asset source watcher failed to submit sweep");
    }
}

fn forced_watch_renewals(
    affected: &[RegisteredSourceRoot],
    ready: &BTreeMap<i64, SweepScope>,
) -> BTreeSet<i64> {
    affected
        .iter()
        .filter(|root| {
            cfg!(windows) && matches!(ready.get(&root.workspace_root_pk), Some(SweepScope::Root))
        })
        .map(|root| root.workspace_root_pk)
        .collect()
}

fn record_fs_event(
    result: notify::Result<Event>,
    workspace_id: i64,
    watched_roots: &[WatchedRoot],
    debouncer: &mut SourceChangeDebouncer,
) {
    match result {
        Ok(event) => handle_fs_event(&event, watched_roots, debouncer, Instant::now()),
        Err(error) => {
            warn!(
                workspace_id,
                error = %error,
                "asset source watcher observed an error event; scheduling every watched root for rescan"
            );
            let now = Instant::now();
            for root in watched_roots {
                debouncer.record(root.workspace_root_pk, now);
            }
        }
    }
}

/// Takes the startup-owned registered-root snapshot and arms every requested
/// root. Source-root registration is not a live topology feed: builder
/// catalog and reconcile coordination wake this watcher only to release
/// already-known work.
///
/// An absent root is watched through its nearest existing ancestor. That
/// anchor sees the recreate event, at which point [`rearm_watched_roots`]
/// replaces it with the direct root watch without a retry timer.
fn register_initial_watched_roots(
    workspace_id: i64,
    watcher: &mut RecommendedWatcher,
    watched_roots: &mut Vec<WatchedRoot>,
    source_roots: &[RegisteredSourceRoot],
    disarmed: &mut BTreeSet<i64>,
    debouncer: &mut SourceChangeDebouncer,
) {
    rearm_watched_roots(
        workspace_id,
        watcher,
        watched_roots,
        disarmed,
        source_roots,
        &source_roots
            .iter()
            .map(|root| root.workspace_root_pk)
            .collect(),
        debouncer,
    );
}

/// Re-arms OS watches from the immutable registered-root snapshot.
///
/// `ReadDirectoryChangesW` reports a buffer overrun as an error code
/// `notify` neither forwards nor recovers from: it logs, drops the watch and
/// returns (`notify-8.2.0/src/windows.rs`, the catch-all arm), so the root
/// goes unwatched for the life of the process and az never learns. There is no
/// liveness question notify's public API can answer, so a filesystem event
/// re-evaluates the anchor and a sweep forcibly renews its affected root.
/// The latter is safe because:
///
/// 1. A sweep is the aftermath of exactly the event burst that can overrun the
///    buffer, so this is when a watch is most likely to have died.
/// 2. The sweep that follows re-reads the whole root, so a change landing in
///    the sub-millisecond window between the unwatch and the re-arm cannot be
///    lost — which is why no scope escalation is owed for it.
///
/// Logical roots may share one physical ancestor arm. We diff physical arms
/// before calling notify: unwatching one logical root must never silently
/// remove another root's shared OS registration.
///
/// A root whose re-arm fails remains disarmed. The next filesystem or
/// coordination wake tries its existing ancestor again; no cadence is used.
fn rearm_watched_roots(
    workspace_id: i64,
    watcher: &mut RecommendedWatcher,
    watched_roots: &mut Vec<WatchedRoot>,
    disarmed: &mut BTreeSet<i64>,
    source_roots: &[RegisteredSourceRoot],
    force_ids: &BTreeSet<i64>,
    debouncer: &mut SourceChangeDebouncer,
) {
    let previous_by_id: BTreeMap<i64, WatchedRoot> = watched_roots
        .iter()
        .cloned()
        .map(|root| (root.workspace_root_pk, root))
        .collect();
    let mut candidates = build_watched_roots(source_roots);
    let mut desired_by_id = BTreeMap::new();
    for candidate in &mut candidates {
        let Some(arm) = desired_watch_arm(&candidate.path, candidate.recursive) else {
            warn!(
                workspace_id,
                workspace_source_root_id = candidate.workspace_root_pk,
                path = %candidate.path.display(),
                "asset source watcher found no existing ancestor for source root"
            );
            disarmed.insert(candidate.workspace_root_pk);
            continue;
        };
        desired_by_id.insert(candidate.workspace_root_pk, arm);
    }
    let desired_mode_by_path = coalesced_watch_modes(desired_by_id.values().cloned());
    for candidate in &mut candidates {
        let Some(arm) = desired_by_id.get(&candidate.workspace_root_pk) else {
            continue;
        };
        candidate.arm = WatchArm {
            path: arm.path.clone(),
            recursive: desired_mode_by_path[&arm.path],
        };
    }
    let desired_arms: BTreeSet<WatchArm> = desired_by_id
        .values()
        .map(|arm| WatchArm {
            path: arm.path.clone(),
            recursive: desired_mode_by_path[&arm.path],
        })
        .collect();
    let current_arms: BTreeSet<WatchArm> =
        coalesced_watch_modes(watched_roots.iter().map(|root| root.arm.clone()))
            .into_iter()
            .map(|(path, recursive)| WatchArm { path, recursive })
            .collect();
    let forced_arms: BTreeSet<WatchArm> = candidates
        .iter()
        .filter(|root| {
            desired_by_id.contains_key(&root.workspace_root_pk)
                && force_ids.contains(&root.workspace_root_pk)
        })
        .map(|root| root.arm.clone())
        .collect();
    let (armed_arms, arms_to_replace) = apply_watch_arms(
        workspace_id,
        watcher,
        current_arms,
        &desired_arms,
        &forced_arms,
    );
    let mut next = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !desired_by_id.contains_key(&candidate.workspace_root_pk) {
            continue;
        }
        if !armed_arms.contains(&candidate.arm) {
            disarmed.insert(candidate.workspace_root_pk);
            continue;
        }
        let previous = previous_by_id.get(&candidate.workspace_root_pk);
        let was_disarmed = disarmed.remove(&candidate.workspace_root_pk);
        let moved_anchor = previous.is_some_and(|root| root.arm != candidate.arm);
        // The forced caller is about to reconcile its own root, so that sweep
        // covers the tiny re-arm gap. A different logical root sharing the
        // physical arm needs its own dirty signal.
        let arm_replaced = arms_to_replace.contains(&candidate.arm)
            && !force_ids.contains(&candidate.workspace_root_pk);
        if was_disarmed || moved_anchor || arm_replaced {
            debouncer.record(candidate.workspace_root_pk, Instant::now());
        }
        next.push(candidate);
    }
    *watched_roots = next;
}

/// Moves the OS registrations from `current_arms` to `desired_arms`.
///
/// An arm in `forced_arms` is torn down and re-armed even when it is already
/// desired, which is how a sweep renews a watch that an event-buffer overrun
/// may have silently killed.
///
/// Returns the arms that are actually armed afterwards, plus the arms that were
/// torn down — a caller needs the latter to decide which logical roots lost
/// events across the gap.
fn apply_watch_arms(
    workspace_id: i64,
    watcher: &mut RecommendedWatcher,
    current_arms: BTreeSet<WatchArm>,
    desired_arms: &BTreeSet<WatchArm>,
    forced_arms: &BTreeSet<WatchArm>,
) -> (BTreeSet<WatchArm>, Vec<WatchArm>) {
    let arms_to_replace: Vec<WatchArm> = current_arms
        .iter()
        .filter(|arm| !desired_arms.contains(*arm) || forced_arms.contains(*arm))
        .cloned()
        .collect();
    let mut armed_arms = current_arms;
    for arm in &arms_to_replace {
        let _ = watcher.unwatch(&arm.path);
        armed_arms.remove(arm);
    }
    let arms_to_arm = desired_arms
        .iter()
        .filter(|arm| !armed_arms.contains(*arm) || forced_arms.contains(*arm))
        .cloned()
        .collect::<Vec<_>>();
    for arm in &arms_to_arm {
        let mode = if arm.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        match watcher.watch(&arm.path, mode) {
            Ok(()) => {
                armed_arms.insert(arm.clone());
            }
            Err(error) => {
                warn!(
                    workspace_id,
                    armed_path = %arm.path.display(),
                    error = %error,
                    "asset source watcher failed to arm source-root watch"
                );
                armed_arms.remove(arm);
            }
        }
    }
    (armed_arms, arms_to_replace)
}

/// Runs one scoped reconcile over `affected`.
///
/// Returns whether the sweep consumed the change signal. A `false` return
/// means the caller still owns the scope and must union it back into the
/// debouncer — dropping it is how a project that goes quiet stops rescanning.
fn dispatch_watch_sweeps(
    sweeps: &SweepHandle,
    affected: &[RegisteredSourceRoot],
    scopes: &BTreeMap<i64, SweepScope>,
) -> Result<(), crate::AssetProcessorError> {
    for root in affected {
        let scope = scopes.get(&root.workspace_root_pk).cloned().ok_or(
            crate::AssetProcessorError::MissingSweepScope {
                workspace_root_id: root.workspace_root_pk,
            },
        )?;
        sweeps.schedule(SweepRequest {
            root: SweepRoot::registered(root),
            scope,
            provenance: SweepProvenance::Watcher,
        })?;
    }
    Ok(())
}

fn retain_ready_on_dispatch_error(
    debouncer: &mut SourceChangeDebouncer,
    ready: &BTreeMap<i64, SweepScope>,
    result: Result<(), crate::AssetProcessorError>,
) -> Result<(), crate::AssetProcessorError> {
    if result.is_err() {
        // Submission may have accepted an earlier root before a later root
        // saturated the owner. Re-park the full map: accepted scopes
        // coalesce idempotently, while the rejected scope must not disappear.
        debouncer.park_ready(ready);
    }
    result
}

#[cfg(test)]
fn dispatch_watch_reconcile(
    db: &az_assetdb::AssetDb,
    writer: &az_assetdb::AssetDbWriter,
    source_roots: &[RegisteredSourceRoot],
    affected: &[RegisteredSourceRoot],
    scopes: &BTreeMap<i64, SweepScope>,
    classifiers: &crate::SourceAssetClassifiers,
    _coordination: &crate::AssetSourceServiceCoordination,
) -> bool {
    crate::reconcile_registered_source_assets_scoped(
        crate::ReconcilePass {
            db,
            writer,
            changed_by_session: None,
            classifiers,
            now_unix_ms: crate::current_unix_ms_i64().unwrap(),
        },
        source_roots,
        affected,
        Some(scopes),
    )
    .is_ok()
}

/// A source root currently armed for filesystem notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchedRoot {
    workspace_root_pk: i64,
    /// Registered source-root path. This remains the match/reconcile scope
    /// even while its physical arm temporarily points at an ancestor.
    path: PathBuf,
    /// Physical OS watch arm. Multiple logical roots can share one arm.
    arm: WatchArm,
    recursive: bool,
    /// Normalized comparison key used for longest-prefix matching against
    /// event paths; see [`path_key`].
    key: String,
}

/// A unique physical notify registration. Ancestor anchors are deliberately
/// non-recursive: child creation advances one directory at a time rather than
/// recursively watching an arbitrarily broad project or volume root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WatchArm {
    path: PathBuf,
    recursive: bool,
}

/// Coalesces desired physical registrations by path. `notify` owns watches by
/// path, not by `(path, recursive-mode)`, so recursive intent dominates when
/// logical roots collide on one existing directory.
fn coalesced_watch_modes(arms: impl IntoIterator<Item = WatchArm>) -> BTreeMap<PathBuf, bool> {
    let mut modes = BTreeMap::new();
    for arm in arms {
        modes
            .entry(arm.path)
            .and_modify(|recursive| *recursive |= arm.recursive)
            .or_insert(arm.recursive);
    }
    modes
}

fn build_watched_roots(source_roots: &[RegisteredSourceRoot]) -> Vec<WatchedRoot> {
    source_roots
        .iter()
        .filter(|row| row.watch)
        .map(|row| {
            let path = PathBuf::from(&row.path);
            WatchedRoot {
                workspace_root_pk: row.workspace_root_pk,
                recursive: row.recursive,
                key: path_key(&path),
                arm: WatchArm {
                    path: path.clone(),
                    recursive: row.recursive,
                },
                path,
            }
        })
        .collect()
}

/// Returns the nearest existing directory that can carry a watch for `path`.
/// Source roots are absolute project paths, so a missing root normally falls
/// back to one of its ancestors (ultimately its volume root).
fn desired_watch_arm(path: &Path, root_recursive: bool) -> Option<WatchArm> {
    let mut candidate = path.to_path_buf();
    loop {
        if candidate.is_dir() {
            return Some(WatchArm {
                recursive: candidate == path && root_recursive,
                path: candidate,
            });
        }
        if !candidate.pop() {
            return None;
        }
    }
}

/// Normalizes a path into a comparison key: lexically resolved (`.`/`..`)
/// with forward-slash separators, lowercased on Windows where the
/// filesystem is case-insensitive. Used only for matching a raw
/// notify-reported path back to a registered source root -- never persisted
/// or used for DB writes.
fn path_key(path: &Path) -> String {
    let normalized = normalize_lexical(path).to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        normalized.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

/// Cheap guard against watching the asset-processor's own state: the
/// registry DB (and its `-wal`/`-shm` siblings) and the product cache should
/// never live under an authored source root, but if one ever does, changes
/// there must not feed back into a rescan of the source that produced them.
/// This is a coarse component-name check, not an exact path match, which is
/// an intentional and cheap trade-off given source roots are authored
/// directories that should not contain directories with these names.
fn is_ignored_path(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case(AZOTH_STATE_DIR_NAME)
            || name.eq_ignore_ascii_case(ASSET_DB_DIR_NAME)
            || name.eq_ignore_ascii_case(PRODUCT_CACHE_DIR_NAME)
    })
}

/// Finds the most specific (longest-prefix) watched root containing
/// `changed_path_key`, if any.
fn match_root<'a>(watched: &'a [WatchedRoot], changed_path_key: &str) -> Option<&'a WatchedRoot> {
    watched
        .iter()
        .filter(|root| {
            changed_path_key.len() >= root.key.len()
                && changed_path_key.starts_with(root.key.as_str())
                && (changed_path_key.len() == root.key.len()
                    || changed_path_key.as_bytes()[root.key.len()] == b'/')
        })
        .max_by_key(|root| root.key.len())
}

/// Routes one raw filesystem event into the debouncer. Pure access events
/// (reads, with no content change) are ignored; every other event kind
/// records the exact affected source path when the observation is sound.
///
/// The rescan flag is checked *before* the path loop, because that is how the
/// backends signal a dropped queue: inotify emits
/// `EventKind::Other` with `Flag::Rescan` and **zero paths**
/// (`notify-8.2.0/src/inotify.rs`), and fsevent does the same. Iterating
/// `event.paths` first means the loop body never runs and the overflow signal
/// is dropped on the floor. Every watched root is marked dirty, because an
/// overflow says nothing about which paths were lost.
fn handle_fs_event(
    event: &Event,
    watched: &[WatchedRoot],
    debouncer: &mut SourceChangeDebouncer,
    now: Instant,
) {
    if event.need_rescan() {
        for root in watched {
            debouncer.record(root.workspace_root_pk, now);
        }
        return;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    let sound_file_event = matches!(
        event.kind,
        EventKind::Create(notify::event::CreateKind::File)
            | EventKind::Remove(notify::event::RemoveKind::File)
            | EventKind::Modify(
                notify::event::ModifyKind::Data(_)
                    | notify::event::ModifyKind::Metadata(_)
                    | notify::event::ModifyKind::Name(notify::event::RenameMode::Both)
            )
    );
    for path in &event.paths {
        if is_ignored_path(path) {
            continue;
        }
        let key = path_key(path);
        if let Some(root) = match_root(watched, &key) {
            if !sound_file_event || path.is_dir() {
                debouncer.record(root.workspace_root_pk, now);
                continue;
            }
            let Some(relative) = original_relative_path(root, path) else {
                debouncer.record(root.workspace_root_pk, now);
                continue;
            };
            let source_path = relative
                .strip_suffix(crate::SOURCE_META_SIDECAR_SUFFIX)
                .unwrap_or(relative.as_str());
            let Some(canonical_path) = SourcePath::non_empty(source_path) else {
                debouncer.record(root.workspace_root_pk, now);
                continue;
            };
            if debouncer
                .record_observed_path(
                    root.workspace_root_pk,
                    canonical_path,
                    source_path.to_string(),
                    now,
                )
                .is_err()
            {
                debouncer.record(root.workspace_root_pk, now);
            }
        }
    }
}

/// Derives a database path from the original notify path, preserving its
/// spelling. `path_key` is deliberately used only to select the containing
/// root; slicing that comparison key would lowercase Windows paths and is
/// unsafe when Unicode case folding changes the byte length.
fn original_relative_path(root: &WatchedRoot, event_path: &Path) -> Option<String> {
    let normalized_root = normalize_lexical(&root.path);
    let normalized_event = normalize_lexical(event_path);
    let root_components = normalized_root.components().count();
    let relative = normalized_event
        .components()
        .skip(root_components)
        .collect::<PathBuf>();
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Debounces raw filesystem-change signals per source root, coalescing
/// bursts of events (editors frequently emit several writes per save) into
/// a single reconcile trigger once a quiet window has elapsed with no
/// further activity for that root.
///
/// Pure and deterministic: callers drive it with explicit `Instant`s, so
/// tests do not need real sleeps or real filesystem timing.
#[derive(Debug)]
pub struct SourceChangeDebouncer {
    quiet_window: Duration,
    pending: BTreeMap<i64, PendingSweep>,
}

#[derive(Debug)]
struct PendingSweep {
    scope: SweepScope,
    /// Failed scopes remain dirty but have no armed deadline. A subsequent
    /// filesystem or coordination wake releases them; failures never create a
    /// self-sustaining retry cadence.
    deadline: Option<Instant>,
}

impl SourceChangeDebouncer {
    pub(crate) const fn new(quiet_window: Duration) -> Self {
        Self {
            quiet_window,
            pending: BTreeMap::new(),
        }
    }

    /// Records filesystem activity for `source_root_id` observed at `now`,
    /// (re)starting its quiet window. Coalesces bursts: repeated calls for
    /// the same root before it fires only push the deadline forward.
    pub(crate) fn record(&mut self, source_root_id: i64, now: Instant) {
        self.record_scope(source_root_id, SweepScope::Root, now);
    }

    /// Records a whole-path scope directly. Production records observed paths
    /// through [`Self::record_observed_path`], which validates the native
    /// spelling first; only tests drive the pre-validated form.
    #[cfg(test)]
    pub(crate) fn record_path(
        &mut self,
        source_root_id: i64,
        source_path: SourcePath,
        now: Instant,
    ) {
        self.record_scope(source_root_id, SweepScope::path(source_path), now);
    }

    fn record_observed_path(
        &mut self,
        source_root_id: i64,
        source_path: SourcePath,
        native_relative: String,
        now: Instant,
    ) -> Result<(), crate::AssetProcessorError> {
        self.record_scope(
            source_root_id,
            SweepScope::observed_path(source_path, native_relative)?,
            now,
        );
        Ok(())
    }

    fn record_scope(&mut self, source_root_id: i64, scope: SweepScope, now: Instant) {
        self.pending
            .entry(source_root_id)
            .and_modify(|pending| {
                pending.scope.merge(scope.clone());
                pending.deadline = Some(now + self.quiet_window);
            })
            .or_insert(PendingSweep {
                scope,
                deadline: Some(now + self.quiet_window),
            });
    }

    /// Removes and returns every root whose quiet window has elapsed as of
    /// `now`, in ascending root-id order.
    pub(crate) fn drain_ready(&mut self, now: Instant) -> BTreeMap<i64, SweepScope> {
        let ready_ids: Vec<i64> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline.is_some_and(|deadline| deadline <= now))
            .map(|(id, _)| *id)
            .collect();
        let mut ready = BTreeMap::new();
        for id in ready_ids {
            if let Some(pending) = self.pending.remove(&id) {
                ready.insert(id, pending.scope);
            }
        }
        ready
    }

    /// Keep a failed ready set dirty without creating another deadline.
    pub(crate) fn park_ready(&mut self, ready: &BTreeMap<i64, SweepScope>) {
        for (source_root_id, scope) in ready {
            self.pending
                .entry(*source_root_id)
                .and_modify(|pending| {
                    pending.scope.merge(scope.clone());
                    pending.deadline = None;
                })
                .or_insert_with(|| PendingSweep {
                    scope: scope.clone(),
                    deadline: None,
                });
        }
    }

    /// Makes parked failures immediately eligible after a causal
    /// coordination wake. Filesystem events use [`Self::record`] instead so
    /// ordinary burst coalescing still applies.
    ///
    /// The production wake path re-arms through [`Self::record`]; this direct
    /// release is exercised only by the debouncer's own tests.
    #[cfg(test)]
    pub(crate) fn release_parked(&mut self, now: Instant) {
        for pending in self.pending.values_mut() {
            if pending.deadline.is_none() {
                pending.deadline = Some(now);
            }
        }
    }

    /// Assertion helper: the loop itself schedules from [`Self::next_deadline`]
    /// and [`Self::has_ready`], so emptiness is only ever observed by tests.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Whether a pending scope is ready to dispatch without consuming it.
    pub(crate) fn has_ready(&self, now: Instant) -> bool {
        self.pending
            .values()
            .any(|pending| pending.deadline.is_some_and(|deadline| deadline <= now))
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .values()
            .filter_map(|pending| pending.deadline)
            .min()
    }

    /// Assertion helper: nothing in the loop branches on how many roots are
    /// pending, only on whether any is ready.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_assetdb::{
        AssetDb, AssetDbWriter, Diff, Digest, Exclusions, RegisterWorkspace, RegisterWorkspaceRoot,
        WorkspaceEntrySnapshot, WorkspaceKey,
    };

    use crate::SourceRootRole;

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        Event {
            kind,
            paths: paths.iter().map(PathBuf::from).collect(),
            attrs: notify::event::EventAttributes::new(),
        }
    }

    fn create_event(paths: &[&str]) -> Event {
        event(EventKind::Create(notify::event::CreateKind::File), paths)
    }

    fn root_scope(id: i64) -> BTreeMap<i64, SweepScope> {
        BTreeMap::from([(id, SweepScope::Root)])
    }

    fn path_scope(id: i64, paths: &[&str]) -> BTreeMap<i64, SweepScope> {
        let mut paths = paths.iter();
        let first = paths.next().expect("path scope is non-empty");
        let mut scope =
            SweepScope::observed_path(SourcePath::new(first), (*first).to_string()).unwrap();
        for path in paths {
            scope.merge(
                SweepScope::observed_path(SourcePath::new(path), (*path).to_string()).unwrap(),
            );
        }
        BTreeMap::from([(id, scope)])
    }

    fn path_scope_with_native(id: i64, paths: &[(&str, &str)]) -> BTreeMap<i64, SweepScope> {
        let mut paths = paths.iter();
        let (first_key, first_native) = paths.next().expect("path scope is non-empty");
        let mut scope =
            SweepScope::observed_path(SourcePath::new(first_key), (*first_native).to_string())
                .unwrap();
        for (key, native) in paths {
            scope.merge(
                SweepScope::observed_path(SourcePath::new(key), (*native).to_string()).unwrap(),
            );
        }
        BTreeMap::from([(id, scope)])
    }

    fn watched(id: i64, path: &str) -> WatchedRoot {
        let path = PathBuf::from(path);
        WatchedRoot {
            workspace_root_pk: id,
            recursive: true,
            key: path_key(&path),
            arm: WatchArm {
                path: path.clone(),
                recursive: true,
            },
            path,
        }
    }

    fn source_root_row(id: i64, path: &str) -> RegisteredSourceRoot {
        RegisteredSourceRoot {
            workspace_pk: 1,
            workspace_root_pk: id,
            root_pk: id,
            id: format!("root-{id}"),
            owner: String::new(),
            path: path.to_owned(),
            display_name: String::new(),
            portable_key: String::new(),
            mount: String::new(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: Exclusions::default(),
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        }
    }

    fn register_source_root(
        db: &AssetDb,
        project: &str,
        workspace_root: &Path,
        source_root: &Path,
        branch: &str,
    ) -> (AssetDbWriter, i64, RegisteredSourceRoot) {
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: project.to_string(),
                    root: workspace_root.to_string_lossy().into_owned(),
                    branch: branch.to_string(),
                },
                now: 10,
            })
            .wait_blocking()
            .unwrap();
        let key = format!("project:{project}:assets");
        let (root, policy) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: key.clone(),
                owner: project.to_string(),
                path: source_root.to_string_lossy().into_owned(),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        (
            writer,
            workspace.workspace_id,
            RegisteredSourceRoot {
                workspace_pk: workspace.workspace_id,
                workspace_root_pk: policy.workspace_root_id,
                root_pk: root.root_id,
                id: key.clone(),
                owner: project.to_string(),
                path: policy.path,
                display_name: "Project Assets".to_string(),
                portable_key: key,
                mount: "@assets@".to_string(),
                recursive: true,
                watch: true,
                writable: true,
                exclusions: policy.exclusions,
                output_prefix: String::new(),
                role: SourceRootRole::ProjectAssets,
            },
        )
    }

    #[test]
    fn debouncer_coalesces_repeated_activity_into_one_pending_deadline() {
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        debouncer.record(1, t0);
        assert_eq!(debouncer.pending_len(), 1);
        // A second event before the window elapses just pushes the deadline.
        debouncer.record(1, t0 + Duration::from_millis(50));
        assert_eq!(debouncer.pending_len(), 1);

        // Not ready yet at the original deadline...
        assert!(
            debouncer
                .drain_ready(t0 + Duration::from_millis(100))
                .is_empty()
        );
        // ...but ready once the *rescheduled* deadline elapses.
        assert_eq!(
            debouncer.drain_ready(t0 + Duration::from_millis(150)),
            root_scope(1)
        );
        assert!(debouncer.is_empty());
    }

    #[test]
    fn debouncer_drains_multiple_roots_independently() {
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        debouncer.record(1, t0);
        debouncer.record(2, t0 + Duration::from_millis(80));

        let ready = debouncer.drain_ready(t0 + Duration::from_millis(100));
        assert_eq!(ready, root_scope(1));
        assert_eq!(debouncer.pending_len(), 1);

        let ready = debouncer.drain_ready(t0 + Duration::from_millis(200));
        assert_eq!(ready, root_scope(2));
        assert!(debouncer.is_empty());
    }

    #[test]
    fn debouncer_drain_ready_is_a_noop_when_nothing_is_pending() {
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        assert!(debouncer.drain_ready(Instant::now()).is_empty());
    }

    #[test]
    fn failed_reconcile_parks_its_scope_until_a_causal_wake() {
        let t0 = Instant::now();
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(50));
        debouncer.record_path(9, SourcePath::new("a.ron"), t0);
        debouncer.record_path(9, SourcePath::new("b.ron"), t0);

        let ready = debouncer.drain_ready(t0 + Duration::from_millis(50));
        assert_eq!(ready, path_scope(9, &["a.ron", "b.ron"]));
        debouncer.park_ready(&ready);

        assert!(!debouncer.is_empty());
        assert!(
            debouncer
                .drain_ready(t0 + Duration::from_secs(1))
                .is_empty()
        );
        debouncer.release_parked(t0 + Duration::from_secs(1));
        assert_eq!(
            debouncer.drain_ready(t0 + Duration::from_secs(1)),
            path_scope(9, &["a.ron", "b.ron"])
        );
    }

    #[test]
    fn owner_overload_reparks_the_entire_ready_scope() {
        let t0 = Instant::now();
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(50));
        debouncer.record_path(9, SourcePath::new("a.ron"), t0);
        debouncer.record_path(9, SourcePath::new("b.ron"), t0);
        let ready = debouncer.drain_ready(t0 + Duration::from_millis(50));

        let result = retain_ready_on_dispatch_error(
            &mut debouncer,
            &ready,
            Err(crate::AssetProcessorError::SweepOwnerOverloaded { capacity: 1 }),
        );

        assert!(matches!(
            result,
            Err(crate::AssetProcessorError::SweepOwnerOverloaded { .. })
        ));
        assert!(
            debouncer
                .drain_ready(t0 + Duration::from_secs(1))
                .is_empty()
        );
        debouncer.release_parked(t0 + Duration::from_secs(1));
        assert_eq!(debouncer.drain_ready(t0 + Duration::from_secs(1)), ready);
    }

    #[test]
    fn match_root_prefers_longest_prefix_for_nested_roots() {
        let watched = vec![
            watched(1, "/wt/assets"),
            watched(2, "/wt/assets/gems/physics"),
        ];
        let outer = match_root(&watched, &path_key(Path::new("/wt/assets/textures/a.png")));
        assert_eq!(outer.unwrap().workspace_root_pk, 1);

        let inner = match_root(
            &watched,
            &path_key(Path::new("/wt/assets/gems/physics/materials/a.mtl")),
        );
        assert_eq!(inner.unwrap().workspace_root_pk, 2);
    }

    #[test]
    fn match_root_does_not_match_a_sibling_with_a_shared_prefix() {
        let watched = vec![watched(1, "/wt/assets")];
        let result = match_root(&watched, &path_key(Path::new("/wt/assets-other/a.ron")));
        assert!(result.is_none());
    }

    #[test]
    fn match_root_matches_the_root_path_itself() {
        let watched = vec![watched(1, "/wt/assets")];
        let result = match_root(&watched, &path_key(Path::new("/wt/assets")));
        assert_eq!(result.unwrap().workspace_root_pk, 1);
    }

    #[test]
    fn is_ignored_path_skips_asset_db_and_product_cache_components() {
        assert!(is_ignored_path(Path::new(
            "/wt/assets/.azoth/assetdb/assetdb.sqlite"
        )));
        assert!(is_ignored_path(Path::new("/wt/assets/Cache/pc/foo.bin")));
        assert!(!is_ignored_path(Path::new("/wt/assets/textures/a.png")));
    }

    #[test]
    fn handle_fs_event_ignores_pure_access_events() {
        let watched = vec![watched(1, "/wt/assets")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        let access = event(
            EventKind::Access(notify::event::AccessKind::Read),
            &["/wt/assets/textures/a.png"],
        );
        handle_fs_event(&access, &watched, &mut debouncer, Instant::now());
        assert!(debouncer.is_empty());
    }

    #[test]
    fn handle_fs_event_records_the_matched_root_for_create_events() {
        let watched = vec![watched(7, "/wt/assets")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        let now = Instant::now();
        let created = create_event(&["/wt/assets/textures/new.png"]);
        handle_fs_event(&created, &watched, &mut debouncer, now);
        assert_eq!(
            debouncer.drain_ready(now + Duration::from_millis(100)),
            path_scope(7, &["textures/new.png"])
        );
    }

    #[cfg(windows)]
    #[test]
    fn handle_fs_event_preserves_original_windows_path_spelling() {
        let temp = tempfile::tempdir().unwrap();
        let watched_root = temp.path().join("Workspace/Assets");
        let watched_root = watched_root.to_string_lossy().replace('\\', "/");
        let event_path = format!("{}/Textures/Foo.PNG", watched_root.to_ascii_lowercase());
        let watched = vec![watched(7, &watched_root)];
        assert_eq!(
            original_relative_path(&watched[0], Path::new(&event_path)).as_deref(),
            Some("Textures/Foo.PNG")
        );
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);
        let now = Instant::now();
        let created = create_event(&[&event_path]);
        handle_fs_event(&created, &watched, &mut debouncer, now);
        assert_eq!(
            debouncer.drain_ready(now),
            BTreeMap::from([(
                7,
                SweepScope::observed_path(
                    SourcePath::new("textures/foo.png"),
                    "Textures/Foo.PNG".to_string(),
                )
                .unwrap(),
            )])
        );
    }

    #[test]
    fn debouncer_coalesces_paths_and_root_escalation_absorbs_them() {
        let now = Instant::now();
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);
        debouncer.record_path(7, SourcePath::new("a.ron"), now);
        debouncer.record_path(7, SourcePath::new("b.ron"), now);
        assert_eq!(
            debouncer.drain_ready(now),
            path_scope(7, &["a.ron", "b.ron"])
        );

        debouncer.record_path(7, SourcePath::new("a.ron"), now);
        debouncer.record(7, now);
        debouncer.record_path(7, SourcePath::new("b.ron"), now);
        assert_eq!(debouncer.drain_ready(now), root_scope(7));
    }

    #[test]
    fn metadata_sidecar_event_enqueues_its_owning_source_path() {
        let watched = vec![watched(7, "/wt/assets")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);
        let now = Instant::now();
        handle_fs_event(
            &event(
                EventKind::Modify(notify::event::ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                &["/wt/assets/prefabs/a.prefab.ron.azmeta.json"],
            ),
            &watched,
            &mut debouncer,
            now,
        );
        assert_eq!(
            debouncer.drain_ready(now),
            path_scope(7, &["prefabs/a.prefab.ron"])
        );
    }

    #[test]
    fn rename_event_retains_both_old_and_new_source_paths() {
        let watched = vec![watched(7, "/wt/assets")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);
        let now = Instant::now();
        handle_fs_event(
            &event(
                EventKind::Modify(notify::event::ModifyKind::Name(
                    notify::event::RenameMode::Both,
                )),
                &["/wt/assets/old.ron", "/wt/assets/new.ron"],
            ),
            &watched,
            &mut debouncer,
            now,
        );
        assert_eq!(
            debouncer.drain_ready(now),
            path_scope(7, &["old.ron", "new.ron"])
        );
    }

    #[test]
    fn directory_shaped_event_escalates_only_its_root() {
        let watched = vec![watched(7, "/wt/assets"), watched(8, "/wt/gems")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);
        let now = Instant::now();
        handle_fs_event(
            &event(
                EventKind::Remove(notify::event::RemoveKind::Folder),
                &["/wt/assets/prefabs"],
            ),
            &watched,
            &mut debouncer,
            now,
        );
        assert_eq!(debouncer.drain_ready(now), root_scope(7));
    }

    #[test]
    fn handle_fs_event_ignores_unmatched_and_product_cache_paths() {
        let watched = vec![watched(7, "/wt/assets")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        handle_fs_event(
            &create_event(&["/elsewhere/file.png"]),
            &watched,
            &mut debouncer,
            Instant::now(),
        );
        handle_fs_event(
            &create_event(&["/wt/assets/Cache/pc/product.bin"]),
            &watched,
            &mut debouncer,
            Instant::now(),
        );
        assert!(debouncer.is_empty());
    }

    /// A dropped-event signal carries no paths at all
    /// (`notify-8.2.0/src/inotify.rs`: `EventKind::Other` + `Flag::Rescan`),
    /// so a handler that starts by iterating `event.paths` never runs its body
    /// and the overflow is lost. The flag must be read before the loop.
    #[test]
    fn handle_fs_event_escalates_a_zero_path_rescan_to_every_watched_root() {
        let watched = vec![watched(1, "/wt/assets"), watched(2, "/wt/gems/physics")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        let overflow = Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        assert!(
            overflow.paths.is_empty(),
            "the backends signal a dropped queue with no paths; a path loop cannot see it"
        );

        handle_fs_event(&overflow, &watched, &mut debouncer, Instant::now());

        assert_eq!(debouncer.pending_len(), 2);
    }

    #[test]
    fn watcher_error_marks_every_logical_root_dirty() {
        let watched = vec![watched(1, "/wt/assets"), watched(2, "/wt/gems/physics")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));

        record_fs_event(
            Err(notify::Error::generic("backend watch lost")),
            1,
            &watched,
            &mut debouncer,
        );

        assert_eq!(debouncer.pending_len(), 2);
    }

    /// An ordinary `Other` event without the rescan flag is not an overflow and
    /// must not restage every root.
    #[test]
    fn handle_fs_event_does_not_escalate_an_unflagged_event_without_paths() {
        let watched = vec![watched(1, "/wt/assets")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        handle_fs_event(
            &Event::new(EventKind::Other),
            &watched,
            &mut debouncer,
            Instant::now(),
        );
        assert!(debouncer.is_empty());
    }

    #[test]
    fn root_sweep_renews_only_backends_that_drop_recursive_watches() {
        let affected = vec![source_root_row(7, "/wt/assets")];
        let ready = root_scope(7);

        let renewals = forced_watch_renewals(&affected, &ready);

        if cfg!(windows) {
            assert_eq!(renewals, BTreeSet::from([7]));
        } else {
            assert!(
                renewals.is_empty(),
                "inotify preserves its watch across overflow; recursive renewal rescans the full tree"
            );
        }
    }

    /// A deleted root is immediately re-anchored at its existing parent; when
    /// recreated, the same event-driven re-arm returns to the direct root
    /// without a periodic retry.
    #[test]
    fn rearm_watched_roots_tracks_delete_and_recreate_through_an_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let root_path = temp.path().join("assets");
        std::fs::create_dir_all(&root_path).unwrap();
        let mut watcher = notify::recommended_watcher(|_| {}).expect("create watcher");
        watcher
            .watch(&root_path, RecursiveMode::Recursive)
            .expect("arm root");

        let mut watched_roots = vec![WatchedRoot {
            workspace_root_pk: 7,
            recursive: true,
            key: path_key(&root_path),
            arm: WatchArm {
                path: root_path.clone(),
                recursive: true,
            },
            path: root_path.clone(),
        }];
        let source_roots = vec![source_root_row(7, root_path.to_string_lossy().as_ref())];
        let mut disarmed = BTreeSet::new();
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);
        let forced = BTreeSet::from([7]);

        rearm_watched_roots(
            1,
            &mut watcher,
            &mut watched_roots,
            &mut disarmed,
            &source_roots,
            &forced,
            &mut debouncer,
        );
        assert_eq!(watched_roots.len(), 1, "a live root stays armed");
        assert!(disarmed.is_empty());

        std::fs::remove_dir_all(&root_path).unwrap();
        rearm_watched_roots(
            1,
            &mut watcher,
            &mut watched_roots,
            &mut disarmed,
            &source_roots,
            &BTreeSet::new(),
            &mut debouncer,
        );
        assert!(
            watched_roots[0].arm.path == temp.path(),
            "a missing root remains observable through its nearest ancestor"
        );
        assert!(disarmed.is_empty());

        std::fs::create_dir_all(&root_path).unwrap();
        rearm_watched_roots(
            1,
            &mut watcher,
            &mut watched_roots,
            &mut disarmed,
            &source_roots,
            &BTreeSet::new(),
            &mut debouncer,
        );
        assert_eq!(watched_roots[0].arm.path, root_path);
    }

    #[test]
    fn missing_roots_share_one_nonrecursive_ancestor_arm() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("one").join("assets");
        let second = temp.path().join("two").join("assets");
        let first_arm = desired_watch_arm(&first, true).expect("temp ancestor exists");
        let second_arm = desired_watch_arm(&second, true).expect("temp ancestor exists");

        assert_eq!(first_arm, second_arm);
        assert_eq!(first_arm.path, temp.path());
        assert!(
            !first_arm.recursive,
            "ancestor watches advance one level at a time"
        );
    }

    #[test]
    fn direct_root_and_absent_descendant_coalesce_to_one_recursive_physical_arm() {
        let temp = tempfile::tempdir().unwrap();
        let direct = source_root_row(1, temp.path().to_string_lossy().as_ref());
        let descendant = source_root_row(
            2,
            temp.path()
                .join("missing")
                .join("assets")
                .to_string_lossy()
                .as_ref(),
        );
        let source_roots = vec![direct, descendant];
        let mut watcher = notify::recommended_watcher(|_| {}).expect("create watcher");
        let mut watched_roots = Vec::new();
        let mut disarmed = BTreeSet::new();
        let mut debouncer = SourceChangeDebouncer::new(Duration::ZERO);

        rearm_watched_roots(
            1,
            &mut watcher,
            &mut watched_roots,
            &mut disarmed,
            &source_roots,
            &BTreeSet::from([1, 2]),
            &mut debouncer,
        );

        let arms = coalesced_watch_modes(watched_roots.into_iter().map(|root| root.arm));
        assert_eq!(arms.len(), 1, "physical watch identity is path only");
        assert_eq!(arms.get(temp.path()), Some(&true));
    }

    #[test]
    fn missing_root_rearm_advances_one_existing_directory_per_create() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("one").join("assets");
        let initial = desired_watch_arm(&root, true).unwrap();
        assert_eq!(initial.path, temp.path());
        assert!(!initial.recursive);

        std::fs::create_dir(temp.path().join("one")).unwrap();
        let intermediate = desired_watch_arm(&root, true).unwrap();
        assert_eq!(intermediate.path, temp.path().join("one"));
        assert!(!intermediate.recursive);

        std::fs::create_dir(&root).unwrap();
        let direct = desired_watch_arm(&root, true).unwrap();
        assert_eq!(direct.path, root);
        assert!(direct.recursive);
    }

    #[test]
    fn handle_fs_event_records_every_matched_path_on_a_multi_path_event() {
        // Windows rename delivery: one Modify(Name) event per side, but
        // notify's `Event` type supports multiple paths per event too.
        let watched = vec![watched(1, "/wt/assets"), watched(2, "/wt/gems/physics")];
        let mut debouncer = SourceChangeDebouncer::new(Duration::from_millis(100));
        let renamed = event(
            EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            &["/wt/assets/old.ron", "/wt/gems/physics/new.ron"],
        );
        handle_fs_event(&renamed, &watched, &mut debouncer, Instant::now());
        assert_eq!(debouncer.pending_len(), 2);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    /// End-to-end through the exact code path the watcher thread runs per
    /// event -- `handle_fs_event` -> `SourceChangeDebouncer::drain_ready` ->
    /// `dispatch_watch_reconcile` -- with a real registered source root and
    /// real files, but injected events and injected timestamps so nothing
    /// depends on notify backend delivery or wall-clock timing.
    #[test]
    fn watcher_event_dispatch_reconciles_written_and_deleted_sources() {
        let temp = tempfile::tempdir().unwrap();
        let (mut watch, source_file) = WatchDispatch::open(temp.path());
        let source_bytes = b"watched source bytes";
        let source_file_text = source_file.to_string_lossy().into_owned();

        // A file write under the registered root schedules a scoped
        // reconcile once the quiet window elapses.
        let now = Instant::now();
        handle_fs_event(
            &create_event(&[source_file_text.as_str()]),
            &watch.watched_roots,
            &mut watch.debouncer,
            now,
        );
        assert!(watch.debouncer.drain_ready(now).is_empty());
        assert_eq!(watch.dispatch_ready(now), watch.watched_source_scope());
        let entry = watch
            .entry("sources/watched.ron")
            .expect("watcher-dispatched reconcile records the written source");
        assert_eq!(entry.digest, Digest::from(blake3::hash(source_bytes)));
        let preserved_guid = entry.asset_guid;

        // A sidecar-only identity preservation is scoped back to its owning
        // source; the watcher must not ignore the sidecar or walk the whole
        // root. A sidecar cannot replace an already durable stable identity.
        let sidecar_path = crate::source_meta_sidecar_path(&source_file);
        std::fs::write(
            &sidecar_path,
            serde_json::to_vec(&crate::SourceAssetMeta::preserving_source_guid(
                preserved_guid,
            ))
            .unwrap(),
        )
        .unwrap();
        let sidecar_text = sidecar_path.to_string_lossy().into_owned();
        let ready = watch.observe(&event(
            EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            &[sidecar_text.as_str()],
        ));
        assert_eq!(ready, watch.watched_source_scope());
        let (asset, _) = watch
            .db
            .source_asset(
                watch.workspace_pk,
                watch.scan_folder_pk,
                "sources/watched.ron",
            )
            .unwrap()
            .expect("sidecar-scoped source remains addressable");
        assert_eq!(asset.guid, preserved_guid);

        // An unrecognized new file is a path-scoped no-op, not a reason to
        // walk or mutate the rest of the root.
        let ignored_file =
            PathBuf::from(&watch.source_roots[0].path).join("Sources/ignored.unknown");
        std::fs::write(&ignored_file, b"ignored").unwrap();
        let ignored_text = ignored_file.to_string_lossy().into_owned();
        watch.observe(&create_event(&[ignored_text.as_str()]));
        assert!(
            watch
                .db
                .source_asset(
                    watch.workspace_pk,
                    watch.scan_folder_pk,
                    "sources/ignored.unknown"
                )
                .unwrap()
                .is_none()
        );

        // The exact missing-file observation becomes a scoped removal for the
        // already-recorded canonical path; no root iterator is involved.
        std::fs::remove_file(&source_file).unwrap();
        let ready = watch.observe(&event(
            EventKind::Remove(notify::event::RemoveKind::File),
            &[source_file_text.as_str()],
        ));
        assert_eq!(ready, watch.watched_source_scope());
        let entry = watch
            .entry("sources/watched.ron")
            .expect("deleted source keeps its recorded entry row");
        assert_eq!(entry.diff, Diff::Deleted);
    }

    /// One registered source root plus the debouncer that stands in for the
    /// watcher thread, so a scenario can inject an event and dispatch exactly
    /// the reconcile the thread would have run.
    struct WatchDispatch {
        db: AssetDb,
        writer: AssetDbWriter,
        source_roots: Vec<RegisteredSourceRoot>,
        watched_roots: Vec<WatchedRoot>,
        classifiers: crate::SourceAssetClassifiers,
        coordination: crate::AssetSourceServiceCoordination,
        debouncer: SourceChangeDebouncer,
        workspace_pk: i64,
        scan_folder_pk: i64,
    }

    impl WatchDispatch {
        /// Builds the tree, the database and the registration under `root`, and
        /// returns the fixture with the one watched source file it created.
        fn open(root: &Path) -> (Self, PathBuf) {
            const WATCH_TEST_PROJECT_ID: &str = "local.asset_watcher";
            let workspace_root = root.join("workspace");
            let source_root = workspace_root.join("assets");
            std::fs::create_dir_all(source_root.join("Sources")).unwrap();
            let source_file = source_root.join("Sources").join("Watched.RON");
            std::fs::write(&source_file, b"watched source bytes").unwrap();

            let db = AssetDb::open(root.join("asset.db")).unwrap();
            let (writer, workspace_pk, registered) = register_source_root(
                &db,
                WATCH_TEST_PROJECT_ID,
                &workspace_root,
                &source_root,
                "az/session/watcher",
            );
            let scan_folder_pk = registered.root_pk;
            let source_roots = vec![registered];
            let watched_roots = build_watched_roots(&source_roots);
            (
                Self {
                    db,
                    writer,
                    source_roots,
                    watched_roots,
                    classifiers: crate::source_asset_classifiers(
                        None,
                        crate::tests::test_registries(),
                    ),
                    coordination: crate::AssetSourceServiceCoordination::default(),
                    debouncer: SourceChangeDebouncer::new(SOURCE_WATCH_DEBOUNCE_WINDOW),
                    workspace_pk,
                    scan_folder_pk,
                },
                source_file,
            )
        }

        /// Feeds one injected filesystem event through the debouncer, lets the
        /// quiet window elapse, and dispatches whatever became ready.
        fn observe(&mut self, event: &Event) -> BTreeMap<i64, SweepScope> {
            let now = Instant::now();
            handle_fs_event(event, &self.watched_roots, &mut self.debouncer, now);
            self.dispatch_ready(now)
        }

        /// Dispatches the reconcile for everything ready one debounce window
        /// after `now`, asserting the dispatch itself succeeded.
        fn dispatch_ready(&mut self, now: Instant) -> BTreeMap<i64, SweepScope> {
            let ready = self
                .debouncer
                .drain_ready(now + SOURCE_WATCH_DEBOUNCE_WINDOW);
            assert!(dispatch_watch_reconcile(
                &self.db,
                &self.writer,
                &self.source_roots,
                &self.source_roots,
                &ready,
                &self.classifiers,
                &self.coordination,
            ));
            ready
        }

        /// The scope a change to the one watched source file must produce.
        fn watched_source_scope(&self) -> BTreeMap<i64, SweepScope> {
            path_scope_with_native(
                self.source_roots[0].workspace_root_pk,
                &[("sources/watched.ron", "Sources/Watched.RON")],
            )
        }

        /// The recorded workspace entry for `source_path`, if it has one.
        fn entry(&self, source_path: &str) -> Option<WorkspaceEntrySnapshot> {
            self.db
                .workspace_entry_page(self.workspace_pk, Some(&[self.scan_folder_pk]), 0, 100)
                .unwrap()
                .into_iter()
                .find(|entry| entry.source_path == source_path)
        }
    }
}
