//! `GameData` mode projection data.
//!
//! Editor-core owns the `GameData` catalog boundary (project-host RPC snapshot,
//! schema catalog joins, loaded-document grid rows) and publishes
//! [`EditorGameDataProjection`] for the `GameData` mode panels to render. The
//! panels deliberately know nothing about `GameDataCatalogSnapshot` or
//! project-host; they render this projection and dispatch typed
//! `GameData*` actions from [`crate::actions`].

use gpui::Global;

/// Which left-rail list is active in the `GameData` catalog panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameDataRailView {
    #[default]
    Tables,
    Schemas,
    Managers,
}

/// Which right-rail inspector tab is active in the `GameData` details panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataInspectorTab {
    Field,
    Table,
    Schema,
    Manager,
}

/// Semantic tint for a projected `GameData` row/stage/chip. Panels map tones to
/// theme tokens; the projection never carries concrete colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataTone {
    Neutral,
    Accent,
    Info,
    Success,
    Warning,
    Danger,
}

/// Classified authored-field/value kind used for grid columns, cells, and
/// schema field rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataFieldKind {
    Key,
    Text,
    Number,
    Boolean,
    Reference,
    Asset,
    Color,
    Object,
    Unset,
}

impl GameDataFieldKind {
    /// Semantic tone for this field kind.
    #[must_use]
    pub const fn tone(self) -> GameDataTone {
        match self {
            Self::Key => GameDataTone::Warning,
            Self::Text | Self::Unset => GameDataTone::Neutral,
            Self::Number | Self::Asset => GameDataTone::Info,
            Self::Boolean => GameDataTone::Success,
            Self::Reference | Self::Color | Self::Object => GameDataTone::Accent,
        }
    }
}

/// Root `GameData` mode projection global published by editor-core.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorGameDataProjection {
    /// Whether a `GameData` catalog snapshot has been published for the session.
    pub catalog_loaded: bool,
    pub rail_view: GameDataRailView,
    pub table_folders: Vec<GameDataTableFolderProjection>,
    pub table_count: usize,
    pub schemas: Vec<GameDataSchemaRowProjection>,
    pub schema_count: usize,
    pub manager_groups: Vec<GameDataManagerGroupProjection>,
    pub manager_count: usize,
    pub workbench: GameDataWorkbenchProjection,
    pub inspector: GameDataInspectorProjection,
    /// Catalog-level diagnostics published by project-host.
    pub diagnostics: Vec<GameDataDiagnosticProjection>,
}

impl Global for EditorGameDataProjection {}

/// One UI category folder in the tables rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataTableFolderProjection {
    pub name: String,
    pub tables: Vec<GameDataTableRowProjection>,
}

/// One table row in the tables rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataTableRowProjection {
    pub key: String,
    pub name: String,
    pub document_id: String,
    /// Trailing meta cell: catalog row count, or load state when unknown.
    pub count_label: String,
    pub selected: bool,
    pub loaded: bool,
}

/// One schema row in the schemas rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataSchemaRowProjection {
    pub key: String,
    pub label: String,
    pub used_by_label: String,
    pub selected: bool,
    /// Whether the schema key resolved against the published schema catalog.
    pub resolved: bool,
}

/// Grouping kind for the managers rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataManagerGroupKind {
    /// Derived read-only table/family providers.
    AutomaticProviders,
    /// Hand-authored manager descriptors grouped by owner gem.
    OwnerGem,
}

/// One owner group in the managers rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerGroupProjection {
    pub key: String,
    pub name: String,
    pub kind: GameDataManagerGroupKind,
    pub managers: Vec<GameDataManagerRowProjection>,
}

/// One manager row in the managers rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerRowProjection {
    pub key: String,
    pub name: String,
    pub kind_label: String,
    pub read_only: bool,
    pub has_diagnostics: bool,
    pub selected: bool,
}

/// Center workbench content, switched by the current selection kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameDataWorkbenchProjection {
    Empty { message: String },
    Grid(GameDataGridProjection),
    Schema(GameDataSchemaEditorProjection),
    Manager(GameDataManagerCompositionProjection),
}

impl Default for GameDataWorkbenchProjection {
    fn default() -> Self {
        Self::Empty {
            message: "GameData catalog not loaded".to_owned(),
        }
    }
}

/// Where the grid rows came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataGridRowState {
    /// Rows projected from the loaded authored document.
    Loaded,
    /// The table document has not been loaded into the editor yet; columns
    /// come from the schema catalog and rows are unavailable.
    Loading,
    /// The selected source could not be opened or edited. Rows are cleared so
    /// a prior table can never be shown under the current selection.
    Error,
}

/// Data-grid projection for the selected table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataGridProjection {
    pub table_key: String,
    pub table_name: String,
    pub document_id: String,
    pub source_path: String,
    pub owner: String,
    pub category: String,
    pub families: Vec<String>,
    pub schema_key: String,
    pub schema_label: String,
    pub primary_key_label: String,
    pub row_state: GameDataGridRowState,
    /// Source-authoring controls are only enabled for a canonical ready snapshot.
    pub can_edit: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Current source-authoring failure, if loading the selected table failed.
    pub status_detail: Option<String>,
    pub columns: Vec<GameDataGridColumnProjection>,
    pub rows: Vec<GameDataGridRowProjection>,
    /// Row count before the filter was applied.
    pub total_row_count: usize,
    pub filter: String,
    /// Selected field detail for the right-rail field inspector.
    pub selected_field: Option<GameDataFieldDetailProjection>,
    /// Catalog diagnostics targeting this table.
    pub diagnostics: Vec<GameDataDiagnosticProjection>,
}

/// One grid header column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataGridColumnProjection {
    pub key: String,
    pub label: String,
    pub type_label: String,
    pub kind: GameDataFieldKind,
    pub width: u32,
    pub primary: bool,
    pub selected: bool,
}

/// One grid body row (one authored row object).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataGridRowProjection {
    pub object_id: String,
    pub index_label: String,
    pub cells: Vec<GameDataGridCellProjection>,
}

/// One grid cell value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataGridCellProjection {
    pub column_key: String,
    pub text: String,
    pub kind: GameDataFieldKind,
    pub bool_value: Option<bool>,
}

/// Selected-field detail for the field inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataFieldDetailProjection {
    pub key: String,
    pub label: String,
    pub type_label: String,
    pub kind: GameDataFieldKind,
    pub schema_label: String,
    /// Number of tables sharing the owning schema.
    pub shared_table_count: usize,
    pub value_preview: Option<String>,
    pub description: Option<String>,
    pub read_only: bool,
}

/// Schema editor projection for the selected schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataSchemaEditorProjection {
    pub schema_key: String,
    pub label: String,
    pub rust_type: Option<String>,
    pub description: Option<String>,
    /// Whether the schema key resolved against the published schema catalog.
    pub resolved: bool,
    pub fields: Vec<GameDataSchemaFieldProjection>,
    pub used_by: Vec<GameDataSchemaUsageProjection>,
}

/// One schema field row in the schema editor / schema inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataSchemaFieldProjection {
    pub key: String,
    pub label: String,
    pub type_label: String,
    pub kind: GameDataFieldKind,
    pub read_only: bool,
}

/// One "table using this schema" row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataSchemaUsageProjection {
    pub table_key: String,
    pub name: String,
    pub category: String,
    pub count_label: String,
}

/// Manager composition projection for the selected manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerCompositionProjection {
    pub key: String,
    pub name: String,
    pub owner: String,
    pub kind_label: String,
    pub catalog_kind: String,
    pub row_type: String,
    pub output_type: String,
    pub read_only: bool,
    /// Factual summary line derived from descriptor truth.
    pub summary: String,
    pub status_ok: bool,
    pub status_label: String,
    /// Resolved backing table for auto providers (jump target).
    pub backing_table_key: Option<String>,
    pub backing_table_name: Option<String>,
    pub stages: Vec<GameDataManagerStageProjection>,
    pub sources: Vec<GameDataManagerSourceProjection>,
    pub projection_rows: Vec<GameDataManagerProjectionRowProjection>,
    pub projection_count: usize,
    pub consumers: Vec<GameDataManagerNodeChipProjection>,
    pub dependencies: Vec<GameDataManagerNodeChipProjection>,
    pub key_policy_summary: String,
    pub key_kind: String,
    pub transform_chips: Vec<String>,
    pub duplicate_key_policy: String,
    pub row_filters: Vec<GameDataManagerFilterRowProjection>,
    pub validation: Vec<GameDataManagerValidationRowProjection>,
    /// Why descriptor source text is not rendered (project-host publishes
    /// resolved descriptors, not registration source).
    pub descriptor_note: String,
}

/// One access-pipeline stage box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerStageProjection {
    pub key: String,
    pub title: String,
    pub detail: String,
    pub tone: GameDataTone,
}

/// One manager source card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerSourceProjection {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub detail: String,
    /// Catalog table this source resolves to, when it is table-backed.
    pub table_key: Option<String>,
    /// Manager dependency this source resolves to, when manager-backed.
    pub manager_key: Option<String>,
}

/// One projection row (output field <- source column/transform).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerProjectionRowProjection {
    pub output_field: String,
    pub source_label: String,
    /// True when the row is a computed transform rather than an identity
    /// column projection.
    pub computed: bool,
}

/// One dependency/consumer chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerNodeChipProjection {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub provider: bool,
}

/// One declared row filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerFilterRowProjection {
    pub field: String,
    pub predicate: String,
    pub compare_field: String,
}

/// One validation/diagnostic row for the manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerValidationRowProjection {
    pub message: String,
    pub target_label: String,
    pub ok: bool,
}

/// One catalog diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataDiagnosticProjection {
    pub code: String,
    pub message: String,
    pub target_label: String,
}

/// Right-rail inspector tab state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataInspectorProjection {
    pub tab: GameDataInspectorTab,
    pub tabs: Vec<GameDataInspectorTab>,
}

impl Default for GameDataInspectorProjection {
    fn default() -> Self {
        Self {
            tab: GameDataInspectorTab::Table,
            tabs: Vec::new(),
        }
    }
}
