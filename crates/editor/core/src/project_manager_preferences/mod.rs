//! Drizzle-backed persistence for editor-local preferences and UI state.

// `SQLiteTable` hard-codes `#[derive(Debug, Clone, PartialEq, Default)]` on the
// select models it generates and forwards no attributes from the input item, so
// the lint level cannot be set any nearer than this module declaration.
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod schema;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use az_filesystem::AzothDataHome;
use drizzle::core::expr::{eq, excluded};
use drizzle::sqlite::pragma::{Pragma, Synchronous, TempStore};
use drizzle::sqlite::prelude::*;
use drizzle::sqlite::turso::Drizzle;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use self::schema::{
    CURRENT_SCHEMA_VERSION, InsertDbInfo, InsertProjectManagerPreferences, InsertProjectUiStates,
    InsertWorkspaceUiLayouts, ProjectManagerPreferenceSchema, ProjectManagerPreferences,
    ProjectUiStates, UpdateDbInfo, UpdateProjectManagerPreferences, UpdateProjectUiStates,
    UpdateWorkspaceUiLayouts, WorkspaceUiLayouts,
};

pub const UI_STATE_VERSION: i64 = 1;
pub const WORKSPACE_LAYOUT_VERSION: i64 = 1;

trait PreferenceFutureExt: Future + Sized {
    fn wait(self) -> Self::Output {
        futures::executor::block_on(self)
    }
}

impl<F> PreferenceFutureExt for F where F: Future + Sized {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectManagerPreferenceSnapshot {
    pub recent_layout: String,
    pub recent_sort: String,
    pub pinned_project_ids: BTreeSet<String>,
}

impl Default for ProjectManagerPreferenceSnapshot {
    fn default() -> Self {
        Self {
            recent_layout: "list".to_string(),
            recent_sort: "last_opened".to_string(),
            pinned_project_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistedWindowBounds {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectUiStateSnapshot {
    pub last_mode: String,
    pub asset_view_mode: String,
    pub asset_folder_key: Option<String>,
    pub active_level_document_id: Option<String>,
    pub window: Option<PersistedWindowBounds>,
}

impl Default for ProjectUiStateSnapshot {
    fn default() -> Self {
        Self {
            last_mode: "scene".to_owned(),
            asset_view_mode: "grid".to_owned(),
            asset_folder_key: None,
            active_level_document_id: None,
            window: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedDockRegion {
    pub size: f32,
    pub open: bool,
    pub active_panel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceUiLayoutSnapshot {
    pub version: i64,
    pub center_active_panel: Option<String>,
    pub left: Option<PersistedDockRegion>,
    pub right: Option<PersistedDockRegion>,
    pub bottom: Option<PersistedDockRegion>,
    #[serde(default)]
    pub panel_states: BTreeMap<String, serde_json::Value>,
}

impl WorkspaceUiLayoutSnapshot {
    pub(crate) fn validate(&self) -> ProjectManagerPreferenceResult<()> {
        if self.version != WORKSPACE_LAYOUT_VERSION {
            return Err(ProjectManagerPreferenceError::IncompatibleUiState {
                stored: self.version,
                current: WORKSPACE_LAYOUT_VERSION,
            });
        }
        for region in [&self.left, &self.right, &self.bottom]
            .into_iter()
            .flatten()
        {
            if !region.size.is_finite() || region.size < 1.0 {
                return Err(ProjectManagerPreferenceError::InvalidUiState(
                    "dock size is not finite and positive".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ProjectManagerPreferenceError {
    #[error("machine-local Azoth data-home error: {0}")]
    DataHome(#[from] az_filesystem::DataHomeError),

    #[error("open editor preference database at {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: az_turso::OpenError,
    },
    #[error("editor preference database path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("create editor preference directory at {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("editor preference storage error: {0}")]
    Storage(#[source] drizzle::error::DrizzleError),
    #[error("editor preference schema version {stored} is newer than supported version {current}")]
    UnsupportedSchemaVersion { stored: i64, current: i64 },
    #[error("serialize pinned project preferences: {0}")]
    SerializePinnedProjects(#[source] serde_json::Error),
    #[error("deserialize pinned project preferences: {0}")]
    DeserializePinnedProjects(#[source] serde_json::Error),
    #[error("serialize workspace UI layout: {0}")]
    SerializeUiLayout(#[source] serde_json::Error),
    #[error("deserialize workspace UI layout: {0}")]
    DeserializeUiLayout(#[source] serde_json::Error),
    #[error("incompatible UI state version {stored}; expected {current}")]
    IncompatibleUiState { stored: i64, current: i64 },
    #[error("invalid UI state: {0}")]
    InvalidUiState(String),
}

impl From<drizzle::error::DrizzleError> for ProjectManagerPreferenceError {
    fn from(error: drizzle::error::DrizzleError) -> Self {
        Self::Storage(error)
    }
}

pub type ProjectManagerPreferenceResult<T> = Result<T, ProjectManagerPreferenceError>;

#[derive(Clone)]
pub struct ProjectManagerPreferenceStore {
    local_database: az_turso::LocalDatabase,
    drizzle: Drizzle<ProjectManagerPreferenceSchema>,
    tables: ProjectManagerPreferenceSchema,
}

impl std::fmt::Debug for ProjectManagerPreferenceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectManagerPreferenceStore")
            .field("path", &Self::path())
            .finish_non_exhaustive()
    }
}

impl ProjectManagerPreferenceStore {
    pub(crate) fn open_global() -> ProjectManagerPreferenceResult<Self> {
        AzothDataHome::resolve().prepare()?;
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| {
                ProjectManagerPreferenceError::CreateDirectory {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }
        Self::open_local(&path)
    }

    #[must_use]
    pub(crate) fn path() -> PathBuf {
        AzothDataHome::resolve().project_manager_preferences_path()
    }

    pub(crate) fn open_local(path: &Path) -> ProjectManagerPreferenceResult<Self> {
        let path_text = path
            .to_str()
            .ok_or_else(|| ProjectManagerPreferenceError::NonUtf8Path(path.to_path_buf()))?;
        let local_database = az_turso::open_local(path_text, Duration::from_secs(5))
            .wait()
            .map_err(|source| ProjectManagerPreferenceError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        let (drizzle, tables) = Drizzle::new(
            local_database.connection().clone(),
            ProjectManagerPreferenceSchema::new(),
        );
        let mut store = Self {
            local_database,
            drizzle,
            tables,
        };
        store.apply_pragmas()?;
        store.migrate_to_current_schema()?;
        let runtime_connection = store
            .local_database
            .new_connection(Duration::from_secs(5))
            .map_err(|source| ProjectManagerPreferenceError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        let (runtime_drizzle, runtime_tables) =
            Drizzle::new(runtime_connection, ProjectManagerPreferenceSchema::new());
        store.drizzle = runtime_drizzle;
        store.tables = runtime_tables;
        store.apply_pragmas()?;
        Ok(store)
    }

    fn apply_pragmas(&self) -> ProjectManagerPreferenceResult<()> {
        self.drizzle.execute(Pragma::foreign_keys(true)).wait()?;
        self.drizzle
            .execute(Pragma::Synchronous(Synchronous::Normal))
            .wait()?;
        self.drizzle.execute(Pragma::CacheSize(-16_000)).wait()?;
        self.drizzle
            .execute(Pragma::TempStore(TempStore::Memory))
            .wait()?;
        Ok(())
    }

    fn migrate_to_current_schema(&mut self) -> ProjectManagerPreferenceResult<()> {
        let migrations = drizzle::include_migrations!("./drizzle");
        self.drizzle
            .migrate(&migrations, drizzle::migrations::Tracking::SQLITE)
            .wait()?;

        let db_info = self.tables.db_info;
        let stored_version: Option<i64> = match self
            .drizzle
            .select((db_info.version,))
            .from(db_info)
            .r#where(eq(db_info.row_id, 1))
            .limit(1)
            .get()
            .wait()
        {
            Ok((version,)) => Some(version),
            Err(drizzle::error::DrizzleError::NotFound) => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(stored) = stored_version
            && stored > CURRENT_SCHEMA_VERSION
        {
            return Err(ProjectManagerPreferenceError::UnsupportedSchemaVersion {
                stored,
                current: CURRENT_SCHEMA_VERSION,
            });
        }
        if stored_version == Some(CURRENT_SCHEMA_VERSION) {
            return Ok(());
        }
        self.drizzle
            .insert(db_info)
            .values([InsertDbInfo::new(CURRENT_SCHEMA_VERSION).with_row_id(1)])
            .on_conflict(db_info.row_id)
            .do_update(UpdateDbInfo::default().with_version(excluded(db_info.version)))
            .execute()
            .wait()?;
        Ok(())
    }

    pub(crate) fn load(&self) -> ProjectManagerPreferenceResult<ProjectManagerPreferenceSnapshot> {
        let prefs = self.tables.project_manager_preferences;
        let rows: Vec<ProjectManagerPreferencesRecord> = self
            .drizzle
            .select(ProjectManagerPreferencesRecord::default())
            .from(prefs)
            .r#where(eq(prefs.row_id, 1))
            .limit(1)
            .all()
            .wait()?;

        let Some(row) = rows.into_iter().next() else {
            return Ok(ProjectManagerPreferenceSnapshot::default());
        };
        let pinned_project_ids = serde_json::from_str(&row.pinned_project_ids_json)
            .map_err(ProjectManagerPreferenceError::DeserializePinnedProjects)?;
        Ok(ProjectManagerPreferenceSnapshot {
            recent_layout: row.recent_layout,
            recent_sort: row.recent_sort,
            pinned_project_ids,
        })
    }

    pub(crate) fn save(
        &self,
        snapshot: &ProjectManagerPreferenceSnapshot,
    ) -> ProjectManagerPreferenceResult<()> {
        let prefs = self.tables.project_manager_preferences;
        let pinned_json = serde_json::to_string(&snapshot.pinned_project_ids)
            .map_err(ProjectManagerPreferenceError::SerializePinnedProjects)?;
        self.drizzle
            .insert(prefs)
            .values([InsertProjectManagerPreferences::new(
                &snapshot.recent_layout,
                &snapshot.recent_sort,
                &pinned_json,
                current_unix_ms(),
            )
            .with_row_id(1)])
            .on_conflict(prefs.row_id)
            .do_update(
                UpdateProjectManagerPreferences::default()
                    .with_recent_layout(excluded(prefs.recent_layout))
                    .with_recent_sort(excluded(prefs.recent_sort))
                    .with_pinned_project_ids_json(excluded(prefs.pinned_project_ids_json))
                    .with_updated_unix_ms(excluded(prefs.updated_unix_ms)),
            )
            .execute()
            .wait()?;
        Ok(())
    }

    pub(crate) fn load_project_ui_state(
        &self,
        project_key: &str,
    ) -> ProjectManagerPreferenceResult<Option<ProjectUiStateSnapshot>> {
        let table = self.tables.project_ui_states;
        let rows: Vec<ProjectUiStateRecord> = self
            .drizzle
            .select(ProjectUiStateRecord::default())
            .from(table)
            .r#where(eq(table.project_key, project_key))
            .limit(1)
            .all()
            .wait()?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        if row.state_version != UI_STATE_VERSION {
            return Err(ProjectManagerPreferenceError::IncompatibleUiState {
                stored: row.state_version,
                current: UI_STATE_VERSION,
            });
        }
        let window = match (
            row.window_x,
            row.window_y,
            row.window_width,
            row.window_height,
        ) {
            (Some(x), Some(y), Some(width), Some(height)) if width >= 320 && height >= 240 => {
                Some(PersistedWindowBounds {
                    x,
                    y,
                    width,
                    height,
                    maximized: row.window_maximized,
                })
            }
            (Some(_), Some(_), Some(0), Some(0)) | (None, None, None, None) => None,
            _ => {
                return Err(ProjectManagerPreferenceError::InvalidUiState(
                    "window bounds are incomplete".to_owned(),
                ));
            }
        };
        Ok(Some(ProjectUiStateSnapshot {
            last_mode: row.last_mode,
            asset_view_mode: row.asset_view_mode,
            asset_folder_key: row.asset_folder_key.filter(|key| !key.is_empty()),
            active_level_document_id: row
                .active_level_document_id
                .filter(|document_id| !document_id.is_empty()),
            window,
        }))
    }

    pub(crate) fn save_project_ui_state(
        &self,
        project_key: &str,
        snapshot: &ProjectUiStateSnapshot,
    ) -> ProjectManagerPreferenceResult<()> {
        let table = self.tables.project_ui_states;
        let (x, y, width, height, maximized) =
            snapshot
                .window
                .map_or((None, None, None, None, false), |bounds| {
                    (
                        Some(bounds.x),
                        Some(bounds.y),
                        Some(bounds.width),
                        Some(bounds.height),
                        bounds.maximized,
                    )
                });
        let row = InsertProjectUiStates::new(
            project_key,
            UI_STATE_VERSION,
            &snapshot.last_mode,
            &snapshot.asset_view_mode,
            maximized,
            current_unix_ms(),
        )
        .with_asset_folder_key(snapshot.asset_folder_key.as_deref().unwrap_or(""))
        .with_active_level_document_id(snapshot.active_level_document_id.as_deref().unwrap_or(""))
        .with_window_x(x.unwrap_or_default())
        .with_window_y(y.unwrap_or_default())
        .with_window_width(width.unwrap_or_default())
        .with_window_height(height.unwrap_or_default());
        self.drizzle
            .insert(table)
            .values([row])
            .on_conflict(table.project_key)
            .do_update(
                UpdateProjectUiStates::default()
                    .with_state_version(excluded(table.state_version))
                    .with_last_mode(excluded(table.last_mode))
                    .with_asset_view_mode(excluded(table.asset_view_mode))
                    .with_asset_folder_key(excluded(table.asset_folder_key))
                    .with_active_level_document_id(excluded(table.active_level_document_id))
                    .with_window_x(excluded(table.window_x))
                    .with_window_y(excluded(table.window_y))
                    .with_window_width(excluded(table.window_width))
                    .with_window_height(excluded(table.window_height))
                    .with_window_maximized(excluded(table.window_maximized))
                    .with_updated_unix_ms(excluded(table.updated_unix_ms)),
            )
            .execute()
            .wait()?;
        Ok(())
    }

    pub(crate) fn clear_project_ui_state(
        &self,
        project_key: &str,
    ) -> ProjectManagerPreferenceResult<()> {
        let table = self.tables.project_ui_states;
        self.drizzle
            .delete(table)
            .r#where(eq(table.project_key, project_key))
            .execute()
            .wait()?;
        Ok(())
    }

    pub(crate) fn load_workspace_layout(
        &self,
        project_key: &str,
        mode: &str,
    ) -> ProjectManagerPreferenceResult<Option<WorkspaceUiLayoutSnapshot>> {
        let table = self.tables.workspace_ui_layouts;
        let layout_key = workspace_layout_key(project_key, mode);
        let rows: Vec<WorkspaceUiLayoutRecord> = self
            .drizzle
            .select(WorkspaceUiLayoutRecord::default())
            .from(table)
            .r#where(eq(table.layout_key, layout_key.as_str()))
            .limit(1)
            .all()
            .wait()?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        if row.state_version != WORKSPACE_LAYOUT_VERSION {
            return Err(ProjectManagerPreferenceError::IncompatibleUiState {
                stored: row.state_version,
                current: WORKSPACE_LAYOUT_VERSION,
            });
        }
        let snapshot = serde_json::from_str::<WorkspaceUiLayoutSnapshot>(&row.layout_json)
            .map_err(ProjectManagerPreferenceError::DeserializeUiLayout)?;
        snapshot.validate()?;
        Ok(Some(snapshot))
    }

    pub(crate) fn save_workspace_layout(
        &self,
        project_key: &str,
        mode: &str,
        snapshot: &WorkspaceUiLayoutSnapshot,
    ) -> ProjectManagerPreferenceResult<()> {
        snapshot.validate()?;
        let table = self.tables.workspace_ui_layouts;
        let layout_key = workspace_layout_key(project_key, mode);
        let layout_json = serde_json::to_string(snapshot)
            .map_err(ProjectManagerPreferenceError::SerializeUiLayout)?;
        self.drizzle
            .insert(table)
            .values([InsertWorkspaceUiLayouts::new(
                &layout_key,
                project_key,
                mode,
                WORKSPACE_LAYOUT_VERSION,
                &layout_json,
                current_unix_ms(),
            )])
            .on_conflict(table.layout_key)
            .do_update(
                UpdateWorkspaceUiLayouts::default()
                    .with_project_key(excluded(table.project_key))
                    .with_mode(excluded(table.mode))
                    .with_state_version(excluded(table.state_version))
                    .with_layout_json(excluded(table.layout_json))
                    .with_updated_unix_ms(excluded(table.updated_unix_ms)),
            )
            .execute()
            .wait()?;
        Ok(())
    }

    pub(crate) fn clear_workspace_layout(
        &self,
        project_key: &str,
        mode: &str,
    ) -> ProjectManagerPreferenceResult<()> {
        let table = self.tables.workspace_ui_layouts;
        let layout_key = workspace_layout_key(project_key, mode);
        self.drizzle
            .delete(table)
            .r#where(eq(table.layout_key, layout_key.as_str()))
            .execute()
            .wait()?;
        Ok(())
    }
}

#[derive(Debug, Default, SQLiteFromRow)]
#[from(ProjectManagerPreferences)]
struct ProjectManagerPreferencesRecord {
    recent_layout: String,
    recent_sort: String,
    pinned_project_ids_json: String,
}

#[derive(Debug, Default, SQLiteFromRow)]
#[from(ProjectUiStates)]
struct ProjectUiStateRecord {
    state_version: i64,
    last_mode: String,
    asset_view_mode: String,
    asset_folder_key: Option<String>,
    active_level_document_id: Option<String>,
    window_x: Option<i64>,
    window_y: Option<i64>,
    window_width: Option<i64>,
    window_height: Option<i64>,
    window_maximized: bool,
}

#[derive(Debug, Default, SQLiteFromRow)]
#[from(WorkspaceUiLayouts)]
struct WorkspaceUiLayoutRecord {
    state_version: i64,
    layout_json: String,
}

fn workspace_layout_key(project_key: &str, mode: &str) -> String {
    format!("{project_key}\n{mode}")
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        // Saturating at `i64::MAX` is unreachable until the year 292'277'026'596.
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_round_trips_through_drizzle_store() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let store = ProjectManagerPreferenceStore::open_local(&temp.path().join("prefs.db"))
            .expect("open preference store");
        let mut journal_rows = store
            .local_database
            .connection()
            .query("PRAGMA journal_mode", ())
            .wait()
            .expect("query journal mode");
        let journal_mode = journal_rows
            .next()
            .wait()
            .expect("read journal mode")
            .expect("journal mode row")
            .get::<String>(0)
            .expect("journal mode string");
        assert_eq!(journal_mode, az_turso::WAL_JOURNAL_MODE);
        drop(journal_rows);
        let project_key = std::env::temp_dir()
            .join("azoth-e2e-proj")
            .to_string_lossy()
            .into_owned();
        let project = ProjectUiStateSnapshot {
            last_mode: "materials".to_owned(),
            asset_view_mode: "list".to_owned(),
            asset_folder_key: Some("project-assets/materials".to_owned()),
            active_level_document_id: Some("scenes/canyon.scene.ron".to_owned()),
            window: Some(PersistedWindowBounds {
                x: 120,
                y: 80,
                width: 1440,
                height: 900,
                maximized: true,
            }),
        };
        let layout = WorkspaceUiLayoutSnapshot {
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
        };

        store
            .save_project_ui_state(&project_key, &project)
            .expect("save project UI state");
        store
            .save_workspace_layout(&project_key, "materials", &layout)
            .expect("save workspace layout");

        assert_eq!(
            store
                .load_project_ui_state(&project_key)
                .expect("load project UI state"),
            Some(project)
        );
        assert_eq!(
            store
                .load_workspace_layout(&project_key, "materials")
                .expect("load workspace layout"),
            Some(layout)
        );

        store
            .clear_workspace_layout(&project_key, "materials")
            .expect("clear workspace layout");
        let cleared = store
            .load_workspace_layout(&project_key, "materials")
            .expect("load cleared workspace layout");
        drop(store);
        assert_eq!(cleared, None);
    }
}
