//! Project-scoped editor UI-state lifecycle and debounced persistence.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, Global, Subscription, Window, WindowBounds, point, px, size};
use tracing::{info, instrument, warn};

use crate::project_manager_preferences::{
    PersistedWindowBounds, ProjectManagerPreferenceStore, ProjectUiStateSnapshot,
    WorkspaceUiLayoutSnapshot,
};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
pub const EDITOR_MODES: &[&str] = &[
    "scene",
    "materials",
    "scripting",
    "gamedata",
    "sequencer",
    "animation",
    "profiler",
];

#[derive(Debug, Clone)]
struct UiStateCache {
    project: ProjectUiStateSnapshot,
    layouts: BTreeMap<String, WorkspaceUiLayoutSnapshot>,
}

pub struct UiStateBootstrap {
    store: Arc<ProjectManagerPreferenceStore>,
    project_key: Arc<str>,
    cache: UiStateCache,
}

pub struct UiStatePersistence {
    store: Arc<ProjectManagerPreferenceStore>,
    project_key: Arc<str>,
    cache: Arc<Mutex<UiStateCache>>,
    revision: Arc<AtomicU64>,
    write_lock: Arc<Mutex<()>>,
    _quit_subscription: Subscription,
}

impl Global for UiStatePersistence {}

#[instrument(skip_all, fields(project_root = ?project_root))]
pub fn load_bootstrap(project_root: Option<&Path>) -> Option<UiStateBootstrap> {
    let project_root = project_root?;
    let project_key: Arc<str> = normalized_project_key(project_root).into();
    let store = match ProjectManagerPreferenceStore::open_global() {
        Ok(store) => Arc::new(store),
        Err(error) => {
            warn!(%error, "failed to open editor UI-state store; using design defaults");
            return None;
        }
    };

    let mut discarded = 0usize;
    let mut project = match store.load_project_ui_state(&project_key) {
        Ok(Some(project)) if valid_project_state(&project) => project,
        Ok(Some(_)) | Err(_) => {
            discarded += 1;
            let _ = store.clear_project_ui_state(&project_key);
            ProjectUiStateSnapshot::default()
        }
        Ok(None) => ProjectUiStateSnapshot::default(),
    };
    if !valid_mode(&project.last_mode) {
        "scene".clone_into(&mut project.last_mode);
    }

    let mut layouts = BTreeMap::new();
    for mode in EDITOR_MODES {
        match store.load_workspace_layout(&project_key, mode) {
            Ok(Some(layout)) if valid_layout_for_mode(mode, &layout) => {
                layouts.insert((*mode).to_owned(), layout);
            }
            // An incompatible layout and a failed read are both discarded and
            // cleared; only a clean absence is left alone.
            Ok(Some(_)) | Err(_) => {
                discarded += 1;
                let _ = store.clear_workspace_layout(&project_key, mode);
            }
            Ok(None) => {}
        }
    }
    if discarded > 0 {
        warn!(
            discarded,
            "discarded incompatible or corrupt persisted editor UI state"
        );
    }

    Some(UiStateBootstrap {
        store,
        project_key,
        cache: UiStateCache { project, layouts },
    })
}

pub fn install(cx: &mut App, bootstrap: UiStateBootstrap) {
    let cache = Arc::new(Mutex::new(bootstrap.cache));
    let revision = Arc::new(AtomicU64::new(0));
    let write_lock = Arc::new(Mutex::new(()));
    let store = bootstrap.store.clone();
    let project_key = bootstrap.project_key.clone();
    let quit_cache = cache.clone();
    let quit_write_lock = write_lock.clone();
    let executor = cx.background_executor().clone();
    let quit_subscription = cx.on_app_quit(move |_| {
        let store = store.clone();
        let project_key = project_key.clone();
        let cache = quit_cache.clone();
        let write_lock = quit_write_lock.clone();
        let task = executor
            .spawn(async move { persist_cache(&store, &project_key, &cache, &write_lock, None) });
        async move {
            if let Err(error) = task.await {
                warn!(%error, "failed to flush editor UI state during shutdown");
            }
        }
    });

    cx.set_global(UiStatePersistence {
        store: bootstrap.store,
        project_key: bootstrap.project_key,
        cache,
        revision,
        write_lock,
        _quit_subscription: quit_subscription,
    });
    info!("editor UI-state persistence installed");
}

/// Install project-scoped persistence when the Project Manager attaches a
/// project inside the already-running editor process.
#[instrument(skip_all, fields(project_root = %project_root.display()))]
pub fn install_for_attached_project(project_root: &Path, cx: &mut App) {
    if cx.has_global::<UiStatePersistence>() {
        return;
    }
    if let Some(bootstrap) = load_bootstrap(Some(project_root)) {
        install(cx, bootstrap);
    }
}

pub fn initial_project_state(cx: &App) -> ProjectUiStateSnapshot {
    cx.try_global::<UiStatePersistence>()
        .map(|state| lock_cache(&state.cache).project.clone())
        .unwrap_or_default()
}

/// The persisted workspace mode is also the live mode owner: mode switches
/// update it synchronously before command dispatch can be observed.
pub fn is_gamedata_mode(cx: &App) -> bool {
    cx.try_global::<UiStatePersistence>().is_some_and(|state| {
        mode_owns_gamedata_history(&lock_cache(&state.cache).project.last_mode)
    })
}

fn mode_owns_gamedata_history(mode: &str) -> bool {
    mode == "gamedata"
}

pub fn layout_for_mode(cx: &App, mode: &str) -> Option<WorkspaceUiLayoutSnapshot> {
    cx.try_global::<UiStatePersistence>()
        .and_then(|state| lock_cache(&state.cache).layouts.get(mode).cloned())
}

pub fn update_workspace_state(
    mode: &str,
    layout: WorkspaceUiLayoutSnapshot,
    asset_view_mode: &str,
    asset_folder_key: Option<String>,
    immediate: bool,
    cx: &App,
) {
    let Some(state) = cx.try_global::<UiStatePersistence>() else {
        return;
    };
    {
        let mut cache = lock_cache(&state.cache);
        cache.layouts.insert(mode.to_owned(), layout);
        asset_view_mode.clone_into(&mut cache.project.asset_view_mode);
        cache.project.asset_folder_key = asset_folder_key;
    }
    schedule_write(immediate, cx);
}

pub fn set_last_mode(mode: &str, immediate: bool, cx: &App) {
    if !valid_mode(mode) {
        return;
    }
    let Some(state) = cx.try_global::<UiStatePersistence>() else {
        return;
    };
    mode.clone_into(&mut lock_cache(&state.cache).project.last_mode);
    schedule_write(immediate, cx);
}

pub fn capture_window(window: &Window, cx: &App) {
    let Some(state) = cx.try_global::<UiStatePersistence>() else {
        return;
    };
    lock_cache(&state.cache).project.window = Some(persisted_window_bounds(window));
    schedule_write(false, cx);
}

pub fn reset_layout(mode: &str, cx: &App) {
    let Some(state) = cx.try_global::<UiStatePersistence>() else {
        return;
    };
    lock_cache(&state.cache).layouts.remove(mode);
    // Deletion rides the debounced write: `persist_cache` reconciles the
    // store against the cache, clearing known modes whose layout is gone.
    // A later write bumping the revision therefore cannot skip or resurrect
    // the reset anymore — the winning write commits the deletion.
    schedule_write(false, cx);
}

pub fn restored_window_bounds(cx: &App) -> Option<WindowBounds> {
    let persisted = initial_project_state(cx).window?;
    clamp_window_bounds(persisted, cx)
}

pub fn default_window_bounds(cx: &App) -> WindowBounds {
    let Some(display) = cx.primary_display() else {
        return WindowBounds::Maximized(gpui::Bounds {
            origin: point(px(80.0), px(80.0)),
            size: size(px(1280.0), px(800.0)),
        });
    };
    let display = display.bounds();
    let width = (f32::from(display.size.width) * 0.8).max(960.0);
    let height = (f32::from(display.size.height) * 0.8).max(640.0);
    let x = f32::from(display.origin.x) + (f32::from(display.size.width) - width) / 2.0;
    let y = f32::from(display.origin.y) + (f32::from(display.size.height) - height) / 2.0;
    WindowBounds::Maximized(gpui::Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    })
}

fn schedule_write(immediate: bool, cx: &App) {
    let Some(state) = cx.try_global::<UiStatePersistence>() else {
        return;
    };
    let revision = state.revision.fetch_add(1, Ordering::AcqRel) + 1;
    let current_revision = state.revision.clone();
    let store = state.store.clone();
    let project_key = state.project_key.clone();
    let cache = state.cache.clone();
    let write_lock = state.write_lock.clone();
    let executor = cx.background_executor().clone();
    executor
        .clone()
        .spawn(async move {
            if !immediate {
                executor.timer(SAVE_DEBOUNCE).await;
            }
            if current_revision.load(Ordering::Acquire) != revision {
                return;
            }
            if let Err(error) = persist_cache(
                &store,
                &project_key,
                &cache,
                &write_lock,
                Some((&current_revision, revision)),
            ) {
                warn!(%error, "failed to persist editor UI state");
            }
        })
        .detach();
}

fn persist_cache(
    store: &ProjectManagerPreferenceStore,
    project_key: &str,
    cache: &Mutex<UiStateCache>,
    write_lock: &Mutex<()>,
    expected_revision: Option<(&AtomicU64, u64)>,
) -> Result<(), crate::project_manager_preferences::ProjectManagerPreferenceError> {
    let snapshot = lock_cache(cache).clone();
    let _guard = lock_write(write_lock);
    if expected_revision
        .is_some_and(|(revision, expected)| revision.load(Ordering::Acquire) != expected)
    {
        return Ok(());
    }
    store.save_project_ui_state(project_key, &snapshot.project)?;
    for (mode, layout) in &snapshot.layouts {
        store.save_workspace_layout(project_key, mode, layout)?;
    }
    // The cache is the source of truth for which layouts exist. Clearing
    // known modes absent from the snapshot makes every successful write
    // commit pending deletions, so a reset whose own write was superseded
    // still lands with the next write instead of resurrecting on relaunch.
    for mode in EDITOR_MODES {
        if !snapshot.layouts.contains_key(*mode) {
            store.clear_workspace_layout(project_key, mode)?;
        }
    }
    Ok(())
}

/// Round a device-pixel coordinate into the persisted integer form.
///
/// `f32` has no fallible integer conversion, so the saturating `as` cast is the
/// only conversion available. Every value here is a display or window extent in
/// device pixels, far inside `i64`, and a non-finite coordinate persists as 0
/// so a corrupt bound is simply discarded by `clamp_window_bounds`.
#[allow(clippy::cast_possible_truncation)] // no f32 -> i64 TryFrom exists; pixel extents cannot overflow i64.
const fn persisted_pixels(value: f32) -> i64 {
    let rounded = value.round();
    if rounded.is_finite() {
        rounded as i64
    } else {
        0
    }
}

/// Widen a persisted pixel extent back to the `f32` gpui measures windows in.
///
/// `i64` has no lossless float conversion, but persisted window geometry is a
/// device-pixel extent well inside `f32`'s exact integer range (2^24).
#[allow(clippy::cast_precision_loss)] // no i64 -> f32 conversion exists; pixel extents are exact below 2^24.
const fn pixels_from_persisted(value: i64) -> f32 {
    value as f32
}

fn persisted_window_bounds(window: &Window) -> PersistedWindowBounds {
    let window_bounds = window.window_bounds();
    let bounds = window_bounds.get_bounds();
    PersistedWindowBounds {
        x: persisted_pixels(f32::from(bounds.origin.x)),
        y: persisted_pixels(f32::from(bounds.origin.y)),
        width: persisted_pixels(f32::from(bounds.size.width)),
        height: persisted_pixels(f32::from(bounds.size.height)),
        maximized: matches!(window_bounds, WindowBounds::Maximized(_)),
    }
}

fn clamp_window_bounds(persisted: PersistedWindowBounds, cx: &App) -> Option<WindowBounds> {
    if persisted.width < 320 || persisted.height < 240 {
        return None;
    }
    let displays = cx.displays();
    let target = displays
        .iter()
        .max_by_key(|display| {
            let bounds = display.bounds();
            intersection_area(persisted, bounds)
        })
        .filter(|display| intersection_area(persisted, display.bounds()) > 0)
        .cloned()
        .or_else(|| cx.primary_display())?;
    let display = target.bounds();
    let display_x = f32::from(display.origin.x);
    let display_y = f32::from(display.origin.y);
    let display_width = f32::from(display.size.width).max(1.0);
    let display_height = f32::from(display.size.height).max(1.0);
    let width =
        pixels_from_persisted(persisted.width).clamp(320.0_f32.min(display_width), display_width);
    let height = pixels_from_persisted(persisted.height)
        .clamp(240.0_f32.min(display_height), display_height);
    let x = pixels_from_persisted(persisted.x).clamp(display_x, display_x + display_width - width);
    let y =
        pixels_from_persisted(persisted.y).clamp(display_y, display_y + display_height - height);
    let bounds = gpui::Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    };
    Some(if persisted.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    })
}

fn intersection_area(persisted: PersistedWindowBounds, display: gpui::Bounds<gpui::Pixels>) -> i64 {
    let left = persisted
        .x
        .max(persisted_pixels(f32::from(display.origin.x)));
    let top = persisted
        .y
        .max(persisted_pixels(f32::from(display.origin.y)));
    let right = (persisted.x + persisted.width).min(persisted_pixels(f32::from(
        display.origin.x + display.size.width,
    )));
    let bottom = (persisted.y + persisted.height).min(persisted_pixels(f32::from(
        display.origin.y + display.size.height,
    )));
    (right - left).max(0) * (bottom - top).max(0)
}

fn normalized_project_key(project_root: &Path) -> String {
    let path = std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    let normalized = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn valid_project_state(state: &ProjectUiStateSnapshot) -> bool {
    valid_mode(&state.last_mode) && matches!(state.asset_view_mode.as_str(), "grid" | "list")
}

fn valid_mode(mode: &str) -> bool {
    EDITOR_MODES.contains(&mode)
}

fn valid_layout_for_mode(mode: &str, layout: &WorkspaceUiLayoutSnapshot) -> bool {
    let profile = crate::workspace::layout::layout_profile_for_mode(mode);
    region_matches_profile(layout.left.as_ref(), profile.left)
        && region_matches_profile(layout.right.as_ref(), profile.right)
        && region_matches_profile(layout.bottom.as_ref(), profile.bottom)
        && layout.center_active_panel.as_deref().is_none_or(|active| {
            profile
                .center
                .iter()
                .any(|placement| placement.panel_name == active)
        })
}

fn region_matches_profile(
    region: Option<&crate::project_manager_preferences::PersistedDockRegion>,
    placements: &[crate::workspace::layout::PanelPlacement],
) -> bool {
    match (region, placements.is_empty()) {
        (None, true) => true,
        (Some(region), false) => region.active_panel.as_deref().is_none_or(|active| {
            placements
                .iter()
                .any(|placement| placement.panel_name == active)
        }),
        _ => false,
    }
}

fn lock_cache(cache: &Mutex<UiStateCache>) -> std::sync::MutexGuard<'_, UiStateCache> {
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn lock_write(write_lock: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    write_lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_manager_preferences::{PersistedDockRegion, WORKSPACE_LAYOUT_VERSION};

    #[test]
    fn project_key_is_case_and_separator_stable_on_windows() {
        let drive = "E:";
        let path = format!(r"{drive}\Projects\sample-project");
        let key = normalized_project_key(Path::new(&path));
        assert!(!key.contains('\\'));
        if cfg!(windows) {
            assert_eq!(key, key.to_lowercase());
        }
    }

    fn test_store() -> (tempfile::TempDir, ProjectManagerPreferenceStore) {
        let temp = tempfile::tempdir().expect("create temp directory");
        let store = ProjectManagerPreferenceStore::open_local(&temp.path().join("prefs.db"))
            .expect("open preference store");
        (temp, store)
    }

    fn sample_layout() -> WorkspaceUiLayoutSnapshot {
        WorkspaceUiLayoutSnapshot {
            version: WORKSPACE_LAYOUT_VERSION,
            center_active_panel: Some("material_workbench".to_owned()),
            left: Some(PersistedDockRegion {
                size: 312.0,
                open: true,
                active_panel: Some("material_palette".to_owned()),
            }),
            right: Some(PersistedDockRegion {
                size: 288.0,
                open: false,
                active_panel: Some("material_params".to_owned()),
            }),
            bottom: None,
            panel_states: BTreeMap::new(),
        }
    }

    fn test_project_key(label: &str) -> String {
        std::env::temp_dir()
            .join(label)
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn only_gamedata_mode_owns_generic_source_history() {
        assert!(mode_owns_gamedata_history("gamedata"));
        assert!(!mode_owns_gamedata_history("scene"));
        assert!(!mode_owns_gamedata_history("materials"));
    }

    #[test]
    fn persist_cache_clears_known_modes_missing_from_the_cache() {
        let (_temp, store) = test_store();
        let project_key = test_project_key("reset-reconcile");
        store
            .save_workspace_layout(&project_key, "scene", &sample_layout())
            .unwrap();
        let cache = Mutex::new(UiStateCache {
            project: ProjectUiStateSnapshot::default(),
            layouts: BTreeMap::new(),
        });
        let write_lock = Mutex::new(());

        persist_cache(&store, &project_key, &cache, &write_lock, None).unwrap();

        assert!(
            store
                .load_workspace_layout(&project_key, "scene")
                .unwrap()
                .is_none(),
            "reset layout must not survive a reconciling write"
        );
    }

    #[test]
    fn superseded_write_skips_but_next_write_reconciles() {
        let (_temp, store) = test_store();
        let project_key = test_project_key("reset-supersede");
        store
            .save_workspace_layout(&project_key, "scene", &sample_layout())
            .unwrap();
        let cache = Mutex::new(UiStateCache {
            project: ProjectUiStateSnapshot::default(),
            layouts: BTreeMap::new(),
        });
        let write_lock = Mutex::new(());
        let revision = Arc::new(AtomicU64::new(1));

        // A write whose revision was superseded still performs no writes —
        // the guard for writes is untouched.
        persist_cache(
            &store,
            &project_key,
            &cache,
            &write_lock,
            Some((&revision, 0)),
        )
        .unwrap();
        assert!(
            store
                .load_workspace_layout(&project_key, "scene")
                .unwrap()
                .is_some(),
            "superseded write must be a no-op"
        );

        // The winning write commits the pending deletion.
        persist_cache(&store, &project_key, &cache, &write_lock, None).unwrap();
        assert!(
            store
                .load_workspace_layout(&project_key, "scene")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn persist_cache_keeps_cached_layouts_across_reconciliation() {
        let (_temp, store) = test_store();
        let project_key = test_project_key("reset-keep");
        let layout = sample_layout();
        store
            .save_workspace_layout(&project_key, "scene", &layout)
            .unwrap();
        let mut layouts = BTreeMap::new();
        layouts.insert("materials".to_owned(), layout);
        let cache = Mutex::new(UiStateCache {
            project: ProjectUiStateSnapshot::default(),
            layouts,
        });
        let write_lock = Mutex::new(());

        persist_cache(&store, &project_key, &cache, &write_lock, None).unwrap();

        assert!(
            store
                .load_workspace_layout(&project_key, "materials")
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .load_workspace_layout(&project_key, "scene")
                .unwrap()
                .is_none()
        );
    }
}
