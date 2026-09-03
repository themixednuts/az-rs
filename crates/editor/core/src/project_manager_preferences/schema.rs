//! Drizzle schema for editor-local Project Manager preferences.

use drizzle::sqlite::prelude::*;

/// Database metadata. Always exactly one row at `row_id = 1`.
#[SQLiteTable]
pub struct DbInfo {
    #[column(primary)]
    pub row_id: i64,
    pub version: i64,
}

/// User preference state for the standalone Project Manager.
#[SQLiteTable]
pub struct ProjectManagerPreferences {
    #[column(primary)]
    pub row_id: i64,
    pub recent_layout: String,
    pub recent_sort: String,
    pub pinned_project_ids_json: String,
    pub updated_unix_ms: i64,
}

/// Project-scoped editor chrome and navigation state.
#[SQLiteTable]
pub struct ProjectUiStates {
    #[column(primary)]
    pub project_key: String,
    pub state_version: i64,
    pub last_mode: String,
    pub asset_view_mode: String,
    pub asset_folder_key: Option<String>,
    pub active_level_document_id: Option<String>,
    pub window_x: Option<i64>,
    pub window_y: Option<i64>,
    pub window_width: Option<i64>,
    pub window_height: Option<i64>,
    pub window_maximized: bool,
    pub updated_unix_ms: i64,
}

/// Versioned dock layout for one project and editor mode.
#[SQLiteTable]
pub struct WorkspaceUiLayouts {
    #[column(primary)]
    pub layout_key: String,
    pub project_key: String,
    pub mode: String,
    pub state_version: i64,
    pub layout_json: String,
    pub updated_unix_ms: i64,
}

/// Top-level schema struct used by `Drizzle::new`.
// `SQLiteSchema` emits an explicit `Clone` impl alongside `Copy`; the impl is
// macro-generated and cannot be replaced with a derive from here.
#[allow(clippy::expl_impl_clone_on_copy)]
#[derive(SQLiteSchema)]
pub struct ProjectManagerPreferenceSchema {
    pub db_info: DbInfo,
    pub project_manager_preferences: ProjectManagerPreferences,
    pub project_ui_states: ProjectUiStates,
    pub workspace_ui_layouts: WorkspaceUiLayouts,
}

pub const CURRENT_SCHEMA_VERSION: i64 = 2;
