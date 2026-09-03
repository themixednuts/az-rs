// Runtime projection for the adopted Aether editor view.
//
// The adopted view is backed by typed editor state. Fixed chrome lives in this
// model as Rust projections; unimplemented editor modes render explicit empty
// toolbars instead of generated mock controls.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use az_editor_inspector::{
    ReflectedComponentInspection, ReflectedEditBinding, ReflectedEntityInspection,
    ReflectedInspectionChild, ReflectedInspectionField, ReflectedOverrideOperation,
    ReflectedScalar, ReflectedValue, ReflectedValueNode, WidgetFamily,
};
use az_editor_ui::panels::asset_creation::{
    AssetSourceCreateRequestData, CreatableAssetSourceData, build_asset_source_create_request,
    creatable_asset_sources, default_target_folder_for_source,
};
use az_editor_ui::panels::{
    AssetBrowserEntryData, AssetBrowserEntryStatus, AssetBrowserFolderData, AssetBrowserJobStatus,
    AssetSourceFileWorkflowData, AuthoredDocumentOutlineData, AuthoredLayerRow,
    AuthoredObjectOutlineData, ConsoleLevelCounts, ConsoleState, CreatableAuthoredSchemaData,
    ENGINE_PREFAB_ROOT_SCHEMA_TYPE, EditorActiveLevel, EditorAddableAuthoredComponents,
    EditorAnimationEventData, EditorAnimationJointData, EditorAnimationMotionData,
    EditorAnimationPreviewCatalog, EditorAssetBrowserStatus, EditorAssetBuilderCatalog,
    EditorAssetProcessorActivity, EditorAssetSourceDependentsPreview, EditorAuthoredOutline,
    EditorBlendSpaceAssetData, EditorBlendSpaceAssetKind, EditorBlendSpaceData,
    EditorBlendSpaceDimensionData, EditorBlendSpaceExampleData, EditorBlendSpacePreview,
    EditorBlendSpacePreviewCatalog, EditorCreatableAuthoredSchemas, EditorGpuStateData,
    EditorGpuStatus, EditorGraphDocumentProjection, EditorLayerVisibility,
    EditorMannequinAnimationRefData, EditorMannequinAuthoringCatalog,
    EditorMannequinFragmentBlendData, EditorMannequinFragmentData,
    EditorMannequinFragmentOptionData, EditorMannequinPreview,
    EditorMannequinResolvedAnimationData, EditorMannequinScopeData, EditorMannequinTagData,
    EditorReflectedSelectionState, EditorRuntimeStateData, EditorRuntimeStatus,
    EditorSessionStateData, EditorSessionStatus, EditorTypeRegistry, EditorViewportRenderStateData,
    EditorViewportRenderStatus, EditorViewportTelemetryData, GraphDocumentListItemProjectionData,
    GraphDocumentProjectionData, GraphNodeProjectionData, GraphPortDirectionData,
    GraphPortProjectionData, LogLevel, LogMessage, OutputLogMessage, OutputLogState,
    ProfilerPipelineStatus, SessionProcessStateData, WorkspaceRootData,
    active_level_prefab_documents, asset_browser_entry_matches_folder,
    asset_browser_folder_for_key, asset_browser_folders, authored_layer_rows, format_console_time,
    gpu_pipeline_status, is_scene_document, is_scene_document_schema, project_workflow,
    runtime_pipeline_status, validate_asset_db_relative_path, viewport_pipeline_status,
};
use az_editor_ui::{
    EditorBuildProfileData, EditorBuildTargetData, EditorGemCatalog, EditorGemInfo,
    EditorGemSelection, EditorProjectBuildCatalog, EditorProjectBuildPhase,
    EditorProjectBuildState, EditorScenePivot, EditorSceneToolKind, EditorSceneToolState,
    EditorSceneTransformSpace,
};
use az_proto_project::vnext::{
    PrefabEditCommand, ReflectedPathSegment, ReflectedValueEncoding, ReflectedValueEnvelope,
    TypeRegistrySnapshot,
};
use az_proto_project::{GameDataCatalogSnapshot, GameDataTableDescriptor};
use gpui::{AppContext, Context, Hsla, Pixels, Point, Rgba, Window};
use gpui_component::ActiveTheme;

use super::aether_common::{
    AetherItem, AetherItems, AetherStyle, asset_display_name, non_empty_string_or,
};
use super::aether_editor_view::AetherEditorView;

use crate::attach::EditorAttachSession;
use crate::game_data_catalog::EditorGameDataCatalog;
use crate::settings::{EditorSettings, SettingsStore};

const TOP_MENU_BAR_HEIGHT: f32 = 34.0;
const TOP_MENU_PANEL_WIDTH: f32 = 228.0;
const TOP_MENU_START_X: f32 = 33.0;
const TOP_MENU_BUTTON_HORIZONTAL_PADDING: f32 = 18.0;
const TOP_MENU_TEXT_CHAR_WIDTH: f32 = 7.0;
const MENU_AIM_TRIANGLE_PADDING: f32 = 24.0;
const MENU_AIM_MAX_DELAY_MS: u64 = 300;
pub(crate) fn trace_aether_ui_interaction(event: &str, detail: impl fmt::Display) {
    let detail = detail.to_string();
    tracing::info!(
        target: "az_editor::aether_ui",
        phase = "interaction",
        event = event,
        detail = %detail,
        "ui interaction"
    );
    if aether_ui_trace_to_stderr() {
        eprintln!("phase=interaction event={event} detail={detail}");
    }
}

pub(super) fn trace_aether_ui_state(
    event: &str,
    detail: impl fmt::Display,
    state: &AetherEditorState,
) {
    let detail = detail.to_string();
    let state = state.trace_summary();
    tracing::info!(
        target: "az_editor::aether_ui",
        phase = "state",
        event = event,
        detail = %detail,
        state = %state,
        "ui state"
    );
    if aether_ui_trace_to_stderr() {
        eprintln!("phase=state event={event} detail={detail} state={state}");
    }
}

pub(super) fn trace_aether_ui_render_state(
    event: &str,
    detail: impl fmt::Display,
    state: &AetherEditorState,
) {
    if !aether_ui_trace_to_stderr() {
        return;
    }

    let detail = detail.to_string();
    let state = state.trace_summary();
    let snapshot = format!("detail={detail} state={state}");
    static RENDER_SNAPSHOTS: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    let snapshots = RENDER_SNAPSHOTS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut snapshots = snapshots
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if snapshots
        .get(event)
        .is_some_and(|previous| previous == &snapshot)
    {
        return;
    }
    snapshots.insert(event.to_owned(), snapshot.clone());
    drop(snapshots);

    tracing::debug!(
        target: "az_editor::aether_ui",
        phase = "render",
        event = event,
        detail = %detail,
        state = %state,
        "ui render state"
    );
    eprintln!("phase=render event={event} {snapshot}");
}

fn aether_ui_trace_to_stderr() -> bool {
    std::env::var("AZOTH_EDITOR_TRACE_UI").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[derive(Copy, Clone, Debug)]
pub(super) struct MenuAimPoint {
    x: f32,
    y: f32,
}

impl MenuAimPoint {
    pub(super) fn from_gpui(position: Point<Pixels>) -> Self {
        Self {
            x: position.x.as_f32(),
            y: position.y.as_f32(),
        }
    }
}

pub(super) fn contains_point_in_triangle(
    point: MenuAimPoint,
    a: MenuAimPoint,
    b: MenuAimPoint,
    c: MenuAimPoint,
) -> bool {
    fn signed_area(p1: MenuAimPoint, p2: MenuAimPoint, p3: MenuAimPoint) -> f32 {
        (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
    }

    let d1 = signed_area(point, a, b);
    let d2 = signed_area(point, b, c);
    let d3 = signed_area(point, c, a);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

pub(super) fn top_menu_button_width(label: &str) -> f32 {
    TOP_MENU_BUTTON_HORIZONTAL_PADDING + label.chars().count() as f32 * TOP_MENU_TEXT_CHAR_WIDTH
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum AetherConsoleFilter {
    All,
    Info,
    Warn,
    Error,
}

impl AetherConsoleFilter {
    pub(super) const ALL: [Self; 4] = [Self::All, Self::Info, Self::Warn, Self::Error];

    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::All => "console-filter-all",
            Self::Info => "console-filter-info",
            Self::Warn => "console-filter-warn",
            Self::Error => "console-filter-error",
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Info => "Info",
            Self::Warn => "Warnings",
            Self::Error => "Errors",
        }
    }

    pub(super) const fn icon(self) -> &'static str {
        match self {
            Self::All => "list",
            Self::Info => "info",
            Self::Warn => "warning",
            Self::Error => "error",
        }
    }

    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "console-filter-all" => Some(Self::All),
            "console-filter-info" => Some(Self::Info),
            "console-filter-warn" => Some(Self::Warn),
            "console-filter-error" => Some(Self::Error),
            _ => None,
        }
    }

    pub(super) const fn shows(self, level: LogLevel) -> bool {
        match self {
            Self::All => true,
            Self::Info => matches!(level, LogLevel::Trace | LogLevel::Debug | LogLevel::Info),
            Self::Warn => matches!(level, LogLevel::Warn),
            Self::Error => matches!(level, LogLevel::Error),
        }
    }

    pub(super) const fn count_from(self, counts: ConsoleLevelCounts) -> usize {
        match self {
            Self::All => counts.total(),
            Self::Info => counts.info_like(),
            Self::Warn => counts.warn,
            Self::Error => counts.error,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OverlayDismissal {
    None,
    Closed,
    ApplySettings(EditorSettings),
    PersistSettings(EditorSettings),
}

#[derive(Debug, Clone)]
pub(crate) struct AetherEditorState {
    workspace: AetherWorkspaceState,
    overlay: AetherOverlayState,
    assets: AetherAssetBrowserState,
    authored_content: AetherAuthoredContentState,
    game_data: AetherGameDataState,
    gems: AetherGemSelectionState,
    diagnostics: AetherDiagnosticsState,
}

/// Local interaction state is intentionally a child module.  The editor view
/// can consume projections and issue commands through `AetherEditorState`, but
/// cannot reach the mutable fields that establish workspace/overlay invariants.
mod local_state {
    use std::collections::BTreeSet;
    use std::time::{Duration, Instant};

    use gpui::{Pixels, Point};

    use crate::settings::EditorSettings;

    use super::{
        MENU_AIM_MAX_DELAY_MS, MENU_AIM_TRIANGLE_PADDING, MenuAimPoint, TOP_MENU_BAR_HEIGHT,
        TOP_MENU_PANEL_WIDTH, TOP_MENU_START_X, contains_point_in_triangle, is_view_pill_key,
        top_menu_button_width, update_editor_setting_value,
    };

    const TOP_MENU_NAMES: &[&str] = &["File", "Edit", "View", "Run", "Session", "Help"];
    const WORKSPACE_MODES: &[&str] = &[
        "scene",
        "materials",
        "scripting",
        "gamedata",
        "sequencer",
        "animation",
        "profiler",
    ];
    const LEFT_TABS: &[&str] = &["hierarchy", "layers"];
    const RIGHT_TABS: &[&str] = &["details", "prefab"];
    const VIEW_TABS: &[&str] = &["perspective", "game"];
    const BOTTOM_TABS: &[&str] = &["assets", "console", "output", "profiler", "gems"];
    const MODAL_KINDS: &[&str] = &[
        "about",
        "preferences",
        "asset-create",
        "asset-rename",
        "asset-delete",
    ];

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct PreferencesModalState {
        snapshot: EditorSettings,
        draft: EditorSettings,
    }

    impl PreferencesModalState {
        fn new(settings: EditorSettings) -> Self {
            Self {
                snapshot: settings.clone(),
                draft: settings,
            }
        }

        fn draft(&self) -> &EditorSettings {
            &self.draft
        }

        fn update(&mut self, key: &str, value: &str) -> Option<EditorSettings> {
            update_editor_setting_value(&mut self.draft, key, value).then(|| self.draft.clone())
        }

        fn reset_to_defaults(&mut self) -> Option<EditorSettings> {
            let defaults = EditorSettings::default();
            if self.draft == defaults {
                return None;
            }
            self.draft = defaults;
            Some(self.draft.clone())
        }

        fn cancel(self) -> EditorSettings {
            self.snapshot
        }

        fn done(self) -> EditorSettings {
            self.draft
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct AetherWorkspaceProjection {
        pub(in crate::app) mode: String,
        pub(in crate::app) left_tab: String,
        pub(in crate::app) right_tab: String,
        pub(in crate::app) view_tab: String,
        pub(in crate::app) bottom_tab: String,
        pub(in crate::app) show_left: bool,
        pub(in crate::app) show_right: bool,
        pub(in crate::app) show_bottom: bool,
        pub(in crate::app) left_w: f32,
        pub(in crate::app) right_w: f32,
        pub(in crate::app) bottom_h: f32,
    }

    #[derive(Debug, Clone)]
    pub(super) struct AetherWorkspaceState {
        mode: String,
        left_tab: String,
        right_tab: String,
        view_tab: String,
        bottom_tab: String,
        show_left: bool,
        show_right: bool,
        show_bottom: bool,
        left_w: f32,
        right_w: f32,
        bottom_h: f32,
        resize_start_x: f32,
        resize_start_y: f32,
        resize_start_size: f32,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(in crate::app) struct AetherOverlayProjection {
        pub(in crate::app) build_open: bool,
        pub(in crate::app) menu_open: bool,
        pub(in crate::app) open_menu: String,
        pub(in crate::app) level_open: bool,
        pub(in crate::app) layout_open: bool,
        pub(in crate::app) pipe_open: bool,
        pub(in crate::app) modal_open: bool,
        pub(in crate::app) modal_kind: String,
        pub(in crate::app) modal_category: String,
        pub(in crate::app) select_popover_open: bool,
        pub(in crate::app) select_popover_key: String,
    }

    #[derive(Debug, Clone)]
    pub(super) struct AetherOverlayState {
        build_open: bool,
        menu_open: bool,
        open_menu: String,
        menu_pointer_previous: Option<Point<Pixels>>,
        menu_pointer_current: Option<Point<Pixels>>,
        menu_aim_pending_menu: Option<String>,
        menu_aim_pending_since: Option<Instant>,
        level_open: bool,
        layout_open: bool,
        pipe_open: bool,
        modal_open: bool,
        modal_kind: String,
        modal_category: String,
        preferences_session: Option<PreferencesModalState>,
        select_popover_open: bool,
        select_popover_key: String,
        add_component_search: String,
        view_pill_menus: BTreeSet<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherPanelCapabilities {
        pub(in crate::app) left: bool,
        pub(in crate::app) right: bool,
        pub(in crate::app) bottom: bool,
    }

    impl AetherPanelCapabilities {
        pub(super) const fn new(left: bool, right: bool, bottom: bool) -> Self {
            Self {
                left,
                right,
                bottom,
            }
        }

        /// Derive left/right/bottom dock availability from the same
        /// `WorkspaceLayoutProfile` that `apply_workspace_layout` uses, so the
        /// toggle buttons never disagree with what the dock area actually mounts
        /// for a given mode.
        pub(super) fn for_mode(mode: &str) -> Self {
            let profile = crate::workspace::layout::layout_profile_for_mode(mode);
            Self::new(
                !profile.left.is_empty(),
                !profile.right.is_empty(),
                !profile.bottom.is_empty(),
            )
        }
    }

    impl AetherWorkspaceState {
        pub(super) fn new() -> Self {
            Self {
                mode: "scene".to_owned(),
                left_tab: "hierarchy".to_owned(),
                right_tab: "details".to_owned(),
                view_tab: "perspective".to_owned(),
                bottom_tab: "assets".to_owned(),
                show_left: true,
                show_right: true,
                show_bottom: true,
                left_w: 264.0,
                right_w: 340.0,
                bottom_h: 240.0,
                resize_start_x: 0.0,
                resize_start_y: 0.0,
                resize_start_size: 0.0,
            }
        }

        pub(super) fn projection(&self) -> AetherWorkspaceProjection {
            AetherWorkspaceProjection {
                mode: self.mode.clone(),
                left_tab: self.left_tab.clone(),
                right_tab: self.right_tab.clone(),
                view_tab: self.view_tab.clone(),
                bottom_tab: self.bottom_tab.clone(),
                show_left: self.show_left,
                show_right: self.show_right,
                show_bottom: self.show_bottom,
                left_w: self.left_w,
                right_w: self.right_w,
                bottom_h: self.bottom_h,
            }
        }

        pub(super) fn restore_mode(&mut self, mode: &str) {
            if mode != "scene" {
                self.set_mode(mode);
            }
        }

        pub(super) fn set_mode(&mut self, mode: &str) -> bool {
            if !WORKSPACE_MODES.contains(&mode) || self.mode == mode {
                return false;
            }
            self.mode = mode.to_owned();
            let profile = crate::workspace::layout::layout_profile_for_mode(mode);
            self.show_left = profile
                .left
                .iter()
                .any(|placement| placement.visibility.is_open());
            self.show_right = profile
                .right
                .iter()
                .any(|placement| placement.visibility.is_open());
            self.show_bottom = profile
                .bottom
                .iter()
                .any(|placement| placement.visibility.is_open());
            true
        }

        pub(super) fn panel_capabilities(&self) -> AetherPanelCapabilities {
            AetherPanelCapabilities::for_mode(&self.mode)
        }

        pub(super) fn select_left_tab(&mut self, tab: &str) -> bool {
            if !self.panel_capabilities().left || !LEFT_TABS.contains(&tab) {
                return false;
            }
            let changed = self.left_tab != tab || !self.show_left;
            self.left_tab = tab.to_owned();
            self.show_left = true;
            changed
        }

        pub(super) fn select_right_tab(&mut self, tab: &str) -> bool {
            if !self.panel_capabilities().right || !RIGHT_TABS.contains(&tab) {
                return false;
            }
            let changed = self.right_tab != tab || !self.show_right;
            self.right_tab = tab.to_owned();
            self.show_right = true;
            changed
        }

        pub(super) fn select_bottom_tab(&mut self, tab: &str) -> bool {
            if !self.panel_capabilities().bottom || !BOTTOM_TABS.contains(&tab) {
                return false;
            }
            let changed = self.bottom_tab != tab || !self.show_bottom;
            self.bottom_tab = tab.to_owned();
            self.show_bottom = true;
            changed
        }

        pub(super) fn toggle_left_dock(&mut self, tab: &str) -> bool {
            if !self.panel_capabilities().left {
                return false;
            }
            if self.left_tab == tab {
                self.show_left = !self.show_left;
                return true;
            }
            self.select_left_tab(tab)
        }

        pub(super) fn toggle_right_dock(&mut self, tab: &str) -> bool {
            if !self.panel_capabilities().right {
                return false;
            }
            if self.right_tab == tab {
                self.show_right = !self.show_right;
                return true;
            }
            self.select_right_tab(tab)
        }

        pub(super) fn toggle_bottom_dock(&mut self, tab: &str) -> bool {
            if !self.panel_capabilities().bottom {
                return false;
            }
            if self.bottom_tab == tab {
                self.show_bottom = !self.show_bottom;
                return true;
            }
            self.select_bottom_tab(tab)
        }

        pub(super) fn select_view_tab(&mut self, tab: &str) -> bool {
            if !VIEW_TABS.contains(&tab) || self.view_tab == tab {
                return false;
            }
            self.view_tab = tab.to_owned();
            true
        }

        pub(super) fn toggle_left_panel(&mut self) {
            if self.panel_capabilities().left {
                self.show_left = !self.show_left;
            }
        }

        pub(super) fn toggle_right_panel(&mut self) {
            if self.panel_capabilities().right {
                self.show_right = !self.show_right;
            }
        }

        pub(super) fn toggle_bottom_panel(&mut self) {
            if self.panel_capabilities().bottom {
                self.show_bottom = !self.show_bottom;
            }
        }

        pub(super) fn resize_left_from_start(&mut self, start_width: f32, delta_x: f32) {
            self.left_w = (start_width + delta_x).clamp(180.0, 420.0);
        }

        pub(super) fn resize_right_from_start(&mut self, start_width: f32, delta_x: f32) {
            self.right_w = (start_width - delta_x).clamp(240.0, 520.0);
        }

        pub(super) fn resize_bottom_from_start(&mut self, start_height: f32, delta_y: f32) {
            self.bottom_h = (start_height - delta_y).clamp(150.0, 420.0);
        }

        pub(super) fn begin_left_resize(&mut self, position: Point<Pixels>) -> (f32, f32) {
            self.resize_start_x = position.x.as_f32();
            self.resize_start_size = self.left_w;
            (self.resize_start_x, self.resize_start_size)
        }

        pub(super) fn begin_right_resize(&mut self, position: Point<Pixels>) -> (f32, f32) {
            self.resize_start_x = position.x.as_f32();
            self.resize_start_size = self.right_w;
            (self.resize_start_x, self.resize_start_size)
        }

        pub(super) fn begin_bottom_resize(&mut self, position: Point<Pixels>) -> (f32, f32) {
            self.resize_start_y = position.y.as_f32();
            self.resize_start_size = self.bottom_h;
            (self.resize_start_y, self.resize_start_size)
        }

        pub(super) fn resize_left_to(&mut self, position: Point<Pixels>) {
            self.resize_left_from_start(
                self.resize_start_size,
                position.x.as_f32() - self.resize_start_x,
            );
        }

        pub(super) fn resize_right_to(&mut self, position: Point<Pixels>) {
            self.resize_right_from_start(
                self.resize_start_size,
                position.x.as_f32() - self.resize_start_x,
            );
        }

        pub(super) fn resize_bottom_to(&mut self, position: Point<Pixels>) {
            self.resize_bottom_from_start(
                self.resize_start_size,
                position.y.as_f32() - self.resize_start_y,
            );
        }
    }

    impl AetherOverlayState {
        pub(super) fn new() -> Self {
            Self {
                build_open: false,
                menu_open: false,
                open_menu: String::new(),
                menu_pointer_previous: None,
                menu_pointer_current: None,
                menu_aim_pending_menu: None,
                menu_aim_pending_since: None,
                level_open: false,
                layout_open: false,
                pipe_open: false,
                modal_open: false,
                modal_kind: String::new(),
                modal_category: "appearance".to_owned(),
                preferences_session: None,
                select_popover_open: false,
                select_popover_key: String::new(),
                add_component_search: String::new(),
                view_pill_menus: BTreeSet::new(),
            }
        }

        pub(super) fn projection(&self) -> AetherOverlayProjection {
            AetherOverlayProjection {
                build_open: self.build_open,
                menu_open: self.menu_open,
                open_menu: self.open_menu.clone(),
                level_open: self.level_open,
                layout_open: self.layout_open,
                pipe_open: self.pipe_open,
                modal_open: self.modal_open,
                modal_kind: self.modal_kind.clone(),
                modal_category: self.modal_category.clone(),
                select_popover_open: self.select_popover_open,
                select_popover_key: self.select_popover_key.clone(),
            }
        }

        pub(super) fn open_preferences(&mut self, settings: EditorSettings) {
            self.modal_open = true;
            self.modal_kind = "preferences".to_owned();
            if self.modal_category.is_empty() {
                self.modal_category = "appearance".to_owned();
            }
            if self.preferences_session.is_none() {
                self.preferences_session = Some(PreferencesModalState::new(settings));
            }
            self.select_popover_open = false;
            self.select_popover_key.clear();
        }

        pub(super) fn update_preferences_draft(
            &mut self,
            key: &str,
            value: &str,
        ) -> Option<EditorSettings> {
            self.preferences_session.as_mut()?.update(key, value)
        }

        pub(super) fn select_preferences_category(&mut self, key: &str) -> bool {
            if key.is_empty() || !(self.modal_open && self.modal_kind == "preferences") {
                return false;
            }
            let changed = self.modal_category != key
                || self.select_popover_open
                || !self.select_popover_key.is_empty();
            self.modal_category = key.to_owned();
            self.select_popover_open = false;
            self.select_popover_key.clear();
            changed
        }

        pub(super) fn apply_settings_option(
            &mut self,
            key: &str,
            value: &str,
        ) -> Option<EditorSettings> {
            let updated = self.update_preferences_draft(key, value);
            self.select_popover_open = false;
            self.select_popover_key.clear();
            updated
        }

        pub(super) fn reset_preferences_draft(&mut self) -> Option<EditorSettings> {
            self.preferences_session.as_mut()?.reset_to_defaults()
        }

        pub(super) fn finish_preferences(&mut self, persist: bool) -> Option<EditorSettings> {
            if !(self.modal_open && self.modal_kind == "preferences") {
                self.close_modal();
                return None;
            }

            let Some(session) = self.preferences_session.take() else {
                self.close_modal();
                return None;
            };
            let settings = if persist {
                session.done()
            } else {
                session.cancel()
            };
            self.close_modal();
            Some(settings)
        }

        pub(super) fn switch_open_menu_from_title_hover(&mut self, name: &str) -> bool {
            if !TOP_MENU_NAMES.contains(&name) || !self.menu_open || self.open_menu == name {
                return false;
            }
            self.open_menu = name.to_owned();
            self.menu_aim_pending_menu = None;
            self.menu_aim_pending_since = None;
            true
        }

        pub(super) fn close_select(&mut self) {
            self.select_popover_open = false;
            self.select_popover_key.clear();
            self.add_component_search.clear();
        }

        pub(super) fn view_pill_is_open(&self, key: &str) -> bool {
            self.view_pill_menus.contains(key)
        }

        pub(super) fn set_view_pill_open(&mut self, key: &str, open: bool) -> bool {
            if !is_view_pill_key(key) {
                return false;
            }
            if open {
                self.view_pill_menus.insert(key.to_owned())
            } else {
                self.view_pill_menus.remove(key)
            }
        }

        pub(super) fn close_view_pills(&mut self) {
            self.view_pill_menus.clear();
        }

        pub(super) fn has_open_view_pill(&self) -> bool {
            !self.view_pill_menus.is_empty()
        }

        pub(super) fn close_modal(&mut self) {
            self.modal_open = false;
            self.modal_kind.clear();
            self.preferences_session = None;
            self.select_popover_open = false;
            self.select_popover_key.clear();
        }

        pub(super) fn open_modal(&mut self, kind: &str) {
            if !MODAL_KINDS.contains(&kind) {
                return;
            }
            self.modal_open = true;
            self.modal_kind = kind.to_owned();
            self.select_popover_open = false;
            self.select_popover_key.clear();
        }

        pub(super) fn open_about_modal(&mut self) {
            self.open_modal("about");
            self.preferences_session = None;
        }

        pub(super) fn toggle_menu(&mut self, name: &str) -> bool {
            if !TOP_MENU_NAMES.contains(&name) {
                return false;
            }
            if self.menu_open && self.open_menu == name {
                self.menu_open = false;
                self.open_menu.clear();
                false
            } else {
                self.menu_open = true;
                self.open_menu = name.to_owned();
                true
            }
        }

        pub(super) fn track_menu_pointer(&mut self, position: Point<Pixels>) -> Option<String> {
            self.menu_pointer_previous = self.menu_pointer_current.take();
            self.menu_pointer_current = Some(position.clone());

            if !self.menu_open {
                self.clear_menu_aim();
                return None;
            }

            let pointer = MenuAimPoint::from_gpui(position.clone());
            if pointer.y > TOP_MENU_BAR_HEIGHT {
                self.clear_menu_aim();
                return None;
            }

            let pending_menu = self.menu_aim_pending_menu.clone()?;
            if self.should_delay_menu_switch(&position) {
                return None;
            }

            self.open_menu = pending_menu.clone();
            self.clear_menu_aim();
            Some(pending_menu)
        }

        pub(super) fn clear_menu_aim(&mut self) {
            self.menu_aim_pending_menu = None;
            self.menu_aim_pending_since = None;
        }

        pub(super) fn menu_open(&self) -> bool {
            self.menu_open
        }

        pub(super) fn menu_open_for(&self, name: &str) -> bool {
            self.menu_open && self.open_menu == name
        }

        pub(super) fn schedule_menu_aim(&mut self, name: &str) {
            if !TOP_MENU_NAMES.contains(&name) || !self.menu_open || self.open_menu == name {
                return;
            }
            self.menu_aim_pending_menu = Some(name.to_owned());
            self.menu_aim_pending_since = Some(Instant::now());
        }

        pub(super) fn close_menu(&mut self) {
            self.menu_open = false;
            self.open_menu.clear();
            self.clear_menu_aim();
        }

        pub(super) fn toggle_build(&mut self) -> bool {
            self.build_open = !self.build_open;
            self.build_open
        }

        pub(super) fn close_build(&mut self) {
            self.build_open = false;
        }

        pub(super) fn close_build_and_menu(&mut self) {
            self.build_open = false;
            self.close_menu();
        }

        pub(super) fn toggle_level(&mut self) -> bool {
            self.level_open = !self.level_open;
            self.level_open
        }

        pub(super) fn close_level(&mut self) {
            self.level_open = false;
        }

        pub(super) fn toggle_layout(&mut self) -> bool {
            self.layout_open = !self.layout_open;
            self.layout_open
        }

        pub(super) fn close_layout(&mut self) {
            self.layout_open = false;
        }

        pub(super) fn toggle_pipe(&mut self) -> bool {
            self.pipe_open = !self.pipe_open;
            self.pipe_open
        }

        pub(super) fn close_pipe(&mut self) {
            self.pipe_open = false;
        }

        pub(super) fn open_add_component_select(&mut self) {
            self.select_popover_open = true;
            self.select_popover_key = "add-component".to_owned();
            self.add_component_search.clear();
        }

        pub(super) fn set_add_component_search(&mut self, value: &str) -> bool {
            if self.add_component_search == value {
                return false;
            }
            self.add_component_search = value.to_owned();
            true
        }

        pub(super) fn add_component_search(&self) -> &str {
            &self.add_component_search
        }

        pub(super) fn add_component_select_open(&self) -> bool {
            self.select_popover_key == "add-component"
        }

        pub(super) fn preferences_draft(&self) -> Option<&EditorSettings> {
            (self.modal_open && self.modal_kind == "preferences")
                .then_some(())
                .and_then(|()| self.preferences_session.as_ref())
                .map(PreferencesModalState::draft)
        }

        pub(super) fn has_preferences_session(&self) -> bool {
            self.preferences_session.is_some()
        }

        pub(super) fn reset_preferences_presentation(&mut self) {
            self.modal_category = "appearance".to_owned();
            self.select_popover_open = false;
            self.select_popover_key.clear();
        }

        fn should_delay_menu_switch(&self, position: &Point<Pixels>) -> bool {
            if !self.menu_open || self.open_menu.is_empty() {
                return false;
            }

            if self
                .menu_aim_pending_since
                .is_some_and(|since| since.elapsed() > Duration::from_millis(MENU_AIM_MAX_DELAY_MS))
            {
                return false;
            }

            let Some(previous_position) = self.menu_pointer_previous.as_ref() else {
                return false;
            };
            let previous = MenuAimPoint::from_gpui(previous_position.clone());
            let current = MenuAimPoint::from_gpui(position.clone());
            if previous.y > TOP_MENU_BAR_HEIGHT
                || current.y > TOP_MENU_BAR_HEIGHT
                || current.y <= previous.y
            {
                return false;
            }

            let mut left = TOP_MENU_START_X;
            let Some((panel_left, panel_right)) = TOP_MENU_NAMES.iter().find_map(|name| {
                let width = top_menu_button_width(name);
                let bounds = (left, left + TOP_MENU_PANEL_WIDTH);
                left += width;
                (*name == self.open_menu).then_some(bounds)
            }) else {
                return false;
            };
            let near_left = MenuAimPoint {
                x: panel_left - MENU_AIM_TRIANGLE_PADDING,
                y: TOP_MENU_BAR_HEIGHT,
            };
            let near_right = MenuAimPoint {
                x: panel_right + MENU_AIM_TRIANGLE_PADDING,
                y: TOP_MENU_BAR_HEIGHT,
            };
            contains_point_in_triangle(current, previous, near_left, near_right)
        }
    }
}

mod asset_browser_state {
    use super::*;

    fn replace_if_changed(slot: &mut String, value: &str) -> bool {
        if slot == value {
            return false;
        }
        slot.clear();
        slot.push_str(value);
        true
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherAssetBrowserNavigation<'a> {
        pub(in crate::app) folder: &'a str,
        pub(in crate::app) can_go_back: bool,
        pub(in crate::app) can_go_forward: bool,
        pub(in crate::app) grid: bool,
        pub(in crate::app) search: &'a str,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherAssetSelection<'a> {
        pub(in crate::app) key: &'a str,
        pub(in crate::app) source_path: &'a str,
        pub(in crate::app) schema_type: &'a str,
        pub(in crate::app) name: &'a str,
        pub(in crate::app) icon: &'a str,
        pub(in crate::app) color: &'a str,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherAssetCreateDraft<'a> {
        pub(in crate::app) schema_type: &'a str,
        pub(in crate::app) name: &'a str,
        pub(in crate::app) folder: &'a str,
        pub(in crate::app) error: &'a str,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherAssetRenameDraft<'a> {
        pub(in crate::app) source_root: &'a str,
        pub(in crate::app) from_path: &'a str,
        pub(in crate::app) to_path: &'a str,
        pub(in crate::app) error: &'a str,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherAssetDeleteDraft<'a> {
        pub(in crate::app) source_root: &'a str,
        pub(in crate::app) source_path: &'a str,
        pub(in crate::app) error: &'a str,
    }

    #[derive(Debug, Clone)]
    pub(super) struct AssetBrowserSelection {
        pub(super) key: String,
        pub(super) source_path: String,
        pub(super) schema_type: String,
        pub(super) name: String,
        pub(super) icon: String,
        pub(super) color: String,
        pub(super) latest_job_attempt_id: Option<i64>,
    }

    #[derive(Debug, Clone)]
    pub(super) struct AetherAssetBrowserState {
        folder: String,
        folder_back: Vec<String>,
        folder_forward: Vec<String>,
        grid: bool,
        search: String,
        selected_key: String,
        selected_source_path: String,
        selected_schema_type: String,
        selected_name: String,
        selected_icon: String,
        selected_color: String,
        selected_latest_job_attempt_id: Option<i64>,
        create_schema_type: String,
        create_name: String,
        create_folder: String,
        create_error: String,
        rename_source_root: String,
        rename_from_path: String,
        rename_to_path: String,
        rename_error: String,
        delete_source_root: String,
        delete_source_path: String,
        delete_error: String,
    }

    impl AetherAssetBrowserState {
        pub(super) fn new() -> Self {
            Self {
                folder: String::new(),
                folder_back: Vec::new(),
                folder_forward: Vec::new(),
                grid: true,
                search: String::new(),
                selected_key: String::new(),
                selected_source_path: String::new(),
                selected_schema_type: String::new(),
                selected_name: String::new(),
                selected_icon: String::new(),
                selected_color: String::new(),
                selected_latest_job_attempt_id: None,
                create_schema_type: String::new(),
                create_name: String::new(),
                create_folder: String::new(),
                create_error: String::new(),
                rename_source_root: String::new(),
                rename_from_path: String::new(),
                rename_to_path: String::new(),
                rename_error: String::new(),
                delete_source_root: String::new(),
                delete_source_path: String::new(),
                delete_error: String::new(),
            }
        }

        pub(super) fn navigation(&self) -> AetherAssetBrowserNavigation<'_> {
            AetherAssetBrowserNavigation {
                folder: &self.folder,
                can_go_back: !self.folder_back.is_empty(),
                can_go_forward: !self.folder_forward.is_empty(),
                grid: self.grid,
                search: &self.search,
            }
        }

        pub(super) fn selection(&self) -> AetherAssetSelection<'_> {
            AetherAssetSelection {
                key: &self.selected_key,
                source_path: &self.selected_source_path,
                schema_type: &self.selected_schema_type,
                name: &self.selected_name,
                icon: &self.selected_icon,
                color: &self.selected_color,
            }
        }

        pub(super) fn create_draft(&self) -> AetherAssetCreateDraft<'_> {
            AetherAssetCreateDraft {
                schema_type: &self.create_schema_type,
                name: &self.create_name,
                folder: &self.create_folder,
                error: &self.create_error,
            }
        }

        pub(super) fn rename_draft(&self) -> AetherAssetRenameDraft<'_> {
            AetherAssetRenameDraft {
                source_root: &self.rename_source_root,
                from_path: &self.rename_from_path,
                to_path: &self.rename_to_path,
                error: &self.rename_error,
            }
        }

        pub(super) fn delete_draft(&self) -> AetherAssetDeleteDraft<'_> {
            AetherAssetDeleteDraft {
                source_root: &self.delete_source_root,
                source_path: &self.delete_source_path,
                error: &self.delete_error,
            }
        }

        pub(super) fn search(&mut self, value: &str) -> bool {
            replace_if_changed(&mut self.search, value)
        }

        pub(super) fn choose_grid_layout(&mut self, grid: bool) -> bool {
            let changed = self.grid != grid;
            self.grid = grid;
            changed
        }

        pub(super) fn edit_create_name(&mut self, value: &str) -> bool {
            let changed = replace_if_changed(&mut self.create_name, value);
            self.create_error.clear();
            changed
        }

        pub(super) fn edit_create_folder(&mut self, value: &str) -> bool {
            let changed = replace_if_changed(&mut self.create_folder, value);
            self.create_error.clear();
            changed
        }

        pub(super) fn edit_rename_target(&mut self, value: &str) -> bool {
            let changed = replace_if_changed(&mut self.rename_to_path, value);
            self.rename_error.clear();
            changed
        }

        pub(super) fn begin_create(
            &mut self,
            schema_type: String,
            folder: String,
            unavailable_error: Option<String>,
        ) {
            self.create_schema_type = schema_type;
            self.create_folder = folder;
            self.create_error = unavailable_error.unwrap_or_default();
        }

        pub(super) fn choose_create_schema(
            &mut self,
            schema_type: &str,
            default_folder: Option<String>,
        ) -> bool {
            if self.create_schema_type == schema_type {
                return false;
            }
            self.create_schema_type = schema_type.to_owned();
            if self.create_folder.trim().is_empty() {
                self.create_folder = default_folder.unwrap_or_default();
            }
            self.create_error.clear();
            true
        }

        pub(super) fn reject_create(&mut self, error: String) {
            self.create_error = error;
        }

        pub(super) fn commit_create(&mut self, selection: AssetBrowserSelection) {
            self.create_error.clear();
            self.select_entry(selection);
        }

        pub(super) fn begin_rename(&mut self, source_root: String, source_path: String) {
            self.rename_source_root = source_root;
            self.rename_from_path = source_path.clone();
            self.rename_to_path = source_path;
            self.rename_error.clear();
        }

        pub(super) fn reject_rename(&mut self, error: String) {
            self.rename_error = error;
        }

        pub(super) fn commit_rename(
            &mut self,
            from_source_path: &str,
            to_source_path: &str,
        ) -> bool {
            self.rename_error.clear();
            if self.selected_source_path != from_source_path {
                return false;
            }
            self.selected_key.clear();
            self.selected_source_path = to_source_path.to_owned();
            self.selected_name = asset_display_name(to_source_path);
            !self.selected_schema_type.trim().is_empty()
        }

        pub(super) fn begin_delete(&mut self, source_root: String, source_path: String) {
            self.delete_source_root = source_root;
            self.delete_source_path = source_path;
            self.delete_error.clear();
        }

        pub(super) fn reject_delete(&mut self, error: String) {
            self.delete_error = error;
        }

        pub(super) fn commit_delete(&mut self, source_path: &str) -> bool {
            self.delete_error.clear();
            if self.selected_source_path != source_path {
                return false;
            }
            let was_authored_document = !self.selected_schema_type.trim().is_empty();
            self.selected_key.clear();
            self.selected_source_path.clear();
            self.selected_schema_type.clear();
            self.selected_name.clear();
            self.selected_icon.clear();
            self.selected_color.clear();
            self.selected_latest_job_attempt_id = None;
            was_authored_document
        }

        pub(super) fn clear_modal_errors(&mut self) {
            self.create_error.clear();
            self.rename_error.clear();
            self.delete_error.clear();
        }

        pub(super) fn select_entry(&mut self, selection: AssetBrowserSelection) {
            self.selected_key = selection.key;
            self.selected_source_path = selection.source_path;
            self.selected_schema_type = selection.schema_type;
            self.selected_name = selection.name;
            self.selected_icon = selection.icon;
            self.selected_color = selection.color;
            self.selected_latest_job_attempt_id = selection.latest_job_attempt_id;
        }

        pub(super) fn select_folder(&mut self, key: &str) -> bool {
            if self.folder == key {
                return false;
            }
            self.folder_back.push(self.folder.clone());
            self.folder_forward.clear();
            self.folder = key.to_owned();
            true
        }

        pub(super) fn navigate_back(&mut self) -> bool {
            let Some(previous) = self.folder_back.pop() else {
                return false;
            };
            self.folder_forward.push(self.folder.clone());
            self.folder = previous;
            true
        }

        pub(super) fn navigate_forward(&mut self) -> bool {
            let Some(next) = self.folder_forward.pop() else {
                return false;
            };
            self.folder_back.push(self.folder.clone());
            self.folder = next;
            true
        }
    }
}

#[path = "aether_authored_content_state.rs"]
mod aether_authored_content_state;
#[path = "aether_diagnostics_state.rs"]
mod aether_diagnostics_state;

// GameData rail navigation and gem selection have unrelated display
// invariants. They remain sibling private aggregates instead of becoming a
// catch-all catalog-selection state.
mod catalog_selection_state {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherGameDataRail<'a> {
        pub(in crate::app) view: &'a str,
        pub(in crate::app) tab: &'a str,
        pub(in crate::app) search: &'a str,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GameDataRailRoute {
        Table,
        Field,
        Schema,
        Manager,
    }

    impl GameDataRailRoute {
        const fn view(self) -> &'static str {
            match self {
                Self::Table | Self::Field => "tables",
                Self::Schema => "schemas",
                Self::Manager => "managers",
            }
        }

        const fn tab(self) -> &'static str {
            match self {
                Self::Table => "table",
                Self::Field => "field",
                Self::Schema => "schema",
                Self::Manager => "manager",
            }
        }

        fn from_view(view: &str) -> Option<Self> {
            match view {
                "tables" => Some(Self::Table),
                "schemas" => Some(Self::Schema),
                "managers" => Some(Self::Manager),
                _ => None,
            }
        }

        fn from_tab(tab: &str) -> Option<Self> {
            match tab {
                "table" => Some(Self::Table),
                "field" => Some(Self::Field),
                "schema" => Some(Self::Schema),
                "manager" => Some(Self::Manager),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) enum AetherGameDataRowCount {
        Loading,
        Known(u64),
    }

    impl AetherGameDataRowCount {
        pub(in crate::app) fn label(self) -> String {
            match self {
                Self::Loading => "loading".to_owned(),
                Self::Known(count) => count.to_string(),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) enum AetherGameDataTableSelection<'a> {
        Empty,
        Selected {
            table_key: &'a str,
            name: &'a str,
            category: &'a str,
            row_count: AetherGameDataRowCount,
        },
    }

    impl<'a> AetherGameDataTableSelection<'a> {
        pub(in crate::app) const fn table_key(self) -> &'a str {
            match self {
                Self::Empty => "",
                Self::Selected { table_key, .. } => table_key,
            }
        }

        pub(in crate::app) const fn name(self) -> &'a str {
            match self {
                Self::Empty => "",
                Self::Selected { name, .. } => name,
            }
        }

        pub(in crate::app) const fn category(self) -> &'a str {
            match self {
                Self::Empty => "",
                Self::Selected { category, .. } => category,
            }
        }

        pub(in crate::app) const fn row_count(self) -> Option<AetherGameDataRowCount> {
            match self {
                Self::Empty => None,
                Self::Selected { row_count, .. } => Some(row_count),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherGameDataSchemaSelection<'a> {
        pub(in crate::app) key: &'a str,
    }

    #[derive(Debug, Clone)]
    struct SelectedGameDataTable {
        table_key: String,
        name: String,
        category: String,
        row_count: AetherGameDataRowCount,
    }

    #[derive(Debug, Clone)]
    pub(super) struct AetherGameDataState {
        route: GameDataRailRoute,
        search: String,
        selected_table: Option<SelectedGameDataTable>,
        selected_schema_key: String,
    }

    impl AetherGameDataState {
        pub(super) fn new() -> Self {
            Self {
                route: GameDataRailRoute::Table,
                search: String::new(),
                selected_table: None,
                selected_schema_key: String::new(),
            }
        }

        pub(super) fn rail(&self) -> AetherGameDataRail<'_> {
            AetherGameDataRail {
                view: self.route.view(),
                tab: self.route.tab(),
                search: &self.search,
            }
        }

        pub(super) fn table_selection(&self) -> AetherGameDataTableSelection<'_> {
            self.selected_table
                .as_ref()
                .map_or(AetherGameDataTableSelection::Empty, |table| {
                    AetherGameDataTableSelection::Selected {
                        table_key: &table.table_key,
                        name: &table.name,
                        category: &table.category,
                        row_count: table.row_count,
                    }
                })
        }

        pub(super) fn schema_selection(&self) -> AetherGameDataSchemaSelection<'_> {
            AetherGameDataSchemaSelection {
                key: &self.selected_schema_key,
            }
        }

        pub(super) fn select_view(&mut self, view: &str) -> bool {
            let Some(route) = GameDataRailRoute::from_view(view) else {
                return false;
            };
            let changed = self.route != route;
            self.route = route;
            changed
        }

        pub(super) fn select_tab(&mut self, tab: &str) -> bool {
            let Some(route) = GameDataRailRoute::from_tab(tab) else {
                return false;
            };
            let changed = self.route != route;
            self.route = route;
            changed
        }

        pub(super) fn select_table(&mut self, table: &GameDataTableDescriptor) -> bool {
            let changed = self.selected_table.as_ref().is_none_or(|selected| {
                selected.table_key != table.name
                    || selected.name != table.name
                    || selected.category != table.category
                    || selected.row_count
                        != table.row_count.map_or(
                            AetherGameDataRowCount::Loading,
                            AetherGameDataRowCount::Known,
                        )
            }) || self.selected_schema_key != table.schema_type
                || self.route != GameDataRailRoute::Table;
            self.selected_table = Some(SelectedGameDataTable {
                table_key: table.name.clone(),
                name: table.name.clone(),
                category: table.category.clone(),
                row_count: table.row_count.map_or(
                    AetherGameDataRowCount::Loading,
                    AetherGameDataRowCount::Known,
                ),
            });
            self.selected_schema_key = table.schema_type.clone();
            self.route = GameDataRailRoute::Table;
            changed
        }

        pub(super) fn select_schema(&mut self, schema_key: &str) -> bool {
            let changed =
                self.selected_schema_key != schema_key || self.route != GameDataRailRoute::Schema;
            self.selected_schema_key = schema_key.to_owned();
            self.route = GameDataRailRoute::Schema;
            changed
        }

        pub(super) fn search(&mut self, search: &str) -> bool {
            if self.search == search {
                return false;
            }
            self.search = search.to_owned();
            true
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::app) struct AetherGemSelection<'a> {
        pub(in crate::app) id: &'a str,
    }

    #[derive(Debug, Clone, Default)]
    pub(super) struct AetherGemSelectionState {
        id: String,
    }

    impl AetherGemSelectionState {
        pub(super) fn selection(&self) -> AetherGemSelection<'_> {
            AetherGemSelection { id: &self.id }
        }

        pub(super) fn select(&mut self, gem_id: &str) -> bool {
            if self.id == gem_id {
                return false;
            }
            self.id = gem_id.to_owned();
            true
        }
    }
}

use aether_authored_content_state::{
    AetherAuthoredContentState, AetherAuthoredExpansion, AetherAuthoredSourcePaths,
};
use aether_diagnostics_state::{AetherDiagnosticsPresentation, AetherDiagnosticsState};
use asset_browser_state::{
    AetherAssetBrowserNavigation, AetherAssetBrowserState, AetherAssetCreateDraft,
    AetherAssetDeleteDraft, AetherAssetRenameDraft, AetherAssetSelection, AssetBrowserSelection,
};
use catalog_selection_state::{
    AetherGameDataRail, AetherGameDataSchemaSelection, AetherGameDataState,
    AetherGameDataTableSelection, AetherGemSelection, AetherGemSelectionState,
};
use local_state::{
    AetherOverlayProjection, AetherOverlayState, AetherPanelCapabilities,
    AetherWorkspaceProjection, AetherWorkspaceState,
};

impl AetherEditorState {
    pub(crate) fn new() -> Self {
        Self {
            workspace: AetherWorkspaceState::new(),
            overlay: AetherOverlayState::new(),
            assets: AetherAssetBrowserState::new(),
            authored_content: AetherAuthoredContentState::default(),
            game_data: AetherGameDataState::new(),
            gems: AetherGemSelectionState::default(),
            diagnostics: AetherDiagnosticsState::default(),
        }
    }

    pub(super) fn workspace_projection(&self) -> AetherWorkspaceProjection {
        self.workspace.projection()
    }

    pub(super) fn overlay_projection(&self) -> AetherOverlayProjection {
        self.overlay.projection()
    }

    pub(super) fn asset_browser_navigation(&self) -> AetherAssetBrowserNavigation<'_> {
        self.assets.navigation()
    }

    pub(super) fn asset_selection(&self) -> AetherAssetSelection<'_> {
        self.assets.selection()
    }

    pub(super) fn asset_create_draft(&self) -> AetherAssetCreateDraft<'_> {
        self.assets.create_draft()
    }

    pub(super) fn asset_rename_draft(&self) -> AetherAssetRenameDraft<'_> {
        self.assets.rename_draft()
    }

    pub(super) fn asset_delete_draft(&self) -> AetherAssetDeleteDraft<'_> {
        self.assets.delete_draft()
    }

    fn authored_expansion(&self) -> AetherAuthoredExpansion<'_> {
        self.authored_content.expansion()
    }

    fn authored_source_paths(&self) -> AetherAuthoredSourcePaths<'_> {
        self.authored_content.source_paths()
    }

    pub(super) fn diagnostics_presentation(&self) -> AetherDiagnosticsPresentation<'_> {
        self.diagnostics.presentation()
    }

    fn game_data_rail(&self) -> AetherGameDataRail<'_> {
        self.game_data.rail()
    }

    fn game_data_table_selection(&self) -> AetherGameDataTableSelection<'_> {
        self.game_data.table_selection()
    }

    fn game_data_schema_selection(&self) -> AetherGameDataSchemaSelection<'_> {
        self.game_data.schema_selection()
    }

    pub(super) fn gem_selection(&self) -> AetherGemSelection<'_> {
        self.gems.selection()
    }

    pub(super) fn search_assets_state(&mut self, value: &str) -> bool {
        self.assets.search(value)
    }

    pub(super) fn choose_asset_grid_layout_state(&mut self, grid: bool) -> bool {
        self.assets.choose_grid_layout(grid)
    }

    pub(super) fn edit_asset_create_name_state(&mut self, value: &str) -> bool {
        self.assets.edit_create_name(value)
    }

    pub(super) fn edit_asset_create_folder_state(&mut self, value: &str) -> bool {
        self.assets.edit_create_folder(value)
    }

    pub(super) fn edit_asset_rename_target_state(&mut self, value: &str) -> bool {
        self.assets.edit_rename_target(value)
    }

    pub(super) fn begin_asset_create_state(
        &mut self,
        schema_type: String,
        folder: String,
        unavailable_error: Option<String>,
    ) {
        self.assets
            .begin_create(schema_type, folder, unavailable_error);
    }

    pub(super) fn choose_asset_create_schema_state(
        &mut self,
        schema_type: &str,
        default_folder: Option<String>,
    ) -> bool {
        self.assets
            .choose_create_schema(schema_type, default_folder)
    }

    pub(super) fn reject_asset_create_state(&mut self, error: String) {
        self.assets.reject_create(error);
    }

    pub(super) fn commit_asset_create_state(
        &mut self,
        schema_type: String,
        source_path: String,
        color: String,
    ) {
        self.assets.commit_create(AssetBrowserSelection {
            key: String::new(),
            name: asset_display_name(&source_path),
            icon: "description".to_owned(),
            latest_job_attempt_id: None,
            source_path,
            schema_type,
            color,
        });
    }

    pub(super) fn select_asset_state(&mut self, item: &AetherItem) {
        self.assets.select_entry(AssetBrowserSelection {
            key: item.key.clone(),
            source_path: item.src.clone(),
            schema_type: item.type_label.clone(),
            name: item.name.clone(),
            icon: item.icon.clone(),
            color: item.color.clone(),
            latest_job_attempt_id: item.idx.parse::<i64>().ok(),
        });
    }

    pub(super) fn begin_asset_rename_state(&mut self, source_root: String, source_path: String) {
        self.assets.begin_rename(source_root, source_path);
    }

    pub(super) fn reject_asset_rename_state(&mut self, error: String) {
        self.assets.reject_rename(error);
    }

    pub(super) fn commit_asset_rename_state(
        &mut self,
        from_source_path: &str,
        to_source_path: &str,
    ) -> bool {
        self.assets.commit_rename(from_source_path, to_source_path)
    }

    pub(super) fn begin_asset_delete_state(&mut self, source_root: String, source_path: String) {
        self.assets.begin_delete(source_root, source_path);
    }

    pub(super) fn reject_asset_delete_state(&mut self, error: String) {
        self.assets.reject_delete(error);
    }

    pub(super) fn commit_asset_delete_state(&mut self, source_path: &str) -> bool {
        self.assets.commit_delete(source_path)
    }

    fn clear_asset_modal_errors(&mut self) {
        self.assets.clear_modal_errors();
    }

    pub(crate) fn restore_mode(&mut self, mode: &str) {
        self.workspace.restore_mode(mode);
    }

    fn remapped_authored_source_path<'a>(&'a self, source_path: &'a str) -> &'a str {
        self.authored_source_paths().remapped(source_path)
    }

    pub(super) fn record_authored_source_path_move(&mut self, from: String, to: String) {
        self.authored_content.record_source_path_move(from, to);
    }

    pub(super) fn clear_resolved_authored_source_path_moves(
        &mut self,
        outline: &EditorAuthoredOutline,
    ) -> bool {
        self.authored_content
            .clear_resolved_source_path_moves(outline)
    }

    pub(super) fn select_asset_folder_state(&mut self, key: &str) -> bool {
        self.assets.select_folder(key)
    }

    pub(super) fn navigate_asset_back_state(&mut self) -> bool {
        self.assets.navigate_back()
    }

    pub(super) fn navigate_asset_forward_state(&mut self) -> bool {
        self.assets.navigate_forward()
    }

    pub(super) fn trace_summary(&self) -> String {
        let workspace = self.workspace.projection();
        let overlay = self.overlay.projection();
        let browser = self.assets.navigation();
        let game_data_rail = self.game_data.rail();
        let game_data_table = self.game_data.table_selection();
        let game_data_schema = self.game_data.schema_selection();
        let gem = self.gems.selection();
        let diagnostics = self.diagnostics.presentation();
        format!(
            concat!(
                "mode={} left_tab={} right_tab={} view_tab={} bottom_tab={} ",
                "asset_folder={} ",
                "menu_open={} open_menu={} pending_menu={} build_open={} level_open={} layout_open={} pipe_open={} ",
                "modal_open={} modal_kind={} modal_category={} select_popover_open={} select_popover_key={} show_stats={} show_left={} show_right={} show_bottom={} ",
                "left_w={:.1} right_w={:.1} bottom_h={:.1} ",
                "asset_grid={} gd_view={} gd_tab={} selected_table={} selected_schema={} selected_gem={} console_filter={:?} console_query={}"
            ),
            workspace.mode,
            workspace.left_tab,
            workspace.right_tab,
            workspace.view_tab,
            workspace.bottom_tab,
            browser.folder,
            overlay.menu_open,
            overlay.open_menu,
            "",
            overlay.build_open,
            overlay.level_open,
            overlay.layout_open,
            overlay.pipe_open,
            overlay.modal_open,
            overlay.modal_kind,
            overlay.modal_category,
            overlay.select_popover_open,
            overlay.select_popover_key,
            diagnostics.show_stats,
            workspace.show_left,
            workspace.show_right,
            workspace.show_bottom,
            workspace.left_w,
            workspace.right_w,
            workspace.bottom_h,
            browser.grid,
            game_data_rail.view,
            game_data_rail.tab,
            game_data_table.table_key(),
            game_data_schema.key,
            gem.id,
            diagnostics.console.filter,
            diagnostics.console.query
        )
    }

    pub(super) fn set_mode_state(&mut self, mode: &str) -> bool {
        self.workspace.set_mode(mode)
    }

    pub(super) fn panel_capabilities(&self) -> AetherPanelCapabilities {
        self.workspace.panel_capabilities()
    }

    pub(super) fn select_left_tab_state(&mut self, tab: &str) -> bool {
        self.workspace.select_left_tab(tab)
    }

    pub(super) fn select_right_tab_state(&mut self, tab: &str) -> bool {
        self.workspace.select_right_tab(tab)
    }

    pub(super) fn select_bottom_tab_state(&mut self, tab: &str) -> bool {
        self.workspace.select_bottom_tab(tab)
    }

    pub(super) fn select_gem_state(&mut self, gem_id: &str) {
        self.gems.select(gem_id);
        self.select_bottom_tab_state("gems");
    }

    pub(super) fn switch_open_menu_from_title_hover_state(&mut self, name: &str) -> bool {
        self.overlay.switch_open_menu_from_title_hover(name)
    }

    pub(super) fn set_game_data_view_state(&mut self, view: &str) -> bool {
        self.game_data.select_view(view)
    }

    pub(super) fn set_game_data_tab_state(&mut self, tab: &str) -> bool {
        self.game_data.select_tab(tab)
    }

    fn set_game_data_search_state(&mut self, search: &str) -> bool {
        self.game_data.search(search)
    }

    pub(super) fn select_gamedata_table_state(&mut self, table: &GameDataTableDescriptor) -> bool {
        self.game_data.select_table(table)
    }

    pub(super) fn select_gamedata_schema_state(&mut self, schema_key: &str) -> bool {
        self.game_data.select_schema(schema_key)
    }

    pub(super) fn set_console_filter_state(&mut self, filter: AetherConsoleFilter) -> bool {
        self.diagnostics.select_console_filter(filter)
    }

    pub(super) fn set_console_query_state(&mut self, query: &str) -> bool {
        self.diagnostics.set_console_query(query)
    }

    pub(super) fn clear_console_query_state(&mut self) -> bool {
        self.diagnostics.clear_console_query()
    }

    pub(super) fn toggle_stats_state(&mut self) {
        self.diagnostics.toggle_stats();
    }

    pub(super) fn toggle_left_dock_state(&mut self, tab: &str) -> bool {
        self.workspace.toggle_left_dock(tab)
    }

    pub(super) fn toggle_right_dock_state(&mut self, tab: &str) -> bool {
        self.workspace.toggle_right_dock(tab)
    }

    pub(super) fn toggle_bottom_dock_state(&mut self, tab: &str) -> bool {
        self.workspace.toggle_bottom_dock(tab)
    }

    pub(super) fn toggle_left_panel_state(&mut self) {
        self.workspace.toggle_left_panel();
    }

    pub(super) fn toggle_right_panel_state(&mut self) {
        self.workspace.toggle_right_panel();
    }

    pub(super) fn toggle_bottom_panel_state(&mut self) {
        self.workspace.toggle_bottom_panel();
    }

    pub(super) fn resize_left_from_start_state(&mut self, start_width: f32, delta_x: f32) {
        self.workspace.resize_left_from_start(start_width, delta_x);
    }

    pub(super) fn resize_right_from_start_state(&mut self, start_width: f32, delta_x: f32) {
        self.workspace.resize_right_from_start(start_width, delta_x);
    }

    pub(super) fn resize_bottom_from_start_state(&mut self, start_height: f32, delta_y: f32) {
        self.workspace
            .resize_bottom_from_start(start_height, delta_y);
    }

    pub(super) fn begin_left_resize_state(&mut self, position: Point<Pixels>) -> (f32, f32) {
        self.workspace.begin_left_resize(position)
    }

    pub(super) fn begin_right_resize_state(&mut self, position: Point<Pixels>) -> (f32, f32) {
        self.workspace.begin_right_resize(position)
    }

    pub(super) fn begin_bottom_resize_state(&mut self, position: Point<Pixels>) -> (f32, f32) {
        self.workspace.begin_bottom_resize(position)
    }

    pub(super) fn resize_left_to_state(&mut self, position: Point<Pixels>) {
        self.workspace.resize_left_to(position);
    }

    pub(super) fn resize_right_to_state(&mut self, position: Point<Pixels>) {
        self.workspace.resize_right_to(position);
    }

    pub(super) fn resize_bottom_to_state(&mut self, position: Point<Pixels>) {
        self.workspace.resize_bottom_to(position);
    }

    pub(super) fn select_view_tab_state(&mut self, tab: &str) -> bool {
        self.workspace.select_view_tab(tab)
    }

    pub(super) fn toggle_menu_state(&mut self, name: &str) -> bool {
        self.overlay.toggle_menu(name)
    }

    pub(super) fn track_menu_pointer_state(&mut self, position: Point<Pixels>) -> Option<String> {
        self.overlay.track_menu_pointer(position)
    }

    pub(super) fn menu_open_state(&self) -> bool {
        self.overlay.menu_open()
    }

    pub(super) fn menu_open_for_state(&self, name: &str) -> bool {
        self.overlay.menu_open_for(name)
    }

    fn schedule_menu_aim_state(&mut self, name: &str) {
        self.overlay.schedule_menu_aim(name);
    }

    pub(super) fn close_menu_state(&mut self) {
        self.overlay.close_menu();
    }

    pub(super) fn open_overlay_modal_state(&mut self, kind: &str) {
        self.overlay.open_modal(kind);
    }

    pub(super) fn open_about_modal_state(&mut self) {
        self.overlay.open_about_modal();
    }

    pub(super) fn toggle_build_state(&mut self) -> bool {
        self.overlay.toggle_build()
    }

    pub(super) fn close_build_state(&mut self) {
        self.overlay.close_build();
    }

    pub(super) fn close_build_and_menu_state(&mut self) {
        self.overlay.close_build_and_menu();
    }

    pub(super) fn toggle_level_state(&mut self) -> bool {
        self.overlay.toggle_level()
    }

    pub(super) fn close_level_state(&mut self) {
        self.overlay.close_level();
    }

    pub(super) fn toggle_layout_state(&mut self) -> bool {
        self.overlay.toggle_layout()
    }

    pub(super) fn close_layout_state(&mut self) {
        self.overlay.close_layout();
    }

    pub(super) fn toggle_pipe_state(&mut self) -> bool {
        self.overlay.toggle_pipe()
    }

    pub(super) fn close_pipe_state(&mut self) {
        self.overlay.close_pipe();
    }

    pub(super) fn open_add_component_select_state(&mut self) {
        self.overlay.open_add_component_select();
    }

    pub(super) fn set_add_component_search_state(&mut self, value: &str) -> bool {
        self.overlay.set_add_component_search(value)
    }

    pub(super) fn add_component_search(&self) -> &str {
        self.overlay.add_component_search()
    }

    pub(super) fn add_component_select_open(&self) -> bool {
        self.overlay.add_component_select_open()
    }

    pub(super) fn preferences_draft(&self) -> Option<&EditorSettings> {
        self.overlay.preferences_draft()
    }

    pub(super) fn has_preferences_session(&self) -> bool {
        self.overlay.has_preferences_session()
    }

    pub(super) fn reset_preferences_presentation(&mut self) {
        self.overlay.reset_preferences_presentation();
    }

    pub(super) fn close_overlay_modal_state(&mut self) {
        self.overlay.close_modal();
    }

    pub(super) fn item_expanded(&self, key: &str, default_open: bool) -> bool {
        if is_view_pill_key(key) {
            return self.overlay.view_pill_is_open(key);
        }
        self.authored_expansion().is_open(key, default_open)
    }

    pub(super) fn set_item_expanded(&mut self, key: &str, open: bool) -> bool {
        if is_view_pill_key(key) {
            return self.overlay.set_view_pill_open(key, open);
        }
        self.authored_content.set_expanded(key, open)
    }

    pub(super) fn open_preferences_modal(&mut self, settings: EditorSettings) {
        self.overlay.open_preferences(settings);
    }

    pub(super) fn update_preferences_draft(
        &mut self,
        key: &str,
        value: &str,
    ) -> Option<EditorSettings> {
        self.overlay.update_preferences_draft(key, value)
    }

    pub(super) fn select_preferences_category_state(&mut self, key: &str) -> bool {
        self.overlay.select_preferences_category(key)
    }

    fn apply_settings_option_state(&mut self, key: &str, value: &str) -> Option<EditorSettings> {
        self.overlay.apply_settings_option(key, value)
    }

    pub(super) fn reset_preferences_draft(&mut self) -> Option<EditorSettings> {
        self.overlay.reset_preferences_draft()
    }

    pub(super) fn cancel_preferences_modal_state(&mut self) -> Option<EditorSettings> {
        let settings = self.overlay.finish_preferences(false);
        self.clear_asset_modal_errors();
        settings
    }

    pub(super) fn confirm_preferences_modal_state(&mut self) -> Option<EditorSettings> {
        let settings = self.overlay.finish_preferences(true);
        self.clear_asset_modal_errors();
        settings
    }

    pub(super) fn close_modal_state(&mut self) {
        self.overlay.close_modal();
        self.clear_asset_modal_errors();
    }

    pub(super) fn close_select_state(&mut self) {
        self.overlay.close_select();
    }

    fn close_view_pill_menus_state(&mut self) {
        self.overlay.close_view_pills();
    }

    pub(super) fn view_pill_menu_open(&self) -> bool {
        self.overlay.has_open_view_pill()
    }

    pub(super) fn dismiss_top_layer_state(&mut self) -> OverlayDismissal {
        let overlay = self.overlay.projection();
        if overlay.select_popover_open {
            self.close_select_state();
            return OverlayDismissal::Closed;
        }
        if overlay.modal_open {
            if overlay.modal_kind == "preferences"
                && let Some(settings) = self.cancel_preferences_modal_state()
            {
                return OverlayDismissal::ApplySettings(settings);
            }
            self.close_modal_state();
            return OverlayDismissal::Closed;
        }
        if overlay.level_open {
            self.overlay.close_level();
            return OverlayDismissal::Closed;
        }
        if overlay.layout_open {
            self.overlay.close_layout();
            return OverlayDismissal::Closed;
        }
        if overlay.pipe_open {
            self.overlay.close_pipe();
            return OverlayDismissal::Closed;
        }
        if self.view_pill_menu_open() {
            self.close_view_pill_menus_state();
            return OverlayDismissal::Closed;
        }
        if overlay.build_open || overlay.menu_open {
            self.overlay.close_build_and_menu();
            return OverlayDismissal::Closed;
        }
        OverlayDismissal::None
    }

    pub(super) fn inner_layer_click_state(&mut self) -> OverlayDismissal {
        OverlayDismissal::None
    }
}

impl AetherItem {
    fn trace_item_interaction(&self, event: &str) {
        trace_aether_ui_interaction(
            event,
            format!(
                "item key={} name={} label={} title={} active={} selected={} sep={}",
                trace_value(&self.key),
                trace_value(&self.name),
                trace_value(&self.label),
                trace_value(&self.title),
                self.active,
                self.selected,
                self.sep
            ),
        );
    }

    fn dispatch_authored_text_edit(
        &self,
        binding: Option<&ReflectedEditBinding>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) -> bool {
        let Some(binding) = binding else {
            return false;
        };
        let payload = if self.edit_text_quoted {
            ron::ser::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
        } else {
            value.trim().to_owned()
        };
        tracing::info!(
            target: "az_editor::aether_ui",
            field = %self.label,
            "editing Aether reflected inspector text field"
        );
        window.dispatch_action(
            Box::new(az_editor_ui::actions::ApplyReflectedPrefabEdit {
                command: binding.set_value(ReflectedValueEnvelope {
                    type_path: self.edit_type_path.clone(),
                    encoding: ReflectedValueEncoding::TypedRon,
                    payload: payload.into_bytes(),
                }),
            }),
            cx,
        );
        cx.stop_propagation();
        true
    }

    fn dispatch_authored_edit_value(
        &self,
        binding: Option<&ReflectedEditBinding>,
        value: ReflectedValueEnvelope,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) -> bool {
        let Some(binding) = binding else {
            return false;
        };
        window.dispatch_action(
            Box::new(az_editor_ui::actions::ApplyReflectedPrefabEdit {
                command: binding.set_value(value),
            }),
            cx,
        );
        cx.stop_propagation();
        true
    }

    fn dispatch_authored_command(
        &self,
        command: Option<&PrefabEditCommand>,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) -> bool {
        let Some(command) = command else {
            return false;
        };
        window.dispatch_action(
            Box::new(az_editor_ui::actions::ApplyReflectedPrefabEdit {
                command: command.clone(),
            }),
            cx,
        );
        cx.stop_propagation();
        true
    }

    pub(crate) fn on_field(
        &self,
        _value: impl AsRef<str>,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_input(
        &self,
        _value: impl AsRef<str>,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_name(
        &self,
        _value: impl AsRef<str>,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_proj_type(
        &self,
        _value: impl AsRef<str>,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_rename(
        &self,
        _value: impl AsRef<str>,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_val(
        &self,
        value: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        self.dispatch_authored_text_edit(self.edit_binding.as_ref(), value.as_ref(), window, cx);
    }
    pub(crate) fn on_x(
        &self,
        value: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        self.dispatch_authored_text_edit(self.x_binding.as_ref(), value.as_ref(), window, cx);
    }
    pub(crate) fn on_y(
        &self,
        value: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        self.dispatch_authored_text_edit(self.y_binding.as_ref(), value.as_ref(), window, cx);
    }
    pub(crate) fn on_z(
        &self,
        value: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        self.dispatch_authored_text_edit(self.z_binding.as_ref(), value.as_ref(), window, cx);
    }
    pub(crate) fn close_layout_menu<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn close_level_menu<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn close_menu<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn close_modal<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn close_pipe<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn close_select<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn close_view_menu<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn go_assets<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn go_console<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn go_output<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn go_profiler<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn handle_click<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn jump<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn left_toggle<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_add<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_add_cond<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_back<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_blend_down<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_bottom_drag<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_caret<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_caret");
    }
    pub(crate) fn on_clear_console<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_click<E>(
        &self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_click");
        match self.kind.as_str() {
            "authored-remove-path" => {
                tracing::info!(
                    target: "az_editor::aether_ui",
                    component = %self.comp,
                    "removing Aether inspector component"
                );
                self.dispatch_authored_command(self.edit_command.as_ref(), window, cx);
            }
            "authored-initialize-path" => {
                self.dispatch_authored_command(self.edit_command.as_ref(), window, cx);
            }
            "authored-enum-option" => {
                self.dispatch_authored_command(self.edit_command.as_ref(), window, cx);
            }
            "add-component-schema" => {
                if !self.disabled && self.edit_command.is_some() {
                    tracing::info!(
                        target: "az_editor::aether_ui",
                        component_schema = %self.key,
                        "adding Aether inspector component"
                    );
                    self.dispatch_authored_command(self.edit_command.as_ref(), window, cx);
                }
            }
            "prefab-source-document" if !self.key.is_empty() => {
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAuthoredDocument {
                        document_id: self.key.clone(),
                    }),
                    cx,
                );
                cx.stop_propagation();
            }
            _ => {}
        }
    }
    pub(crate) fn on_close<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_create<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_cycle_op<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_del<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_delete<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_down<E>(
        &self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        match self.kind.as_str() {
            "anim-fragment" | "anim-fragment-option" | "anim-fragment-transition"
                if !non_empty_string_or(&self.src, &self.key).is_empty() =>
            {
                let fragment_key = non_empty_string_or(&self.src, &self.key);
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectMannequinFragment { fragment_key }),
                    cx,
                );
                cx.stop_propagation();
            }
            "anim-clip" if !non_empty_string_or(&self.src, &self.key).is_empty() => {
                let motion_glb = non_empty_string_or(&self.src, &self.key);
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAnimationMotion { motion_glb }),
                    cx,
                );
                cx.stop_propagation();
            }
            "anim-blend-space" if !non_empty_string_or(&self.src, &self.key).is_empty() => {
                let bspace_ron_path = non_empty_string_or(&self.src, &self.key);
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAnimationBlendSpace { bspace_ron_path }),
                    cx,
                );
                cx.stop_propagation();
            }
            "anim-blend-param" if !self.key.is_empty() => {
                let min = self.x_val.parse::<f32>().unwrap_or(0.0);
                let max = self.y_val.parse::<f32>().unwrap_or(1.0);
                let current = self.value.parse::<f32>().unwrap_or((min + max) * 0.5);
                let span = (max - min).abs().max(0.0001);
                let mut next = current + span * 0.25;
                if next > max.max(min) {
                    next = min.min(max);
                }
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SetAnimationBlendSpaceParameter {
                        dimension: self.key.clone(),
                        value: next,
                    }),
                    cx,
                );
                cx.stop_propagation();
            }
            "anim-blend-sample" => {
                let values = self
                    .items
                    .0
                    .iter()
                    .filter_map(|item| item.val.parse::<f32>().ok())
                    .collect::<Vec<_>>();
                if !values.is_empty() {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::SetAnimationBlendSpaceParameters {
                            values,
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }
    pub(crate) fn on_enable<E>(
        &self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        if let Some(value) = self.edit_value.clone() {
            tracing::info!(
                target: "az_editor::aether_ui",
                component = %self.name,
                "toggling Aether inspector component enabled state"
            );
            self.dispatch_authored_edit_value(self.edit_binding.as_ref(), value, window, cx);
        }
    }
    pub(crate) fn on_enter<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_fire<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_grid_view<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_idx<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_left_drag<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_list_view<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_lock<E>(
        &self,
        _event: &E,
        window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_lock");
        if self.kind == "layer" && !self.key.is_empty() {
            window.dispatch_action(
                Box::new(az_editor_ui::actions::SetLayerLock {
                    document_id: self.key.clone(),
                    locked: self.lock_icon != "lock",
                }),
                _cx,
            );
            _cx.stop_propagation();
        }
    }
    pub(crate) fn on_mirror<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_mute<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_next<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_open<E>(
        &self,
        _event: &E,
        window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_open");
        match self.kind.as_str() {
            "level" if !self.key.is_empty() => {
                tracing::info!(
                    document = %self.key,
                    level = %self.name,
                    "switching active Aether level"
                );
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAuthoredDocument {
                        document_id: self.key.clone(),
                    }),
                    _cx,
                );
                _cx.stop_propagation();
            }
            "level-action" => {
                match self.key.as_str() {
                    "new-level" => window.dispatch_action(
                        Box::new(az_editor_ui::actions::CreateAuthoredDocument {
                            root_schema: ENGINE_PREFAB_ROOT_SCHEMA_TYPE.to_owned(),
                        }),
                        _cx,
                    ),
                    "save-level" => {
                        window.dispatch_action(Box::new(az_editor_ui::actions::Save), _cx);
                    }
                    "refresh-levels" => {
                        window.dispatch_action(
                            Box::new(az_editor_ui::actions::RefreshAuthoredOutline),
                            _cx,
                        );
                    }
                    _ => {}
                }
                _cx.stop_propagation();
            }
            _ => {}
        }
    }
    pub(crate) fn on_phase<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_play_click<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_prio_down<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_prio_up<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_remove<E>(
        &self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        tracing::info!(
            target: "az_editor::aether_ui",
            component = %self.comp,
            "removing Aether inspector authored path"
        );
        self.dispatch_authored_command(self.edit_command.as_ref(), window, cx);
    }
    pub(crate) fn on_revert<E>(
        &self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        tracing::info!(
            target: "az_editor::aether_ui",
            field = %self.field,
            "reverting Aether prefab override"
        );
        self.dispatch_authored_command(self.edit_command.as_ref(), window, cx);
    }
    pub(crate) fn on_right_drag<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_row_click<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_row_click");
    }
    pub(crate) fn on_select<E>(
        &self,
        _event: &E,
        window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_select");
        if self.kind == "layer" && !self.key.is_empty() {
            window.dispatch_action(
                Box::new(az_editor_ui::actions::SelectAuthoredDocument {
                    document_id: self.key.clone(),
                }),
                _cx,
            );
            _cx.stop_propagation();
        }
    }
    pub(crate) fn on_sim_click<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_step_click<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_stop_click<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_sync<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_toggle<E>(
        &self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_toggle");
        if self.kind == "anim-tag" && !self.key.is_empty() {
            window.dispatch_action(
                Box::new(az_editor_ui::actions::SetMannequinTag {
                    tag: self.key.clone(),
                    enabled: !self.active,
                }),
                cx,
            );
            cx.stop_propagation();
            return;
        }
        if let Some(value) = self.edit_value.clone() {
            self.dispatch_authored_edit_value(self.edit_binding.as_ref(), value, window, cx);
        }
    }
    pub(crate) fn on_val_down<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_vis<E>(
        &self,
        _event: &E,
        window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        self.trace_item_interaction("item.on_vis");
        if self.kind == "layer" && !self.key.is_empty() {
            window.dispatch_action(
                Box::new(az_editor_ui::actions::SetLayerVisibility {
                    document_id: self.key.clone(),
                    visible: self.vis_icon != "visibility",
                }),
                _cx,
            );
            _cx.stop_propagation();
        }
    }
    pub(crate) fn on_x_down<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn on_y_down<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn open_settings<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn reset_modal<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn stop_prop<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
        _cx.stop_propagation();
    }
    pub(crate) fn toggle_angle_snap<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn toggle_grid_snap<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn toggle_layout_menu<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn toggle_level_menu<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
    pub(crate) fn toggle_pipe<E>(
        &self,
        _event: &E,
        _window: &mut Window,
        _cx: &mut Context<AetherEditorView>,
    ) {
    }
}

pub(super) fn update_editor_setting_value(
    settings: &mut EditorSettings,
    key: &str,
    value: &str,
) -> bool {
    let before = settings.clone();
    match key {
        "theme" => settings.theme = value.to_owned(),
        "keymap_profile" => settings.keymap_profile = value.to_owned(),
        "source_navigation.file_url_template" => {
            settings.source_navigation.file_url_template = value.to_owned();
        }
        _ => return false,
    }
    before != *settings
}

const VIEW_PILL_KEYS: &[&str] = &["cam", "shade", "show", "giz"];

pub(super) fn is_view_pill_key(key: &str) -> bool {
    VIEW_PILL_KEYS.contains(&key)
}

pub(super) fn trace_value(value: &str) -> &str {
    if value.is_empty() { "<empty>" } else { value }
}

#[cfg(test)]
mod tests {
    use super::super::aether_editor_view_render::{
        asset_browser::{
            asset_browser_empty_message, asset_category_counts, asset_entries_for_folder,
            asset_entry_category, asset_entry_item, asset_folder_category, asset_item_source_root,
            filtered_asset_entries, selection_file_type_projection,
        },
        authored_content::scene_prefab::{
            active_authored_document_id, hierarchy_rows_from_outline_state, level_action_items,
            level_document_meta, level_documents, level_items_from_outline, scene_document_counts,
        },
        diagnostics_preview::{
            AssetPipelineCounts, aether_bottom_tabs, asset_pipeline_counts, asset_status_summary,
            build_status_label, build_status_summary, runtime_play_icon, session_status_summary,
        },
        gamedata_gem::selected_gem_id_from_catalog,
        presentation::hsla_css,
        workspace_overlay::menus_settings::{
            SettingOption, SettingsControlProjection, aether_menu_definitions,
            settings_row_projection, source_control_projection,
        },
    };
    use super::{
        AetherConsoleFilter, AetherEditorState, AetherGameDataTableSelection,
        AetherOverlayProjection, AetherWorkspaceProjection, OverlayDismissal,
    };
    use crate::app::aether_common::AetherItem;
    use crate::mannequin_animation::{
        BlendSpacePreviewAction, MannequinAuthoringAction, apply_blend_space_preview_action,
        apply_mannequin_authoring_action, apply_resolved_mannequin_preview,
        build_blend_space_preview_catalog, build_mannequin_authoring_catalog,
    };
    use crate::settings::EditorSettings;
    use az_editor_ui::panels::{
        AssetBrowserEntryData, AssetBrowserEntryStatus, AssetBrowserJobData, AssetBrowserJobStatus,
        AuthoredDocumentOutlineData, AuthoredObjectOutlineData, AuthoredOutlineData,
        ConsoleLevelCounts, CreatableAuthoredSchemaData, EditorAnimationEventData,
        EditorAnimationJointData, EditorAnimationMotionData, EditorAnimationPreviewCatalog,
        EditorAssetBrowserStatus, EditorAssetProcessorActivity, EditorAuthoredOutline,
        EditorBlendSpaceAssetData, EditorBlendSpaceAssetKind, EditorBlendSpaceCoordinateData,
        EditorBlendSpaceData, EditorBlendSpaceDimensionData, EditorBlendSpaceExampleData,
        EditorBlendSpacePreview, EditorBlendSpacePreviewCatalog,
        EditorBlendSpaceVirtualExampleData, EditorCreatableAuthoredSchemas, EditorLayerVisibility,
        EditorMannequinAuthoringCatalog, EditorMannequinPreview, EditorRuntimeStateData,
        EditorRuntimeStatus, EditorSessionStateData, EditorSessionStatus, LogLevel, OutputLogState,
        SessionProcessData, SessionProcessStateData, SessionServiceRoleData, WorkspaceRootData,
        asset_browser_folders, authored_layer_rows, project_workflow,
    };
    use az_editor_ui::status::ServiceHealthStateData;
    use az_editor_ui::{EditorGemCatalog, EditorGemInfo, EditorGemSelection};
    use az_proto_project::GameDataTableDescriptor;
    use std::fs;
    #[test]
    fn selecting_dock_tabs_reveals_their_panels() {
        let mut state = AetherEditorState::new();
        state.toggle_left_panel_state();
        state.toggle_right_panel_state();
        state.toggle_bottom_panel_state();

        assert!(state.select_left_tab_state("layers"));
        let workspace = state.workspace_projection();
        assert_eq!(workspace.left_tab, "layers");
        assert!(workspace.show_left);

        assert!(state.select_right_tab_state("prefab"));
        let workspace = state.workspace_projection();
        assert_eq!(workspace.right_tab, "prefab");
        assert!(workspace.show_right);

        assert!(state.select_bottom_tab_state("console"));
        let workspace = state.workspace_projection();
        assert_eq!(workspace.bottom_tab, "console");
        assert!(workspace.show_bottom);
    }

    #[test]
    fn menu_hover_switches_open_dropdown_from_title() {
        let mut state = AetherEditorState::new();
        assert!(state.toggle_menu_state("File"));
        state.schedule_menu_aim_state("File");

        assert!(state.switch_open_menu_from_title_hover_state("Edit"));
        let overlay = state.overlay_projection();
        assert_eq!(overlay.open_menu, "Edit");
        assert!(overlay.menu_open);
    }

    #[test]
    fn preferences_modal_category_select_and_theme_choice_update_state() {
        let mut state = AetherEditorState::new();
        let mut settings = EditorSettings {
            theme: "default-dark".to_owned(),
            ..EditorSettings::default()
        };
        state.open_preferences_modal(settings.clone());

        assert!(state.select_preferences_category_state("keymap"));
        let overlay = state.overlay_projection();
        assert_eq!(overlay.modal_category, "keymap");
        assert!(!overlay.select_popover_open);

        assert!(state.select_preferences_category_state("appearance"));
        let overlay = state.overlay_projection();
        assert!(!overlay.select_popover_open);
        assert!(overlay.select_popover_key.is_empty());

        settings.theme = "default-light".to_owned();
        let updated = state
            .apply_settings_option_state("theme", "default-light")
            .expect("theme option should update draft settings");
        assert_eq!(updated, settings);
        assert_eq!(state.preferences_draft().unwrap().theme, "default-light");
        let overlay = state.overlay_projection();
        assert!(!overlay.select_popover_open);
        assert!(overlay.select_popover_key.is_empty());
    }

    #[test]
    fn bottom_tabs_console_badge_reflects_warning_and_error_counts() {
        let theme = gpui_component::theme::Theme::default();
        let mut counts = ConsoleLevelCounts::default();
        let tabs = aether_bottom_tabs(counts, &theme);
        let console = tabs.iter().find(|tab| tab.key == "console").unwrap();
        assert!(!console.has_badge);

        counts.warn = 2;
        counts.error = 1;
        let tabs = aether_bottom_tabs(counts, &theme);
        let console = tabs.iter().find(|tab| tab.key == "console").unwrap();
        assert!(console.has_badge);
        assert_eq!(console.badge, "3");
        let expected_warning = hsla_css(theme.warning);
        assert_eq!(
            console.badge_style.get("background"),
            Some(expected_warning.as_str())
        );
    }

    #[test]
    fn console_query_state_filters_source_or_text() {
        let mut state = AetherEditorState::new();

        assert!(state.set_console_query_state("asset"));
        assert_eq!(state.diagnostics_presentation().console.query, "asset");
        assert!(!state.set_console_query_state("asset"));
    }

    #[test]
    fn selected_gem_id_prefers_attached_project_enabled_gem() {
        let catalog = EditorGemCatalog::new(vec![
            EditorGemInfo {
                id: "azoth.render".to_owned(),
                name: "Render".to_owned(),
                version: "0.1.0".to_owned(),
                description: String::new(),
                category: "Rendering".to_owned(),
                dependencies: Vec::new(),
                deprecation: None,
            },
            EditorGemInfo {
                id: "azoth.physics".to_owned(),
                name: "Physics".to_owned(),
                version: "0.1.0".to_owned(),
                description: String::new(),
                category: "Simulation".to_owned(),
                dependencies: Vec::new(),
                deprecation: None,
            },
        ]);
        let selection = EditorGemSelection::new(vec!["azoth.physics".to_owned()]);

        assert_eq!(
            selected_gem_id_from_catalog(&catalog, Some(&selection), "").as_deref(),
            Some("azoth.physics")
        );
        assert_eq!(
            selected_gem_id_from_catalog(&catalog, Some(&selection), "azoth.render").as_deref(),
            Some("azoth.render")
        );
    }

    #[test]
    fn gem_row_selection_state_opens_gems_detail() {
        let mut state = AetherEditorState::new();
        state.select_bottom_tab_state("console");

        state.select_gem_state("azoth.render");

        assert_eq!(state.gem_selection().id, "azoth.render");
        let workspace = state.workspace_projection();
        assert_eq!(workspace.bottom_tab, "gems");
        assert!(workspace.show_bottom);
    }

    #[test]
    fn status_panel_toggles_only_the_requested_panel() {
        let mut state = AetherEditorState::new();

        state.toggle_left_panel_state();
        let workspace = state.workspace_projection();
        assert!(!workspace.show_left);
        assert!(workspace.show_right);
        assert!(workspace.show_bottom);

        state.toggle_bottom_panel_state();
        let workspace = state.workspace_projection();
        assert!(!workspace.show_left);
        assert!(workspace.show_right);
        assert!(!workspace.show_bottom);

        state.toggle_right_panel_state();
        let workspace = state.workspace_projection();
        assert!(!workspace.show_left);
        assert!(!workspace.show_right);
        assert!(!workspace.show_bottom);
    }

    #[test]
    fn workspace_projection_tracks_mode_tabs_and_dock_visibility() {
        let mut state = AetherEditorState::new();

        assert!(state.set_mode_state("materials"));
        assert!(state.select_left_tab_state("layers"));
        assert!(!state.toggle_bottom_dock_state("console"));

        assert_eq!(
            state.workspace_projection(),
            AetherWorkspaceProjection {
                mode: "materials".to_owned(),
                left_tab: "layers".to_owned(),
                right_tab: "details".to_owned(),
                view_tab: "perspective".to_owned(),
                bottom_tab: "assets".to_owned(),
                show_left: true,
                show_right: true,
                show_bottom: false,
                left_w: 264.0,
                right_w: 340.0,
                bottom_h: 240.0,
            }
        );
    }

    #[test]
    fn overlay_projection_preserves_preference_draft_and_dismissal() {
        let mut state = AetherEditorState::new();
        let initial = EditorSettings::default();
        state.open_preferences_modal(initial.clone());

        assert!(state.select_preferences_category_state("keymap"));
        let updated = state
            .apply_settings_option_state("keymap_profile", "vim")
            .expect("supported preference option");
        assert_eq!(updated.keymap_profile, "vim");
        assert_eq!(
            state.overlay_projection(),
            AetherOverlayProjection {
                build_open: false,
                menu_open: false,
                open_menu: String::new(),
                level_open: false,
                layout_open: false,
                pipe_open: false,
                modal_open: true,
                modal_kind: "preferences".to_owned(),
                modal_category: "keymap".to_owned(),
                select_popover_open: false,
                select_popover_key: String::new(),
            }
        );

        assert_eq!(
            state.dismiss_top_layer_state(),
            OverlayDismissal::ApplySettings(initial)
        );
        assert!(!state.overlay_projection().modal_open);
    }

    #[test]
    fn console_filter_state_transitions_select_requested_filter() {
        let mut state = AetherEditorState::new();
        assert_eq!(
            state.diagnostics_presentation().console.filter,
            AetherConsoleFilter::All
        );

        assert!(state.set_console_filter_state(AetherConsoleFilter::Warn));
        assert_eq!(
            state.diagnostics_presentation().console.filter,
            AetherConsoleFilter::Warn
        );
        assert!(!state.set_console_filter_state(AetherConsoleFilter::Warn));

        assert!(state.set_console_filter_state(AetherConsoleFilter::Error));
        assert_eq!(
            state.diagnostics_presentation().console.filter,
            AetherConsoleFilter::Error
        );
    }

    #[test]
    fn diagnostics_state_keeps_stats_presentation_local_to_the_editor() {
        let mut state = AetherEditorState::new();

        assert!(state.diagnostics_presentation().show_stats);
        state.toggle_stats_state();
        assert!(!state.diagnostics_presentation().show_stats);
        state.toggle_stats_state();
        assert!(state.diagnostics_presentation().show_stats);
    }

    #[test]
    fn console_filter_counts_group_info_debug_and_trace() {
        let counts = ConsoleLevelCounts {
            error: 2,
            warn: 3,
            info: 5,
            debug: 7,
        };

        assert_eq!(AetherConsoleFilter::All.count_from(counts), 17);
        assert_eq!(AetherConsoleFilter::Info.count_from(counts), 12);
        assert_eq!(AetherConsoleFilter::Warn.count_from(counts), 3);
        assert_eq!(AetherConsoleFilter::Error.count_from(counts), 2);
        assert!(AetherConsoleFilter::Info.shows(LogLevel::Trace));
        assert!(!AetherConsoleFilter::Warn.shows(LogLevel::Info));
    }

    #[test]
    fn aether_menus_are_real_action_items_with_keymap_shortcuts() {
        let menus = aether_menu_definitions();
        let menu_names = menus
            .iter()
            .map(|menu| menu.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            menu_names,
            ["File", "Edit", "View", "Run", "Session", "Help"]
        );

        let all_items = menus
            .iter()
            .flat_map(|menu| menu.items.iter())
            .collect::<Vec<_>>();
        assert!(all_items.iter().any(|item| item.key == "save"));
        assert!(all_items.iter().any(|item| item.key == "preferences"));
        assert!(all_items.iter().any(|item| item.key == "about"));
        assert!(!all_items.iter().any(|item| item.label == "Save As..."));
        assert!(
            !all_items
                .iter()
                .any(|item| item.label == "Project Settings...")
        );
        assert!(all_items.iter().filter(|item| item.notsep).all(|item| {
            item.kind == "menu-action" && !item.key.is_empty() && !item.label.is_empty()
        }));
        assert!(
            all_items
                .iter()
                .filter(|item| item.notsep && !item.disabled)
                .all(|item| item.menu_action.is_some()),
            "every enabled menu row carries a typed GPUI command"
        );
        assert!(
            all_items
                .iter()
                .filter(|item| item.notsep && item.menu_action.is_none())
                .all(|item| item.disabled),
            "planned rows without a command are visibly disabled"
        );

        let save = all_items
            .iter()
            .find(|item| item.key == "save")
            .expect("save menu item");
        assert_eq!(
            save.menu_action,
            Some(crate::app::aether_common::AetherMenuAction::Save)
        );
        assert_eq!(
            save.shortcut,
            crate::keymaps::shortcut_label(
                crate::keymaps::KEYMAP_PROFILE_DEFAULT,
                crate::keymaps::ACTION_SAVE,
            )
            .expect("save shortcut")
        );
    }

    #[test]
    fn settings_rows_project_only_backed_editor_settings() {
        let settings = EditorSettings {
            theme: "Aether:dark".to_owned(),
            keymap_profile: "default".to_owned(),
            source_navigation: az_editor_ui::settings::SourceNavigationSettings {
                file_url_template: "cursor://file/{path}{line_column_suffix}".to_owned(),
            },
            ..Default::default()
        };
        let theme_options = vec![SettingOption {
            value: "aether:dark".to_owned(),
            label: "Aether Dark".to_owned(),
        }];

        let appearance = settings_row_projection("appearance", &settings, &theme_options);
        assert_eq!(appearance.len(), 1);
        assert_eq!(appearance[0].key, "theme");
        assert_eq!(appearance[0].value, "Aether Dark");
        assert_eq!(appearance[0].control, SettingsControlProjection::Select);

        let keymap = settings_row_projection("keymap", &settings, &theme_options);
        assert_eq!(keymap[0].key, "keymap_profile");
        assert_eq!(keymap[0].value, "Default");

        let source = settings_row_projection("source-navigation", &settings, &theme_options);
        assert_eq!(source[0].key, "source_navigation.file_url_template");
        assert_eq!(source[0].value, "cursor://file/{path}{line_column_suffix}");
        assert_eq!(source[0].control, SettingsControlProjection::Text);

        assert!(settings_row_projection("project-settings", &settings, &theme_options).is_empty());
    }

    #[test]
    fn preferences_cancel_reverts_live_draft_to_open_snapshot() {
        let initial = EditorSettings {
            theme: "Aether:dark".to_owned(),
            keymap_profile: "default".to_owned(),
            ..Default::default()
        };
        let mut state = AetherEditorState::new();
        state.open_preferences_modal(initial.clone());

        let live = state
            .update_preferences_draft("theme", "default-light")
            .expect("theme update should be supported");
        assert_eq!(live.theme, "default-light");

        let reverted = state
            .cancel_preferences_modal_state()
            .expect("cancel should produce reverted settings");

        assert_eq!(reverted, initial);
        assert!(!state.overlay_projection().modal_open);
        assert!(!state.has_preferences_session());
    }

    #[test]
    fn reopening_preferences_preserves_the_original_cancel_snapshot() {
        let initial = EditorSettings {
            theme: "Aether:dark".to_owned(),
            keymap_profile: "default".to_owned(),
            ..Default::default()
        };
        let mut state = AetherEditorState::new();
        state.open_preferences_modal(initial.clone());
        let live = state
            .update_preferences_draft("theme", "default-light")
            .expect("theme update should be supported");

        state.open_preferences_modal(live);
        let reverted = state
            .cancel_preferences_modal_state()
            .expect("cancel should produce the original settings");

        assert_eq!(reverted, initial);
    }

    #[test]
    fn preferences_done_persists_the_live_draft() {
        let mut state = AetherEditorState::new();
        state.open_preferences_modal(EditorSettings::default());

        state
            .update_preferences_draft(
                "source_navigation.file_url_template",
                "cursor://file/{path}",
            )
            .expect("source navigation update should be supported");
        let persisted = state
            .confirm_preferences_modal_state()
            .expect("done should produce persisted settings");

        assert_eq!(
            persisted.source_navigation.file_url_template,
            "cursor://file/{path}"
        );
        assert!(!state.overlay_projection().modal_open);
        assert!(!state.has_preferences_session());
    }

    #[test]
    fn preferences_escape_matches_cancel_snapshot_revert() {
        let initial = EditorSettings {
            theme: "Aether:dark".to_owned(),
            keymap_profile: "default".to_owned(),
            ..Default::default()
        };
        let mut state = AetherEditorState::new();
        state.open_preferences_modal(initial.clone());
        state
            .update_preferences_draft("theme", "default-light")
            .expect("theme update should be supported");

        let dismissal = state.dismiss_top_layer_state();

        assert_eq!(dismissal, OverlayDismissal::ApplySettings(initial));
        assert!(!state.overlay_projection().modal_open);
        assert!(!state.has_preferences_session());
    }

    #[test]
    fn inner_layer_click_does_not_dismiss_open_modal() {
        let mut state = AetherEditorState::new();
        state.open_preferences_modal(EditorSettings::default());

        let dismissal = state.inner_layer_click_state();

        assert_eq!(dismissal, OverlayDismissal::None);
        let overlay = state.overlay_projection();
        assert!(overlay.modal_open);
        assert_eq!(overlay.modal_kind, "preferences");
    }

    #[test]
    fn top_layer_dismissal_closes_each_transient_overlay_owner() {
        let mut state = AetherEditorState::new();

        state.open_add_component_select_state();
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        assert!(!state.overlay_projection().select_popover_open);

        state.open_about_modal_state();
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        assert!(!state.overlay_projection().modal_open);

        assert!(state.toggle_level_state());
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        assert!(!state.overlay_projection().level_open);

        assert!(state.toggle_layout_state());
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        assert!(!state.overlay_projection().layout_open);

        assert!(state.toggle_pipe_state());
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        assert!(!state.overlay_projection().pipe_open);

        assert!(state.set_item_expanded("cam", true));
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        assert!(!state.view_pill_menu_open());

        assert!(state.toggle_build_state());
        assert!(state.toggle_menu_state("File"));
        assert_eq!(state.dismiss_top_layer_state(), OverlayDismissal::Closed);
        let overlay = state.overlay_projection();
        assert!(!overlay.build_open);
        assert!(!overlay.menu_open);
    }

    #[test]
    fn asset_pipeline_counts_group_all_processor_job_states() {
        let mut unclassified_running = asset_entry_with_job(AssetBrowserJobStatus::Leased);
        unclassified_running.source_path = "src/generated.rs".to_owned();
        unclassified_running.schema_type = None;
        let status = EditorAssetBrowserStatus::new(
            "session-1",
            Vec::new(),
            vec![
                asset_entry_with_job(AssetBrowserJobStatus::Queued),
                unclassified_running,
                asset_entry_with_job(AssetBrowserJobStatus::Failed),
                asset_entry_with_job(AssetBrowserJobStatus::Abandoned),
                asset_entry_with_job(AssetBrowserJobStatus::Succeeded),
                asset_entry_without_job(),
            ],
            None,
        );

        assert_eq!(
            asset_pipeline_counts(&status),
            AssetPipelineCounts {
                active: 2,
                failed: 2,
                succeeded: 1,
            }
        );
    }

    #[test]
    fn status_line_prioritizes_active_asset_pipeline_over_idle_session() {
        let session = editor_session_status();
        let asset_status = EditorAssetBrowserStatus::new(
            "session-1",
            Vec::new(),
            vec![
                asset_entry_with_job(AssetBrowserJobStatus::Leased),
                asset_entry_with_job(AssetBrowserJobStatus::Failed),
                asset_entry_with_job(AssetBrowserJobStatus::Succeeded),
            ],
            None,
        );

        assert_eq!(
            build_status_label(None, None, Some(&session), Some(&asset_status), None),
            "Assets"
        );
        assert_eq!(
            build_status_summary(None, None, Some(&session), Some(&asset_status), None),
            "1 active · 1 failed · 1 done"
        );
    }

    #[test]
    fn status_line_keeps_session_summary_when_asset_pipeline_is_idle() {
        let session = editor_session_status();
        let asset_status = EditorAssetBrowserStatus::new(
            "session-1",
            Vec::new(),
            vec![asset_entry_with_job(AssetBrowserJobStatus::Succeeded)],
            None,
        );

        assert_eq!(
            build_status_label(None, None, Some(&session), Some(&asset_status), None),
            "Session"
        );
        assert_eq!(
            build_status_summary(None, None, Some(&session), Some(&asset_status), None),
            session_status_summary(&session)
        );
        assert_eq!(asset_status_summary(&asset_status), "1 indexed");
    }

    #[test]
    fn status_line_uses_asset_processor_activity_before_idle_session() {
        let session = editor_session_status();
        let activity = EditorAssetProcessorActivity::new(
            "session-1",
            ServiceHealthStateData::Busy,
            "source-reconcile",
            "scanning projects/sample/assets; current prefabs/player.prefab.ron",
        );

        assert_eq!(
            build_status_label(None, None, Some(&session), None, Some(&activity)),
            "Assets"
        );
        assert_eq!(
            build_status_summary(None, None, Some(&session), None, Some(&activity)),
            "scanning projects/sample/assets; current prefabs/player.prefab.ron"
        );
    }

    #[test]
    fn status_line_uses_open_project_progress_before_session_or_asset_status() {
        let workflow = project_workflow::Status::running_with_progress(
            project_workflow::Operation::OpenEditorSession,
            "projects/sample",
            project_workflow::Progress {
                phase: project_workflow::OpenPhase::Build,
                phase_done: 21,
                phase_total: Some(50),
                message: "building project services".to_owned(),
            },
        );
        let session = editor_session_status();
        let asset_status = EditorAssetBrowserStatus::new(
            "session-1",
            Vec::new(),
            vec![asset_entry_with_job(AssetBrowserJobStatus::Succeeded)],
            None,
        );

        assert_eq!(
            build_status_label(
                Some(&workflow),
                None,
                Some(&session),
                Some(&asset_status),
                None,
            ),
            "Building"
        );
        assert_eq!(
            build_status_summary(
                Some(&workflow),
                None,
                Some(&session),
                Some(&asset_status),
                None,
            ),
            "42% · 21/50 · building project services"
        );
    }

    #[test]
    fn status_line_does_not_report_weighted_percent_for_unknown_build_total() {
        let workflow = project_workflow::Status::running_with_progress(
            project_workflow::Operation::OpenEditorSession,
            "projects/sample",
            project_workflow::Progress {
                phase: project_workflow::OpenPhase::Build,
                phase_done: 17,
                phase_total: None,
                message: "compiling az-editor".to_owned(),
            },
        );

        assert_eq!(
            build_status_summary(Some(&workflow), None, None, None, None),
            "live output · 17 unit(s) · compiling az-editor"
        );
    }

    #[test]
    fn status_line_uses_only_latest_output_log_event_after_workflow() {
        let mut output = OutputLogState::default();
        output.append_output(LogLevel::Info, "asset-processor", "job started: old.asset");
        output.append_output(LogLevel::Warn, "project-host:stderr", "Access is denied.");
        let session = editor_session_status();
        let asset_status = EditorAssetBrowserStatus::new(
            "session-1",
            Vec::new(),
            vec![asset_entry_with_job(AssetBrowserJobStatus::Leased)],
            None,
        );

        assert_eq!(
            build_status_label(
                None,
                Some(&output),
                Some(&session),
                Some(&asset_status),
                None
            ),
            "project-host"
        );
        assert_eq!(
            build_status_summary(
                None,
                Some(&output),
                Some(&session),
                Some(&asset_status),
                None
            ),
            "warn · Access is denied."
        );
    }

    #[test]
    fn status_line_reports_truthful_absent_project_state_without_render_fallback() {
        assert_eq!(build_status_label(None, None, None, None, None), "Project");
        assert_eq!(
            build_status_summary(None, None, None, None, None),
            "not attached"
        );
    }

    #[test]
    fn asset_browser_empty_message_is_content_state_not_asset_processor_progress() {
        assert_eq!(
            asset_browser_empty_message(false, ""),
            "No assets in this folder"
        );
        assert_eq!(
            asset_browser_empty_message(false, "  prefabs  "),
            "No assets match `prefabs`"
        );
        assert_eq!(
            asset_browser_empty_message(true, ""),
            "Connecting to project services..."
        );
    }

    #[test]
    fn asset_browser_ignores_unclassified_source_files() {
        let mut raw_source = asset_entry_without_job();
        raw_source.source_path = "src/player_controller.rs".to_owned();
        raw_source.schema_type = None;
        raw_source.latest_job = Some(AssetBrowserJobData {
            job_id: 1,
            attempt_id: Some(1),
            job_key: "compile".to_owned(),
            platform: "pc".to_owned(),
            ordinal: Some(1),
            status: AssetBrowserJobStatus::Leased,
            error_count: 0,
            warning_count: 0,
        });

        let mut script_asset = asset_entry_without_job();
        script_asset.entry_id = 2;
        script_asset.asset_guid = "00000000-0000-0000-0000-000000000002".to_owned();
        script_asset.source_path = "assets/scripts/open_chest.lua".to_owned();
        script_asset.schema_type = Some("az.script.Source".to_owned());
        script_asset.latest_job = Some(AssetBrowserJobData {
            job_id: 2,
            attempt_id: Some(2),
            job_key: "script".to_owned(),
            platform: "pc".to_owned(),
            ordinal: Some(1),
            status: AssetBrowserJobStatus::Succeeded,
            error_count: 0,
            warning_count: 0,
        });

        let status = EditorAssetBrowserStatus::new(
            "session-1",
            Vec::new(),
            vec![raw_source, script_asset],
            None,
        );

        let categories = asset_category_counts(&status);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].0.label, "Scripts");
        assert_eq!(categories[0].1, 1);

        assert_eq!(
            asset_pipeline_counts(&status),
            AssetPipelineCounts {
                active: 1,
                failed: 0,
                succeeded: 1,
            }
        );
    }

    #[test]
    fn asset_browser_classifies_prefab_pipeline_entries() {
        let mut prefab = asset_entry_without_job();
        prefab.source_path = "assets/prefabs/crate.prefab.ron".to_owned();
        prefab.schema_type = Some("az.prefab.Source".to_owned());

        let category = asset_entry_category(&prefab);
        assert_eq!(category.label, "Prefabs");

        let status = EditorAssetBrowserStatus::new("session-1", Vec::new(), vec![prefab], None);
        let categories = asset_category_counts(&status);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].0.label, "Prefabs");
        assert_eq!(categories[0].1, 1);
    }

    #[test]
    fn asset_browser_classifies_scene_documents_as_levels() {
        let mut scene = asset_entry_without_job();
        scene.source_path = "scenes/canyon.scene.ron".to_owned();
        scene.schema_type = Some("azoth.scene.Scene".to_owned());

        assert_eq!(asset_entry_category(&scene).label, "Levels");
    }

    #[test]
    fn asset_browser_rows_carry_source_root_for_crud_actions() {
        let mut entry = asset_entry_without_job();
        entry.source_path = "materials/metal.material.ron".to_owned();
        entry.schema_type = Some("azoth.material.MaterialSource".to_owned());

        let theme = gpui_component::theme::Theme::default();
        let row = asset_entry_item(&entry, true, &theme, "project:session-1:assets".to_owned());

        assert_eq!(row.src, "materials/metal.material.ron");
        assert_eq!(row.sub, "project:session-1:assets");
        assert_eq!(asset_item_source_root(&row), "project:session-1:assets");
        assert!(row.selected);
    }

    #[test]
    fn authored_source_move_remaps_active_level_label_path() {
        let mut state = AetherEditorState::new();
        state.record_authored_source_path_move(
            "levels/old.level.ron".to_owned(),
            "levels/new.level.ron".to_owned(),
        );

        assert_eq!(
            state.remapped_authored_source_path("levels/old.level.ron"),
            "levels/new.level.ron"
        );
        assert_eq!(
            state.remapped_authored_source_path("levels/other.level.ron"),
            "levels/other.level.ron"
        );
    }

    #[test]
    fn authored_content_state_projects_expansion_and_redirects_without_outline_ownership() {
        let mut state = AetherEditorState::new();

        assert!(state.set_item_expanded("levels/old.level.ron", false));
        state.record_authored_source_path_move(
            "levels/old.level.ron".to_owned(),
            "levels/new.level.ron".to_owned(),
        );

        assert!(!state.item_expanded("levels/old.level.ron", true));
        assert_eq!(
            state.remapped_authored_source_path("levels/old.level.ron"),
            "levels/new.level.ron"
        );
        assert!(state.item_expanded("levels/other.level.ron", true));
    }

    #[test]
    fn authored_source_move_remaps_clear_after_outline_refresh_and_collapse_chains() {
        let mut state = AetherEditorState::new();
        state.record_authored_source_path_move(
            "levels/old.level.ron".to_owned(),
            "levels/mid.level.ron".to_owned(),
        );
        state.record_authored_source_path_move(
            "levels/mid.level.ron".to_owned(),
            "levels/new.level.ron".to_owned(),
        );

        assert_eq!(
            state.remapped_authored_source_path("levels/old.level.ron"),
            "levels/new.level.ron"
        );

        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![authored_document(
                "levels/new.level.ron",
                "levels/new.level.ron",
                "azoth.prefab.Prefab",
            )],
        });

        assert!(state.clear_resolved_authored_source_path_moves(&outline));
        assert_eq!(
            state.remapped_authored_source_path("levels/old.level.ron"),
            "levels/old.level.ron"
        );
    }

    #[test]
    fn layer_rows_project_authored_documents_with_visibility_state() {
        let mut hidden = std::collections::BTreeSet::new();
        hidden.insert("scenes/secondary.scene.ron".to_owned());
        let mut locked = std::collections::BTreeSet::new();
        locked.insert("scenes/main.scene.ron".to_owned());
        let visibility = EditorLayerVisibility { hidden, locked };
        let mut main = authored_document(
            "scenes/main.scene.ron",
            "scenes/main.scene.ron",
            az_prefab::SCENE_SOURCE_TYPE,
        );
        main.unsaved_changes = true;
        main.object_count = 3;
        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![
                main,
                authored_document(
                    "scenes/secondary.scene.ron",
                    "scenes/secondary.scene.ron",
                    az_prefab::SCENE_SOURCE_TYPE,
                ),
                authored_document(
                    "gamedata/ability.ron",
                    "gamedata/ability.ron",
                    "azoth.gamedata.TableSource",
                ),
            ],
        });

        let rows = authored_layer_rows(&outline, Some(&visibility));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "main.scene");
        assert_eq!(rows[0].count, 3);
        assert!(rows[0].unsaved);
        assert!(rows[0].locked);
        assert!(!rows[0].hidden);
        assert_eq!(rows[1].name, "secondary.scene");
        assert!(rows[1].hidden);
        assert!(!rows[1].locked);
        assert!(
            rows.iter()
                .all(|row| row.document_id != "gamedata/ability.ron")
        );
    }

    #[test]
    fn hierarchy_projection_filters_game_data_documents_from_scene() {
        let mut scene = authored_document(
            "scenes/main.scene.ron",
            "scenes/main.scene.ron",
            "azoth.scene.Scene",
        );
        scene.objects[0].prefab_source_path = Some("prefabs/main.prefab.ron".to_owned());
        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![
                scene,
                authored_document(
                    "prefabs/main.prefab.ron",
                    "prefabs/main.prefab.ron",
                    "azoth.prefab.Prefab",
                ),
                authored_document(
                    "gamedata/ability.ron",
                    "gamedata/ability.ron",
                    "azoth.gamedata.TableSource",
                ),
            ],
        });

        let rows = hierarchy_rows_from_outline_state(
            &AetherEditorState::new(),
            &outline,
            Some("scenes/main.scene.ron"),
        );
        let keys = rows.iter().map(|row| row.key.as_str()).collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec!["scenes/main.scene.ron", "prefabs/main.prefab.ron:root"]
        );
        assert_eq!(scene_document_counts(&outline), (1, 1));
    }

    #[test]
    fn level_projection_filters_prefab_documents_and_real_actions() {
        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![
                authored_document(
                    "scenes/main.scene.ron",
                    "scenes/main.scene.ron",
                    "azoth.scene.Scene",
                ),
                authored_document(
                    "gamedata/ability.ron",
                    "gamedata/ability.ron",
                    "azoth.gamedata.TableSource",
                ),
                authored_document(
                    "scenes/ui.scene.ron",
                    "scenes/ui.scene.ron",
                    "azoth.scene.Scene",
                ),
                authored_document(
                    "prefabs/standalone.prefab.ron",
                    "prefabs/standalone.prefab.ron",
                    "azoth.prefab.Prefab",
                ),
            ],
        });

        let levels = level_documents(&outline);

        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].document_id, "scenes/main.scene.ron");
        assert_eq!(
            level_document_meta(levels[0]),
            "scenes/main.scene.ron · 1 object"
        );

        let creatable = EditorCreatableAuthoredSchemas::new(vec![CreatableAuthoredSchemaData {
            schema_type: "azoth.scene.Scene".to_owned(),
            label: "Scene".to_owned(),
            category: Some("Levels".to_owned()),
            icon: None,
            component_capabilities: None,
        }]);
        let actions = level_action_items(Some(&creatable), true);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.key.as_str())
                .collect::<Vec<_>>(),
            vec!["new-level", "save-level", "refresh-levels"]
        );

        let actions = level_action_items(None, false);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].key, "refresh-levels");
    }

    #[test]
    fn unloaded_invalid_level_draft_remains_selectable_for_repair() {
        let mut invalid = authored_document(
            "scenes/draft.scene.ron",
            "scenes/draft.scene.ron",
            "azoth.scene.Scene",
        );
        invalid.valid = false;
        invalid.loaded = false;
        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![invalid],
        });
        let theme = gpui_component::theme::Theme::default();

        let levels = level_items_from_outline(&outline, None, &theme);

        assert_eq!(levels.len(), 1);
        assert!(!levels[0].disabled);
        assert!(!levels[0].active);
    }

    #[test]
    fn active_layer_resolution_prefers_inspection_over_outline_selection() {
        let mut selected = authored_document(
            "prefabs/selected.prefab.ron",
            "prefabs/selected.prefab.ron",
            "azoth.prefab.Prefab",
        );
        selected.objects[0].selected = true;
        let outline = EditorAuthoredOutline::new(AuthoredOutlineData {
            documents: vec![
                selected,
                authored_document(
                    "prefabs/inspected.prefab.ron",
                    "prefabs/inspected.prefab.ron",
                    "azoth.prefab.Prefab",
                ),
            ],
        });

        assert_eq!(
            active_authored_document_id(None, &outline).as_deref(),
            Some("prefabs/selected.prefab.ron")
        );

        let inspection = az_editor_inspector::ReflectedEntityInspection {
            selection: az_editor_inspector::ReflectedPrefabSelection::new(
                "prefabs/inspected.prefab.ron",
                "root",
            ),
            registry_schema_catalog_hash: vec![1; 32],
            document_version: 1,
            type_versions: std::collections::BTreeMap::new(),
            revision: 1,
            components: Vec::new(),
            overrides: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(
            active_authored_document_id(Some(&inspection), &outline).as_deref(),
            Some("prefabs/inspected.prefab.ron")
        );
    }

    #[test]
    fn session_status_summary_separates_attached_services_from_supervised_processes() {
        let status = EditorSessionStatus {
            session_id: "session-1".to_owned(),
            project_id: "sample-game".to_owned(),
            session_slug: "sample-main".to_owned(),
            project_root: "projects\\sample".to_owned(),
            workspace_root: "projects\\sample".to_owned(),
            run_dir: "projects\\sample\\.az".to_owned(),
            state: EditorSessionStateData::Active,
            failure_reason: None,
            services_count: 3,
            processes: vec![session_process(
                "runtime-host",
                SessionProcessStateData::Failed,
            )],
        };

        assert_eq!(
            session_status_summary(&status),
            "active · 3 services attached · 0/1 supervised · 1 failed"
        );
    }

    #[test]
    fn command_dock_toggles_restore_the_selected_panel() {
        let mut state = AetherEditorState::new();

        assert!(state.toggle_left_dock_state("hierarchy"));
        assert!(!state.workspace_projection().show_left);
        assert!(state.toggle_left_dock_state("hierarchy"));
        assert!(state.workspace_projection().show_left);
        assert_eq!(state.workspace_projection().left_tab, "hierarchy");

        assert!(state.toggle_bottom_dock_state("assets"));
        assert!(!state.workspace_projection().show_bottom);
        assert!(state.toggle_bottom_dock_state("console"));
        assert!(state.workspace_projection().show_bottom);
        assert_eq!(state.workspace_projection().bottom_tab, "console");
    }

    #[test]
    fn dock_commands_cannot_open_an_unsupported_workspace_placement() {
        let mut state = AetherEditorState::new();
        assert!(state.set_mode_state("materials"));
        let before = state.workspace_projection();

        assert!(!state.select_bottom_tab_state("console"));
        assert!(!state.toggle_bottom_dock_state("console"));
        state.toggle_bottom_panel_state();

        let after = state.workspace_projection();
        assert_eq!(after.bottom_tab, before.bottom_tab);
        assert!(!after.show_bottom);
    }

    #[test]
    fn workspace_commands_reject_unknown_modes_and_tabs() {
        let mut state = AetherEditorState::new();
        let before = state.workspace_projection();

        assert!(!state.set_mode_state("unknown"));
        assert!(!state.select_left_tab_state("unknown"));
        assert!(!state.select_right_tab_state("unknown"));
        assert!(!state.select_view_tab_state("unknown"));
        assert!(!state.select_bottom_tab_state("unknown"));

        assert_eq!(state.workspace_projection(), before);
    }

    #[test]
    fn overlay_commands_reject_unknown_menu_and_modal_literals() {
        let mut state = AetherEditorState::new();
        let closed = state.overlay_projection();

        assert!(!state.toggle_menu_state("Unknown"));
        state.open_overlay_modal_state("unknown");
        assert_eq!(state.overlay_projection(), closed);

        assert!(state.toggle_menu_state("File"));
        let file_open = state.overlay_projection();
        assert!(!state.switch_open_menu_from_title_hover_state("Unknown"));
        let before_invalid_aim = state.trace_summary();
        state.schedule_menu_aim_state("Unknown");
        assert_eq!(state.overlay_projection(), file_open);
        assert_eq!(state.trace_summary(), before_invalid_aim);
    }

    #[test]
    fn graph_workspace_action_targets_scripting_mode() {
        let mut state = AetherEditorState::new();

        assert!(state.set_mode_state("scripting"));
        assert_eq!(state.workspace_projection().mode, "scripting");
    }

    #[test]
    fn mode_changes_resync_status_bar_docks_to_the_layout_profile() {
        let mut state = AetherEditorState::new();

        for mode in [
            "materials",
            "scripting",
            "gamedata",
            "sequencer",
            "animation",
            "profiler",
            "scene",
        ] {
            state.toggle_left_panel_state();
            state.toggle_right_panel_state();
            state.toggle_bottom_panel_state();

            assert!(state.set_mode_state(mode));
            let profile = crate::workspace::layout::layout_profile_for_mode(mode);
            let workspace = state.workspace_projection();
            assert_eq!(
                workspace.show_left,
                profile
                    .left
                    .iter()
                    .any(|placement| placement.visibility.is_open()),
                "{mode} left dock"
            );
            assert_eq!(
                workspace.show_right,
                profile
                    .right
                    .iter()
                    .any(|placement| placement.visibility.is_open()),
                "{mode} right dock"
            );
            assert_eq!(
                workspace.show_bottom,
                profile
                    .bottom
                    .iter()
                    .any(|placement| placement.visibility.is_open()),
                "{mode} bottom dock"
            );
        }
    }

    #[test]
    fn mode_changes_project_dock_capabilities_from_the_layout_profile() {
        let mut state = AetherEditorState::new();

        for mode in [
            "materials",
            "scripting",
            "gamedata",
            "sequencer",
            "animation",
            "profiler",
            "scene",
        ] {
            assert!(state.set_mode_state(mode));

            let profile = crate::workspace::layout::layout_profile_for_mode(mode);
            let capabilities = state.panel_capabilities();
            assert_eq!(capabilities.left, !profile.left.is_empty(), "{mode} left");
            assert_eq!(
                capabilities.right,
                !profile.right.is_empty(),
                "{mode} right"
            );
            assert_eq!(
                capabilities.bottom,
                !profile.bottom.is_empty(),
                "{mode} bottom"
            );
        }
    }

    #[test]
    fn game_data_mode_state_targets_game_data_mode() {
        let mut state = AetherEditorState::new();

        assert!(state.set_mode_state("gamedata"));
        assert_eq!(state.workspace_projection().mode, "gamedata");
        assert!(!state.set_mode_state("gamedata"));
    }

    #[test]
    fn transport_play_icon_follows_runtime_state() {
        assert_eq!(runtime_play_icon(None), "play_arrow");

        let mut status = runtime_status(EditorRuntimeStateData::Stopped);
        assert_eq!(runtime_play_icon(Some(&status)), "play_arrow");

        status.state = EditorRuntimeStateData::Failed;
        assert_eq!(runtime_play_icon(Some(&status)), "play_arrow");

        status.state = EditorRuntimeStateData::Running;
        assert_eq!(runtime_play_icon(Some(&status)), "pause");

        status.state = EditorRuntimeStateData::Starting;
        assert_eq!(runtime_play_icon(Some(&status)), "pause");
    }

    #[test]
    fn game_data_view_buttons_select_matching_detail_tabs() {
        let mut state = AetherEditorState::new();
        assert_eq!(state.game_data_rail().view, "tables");
        assert_eq!(state.game_data_rail().tab, "table");
        assert!(matches!(
            state.game_data_table_selection(),
            AetherGameDataTableSelection::Empty
        ));
        assert_eq!(state.game_data_schema_selection().key, "");
        assert_eq!(state.game_data_table_selection().row_count(), None);
        assert!(state.set_game_data_view_state("schemas"));
        assert_eq!(state.game_data_rail().view, "schemas");
        assert_eq!(state.game_data_rail().tab, "schema");
        assert!(!state.set_game_data_view_state("schemas"));

        assert!(state.set_game_data_view_state("managers"));
        assert_eq!(state.game_data_rail().view, "managers");
        assert_eq!(state.game_data_rail().tab, "manager");

        assert!(state.set_game_data_tab_state("field"));
        assert_eq!(state.game_data_rail().view, "tables");
        assert_eq!(state.game_data_rail().tab, "field");
        assert!(!state.set_game_data_tab_state("field"));
    }

    #[test]
    fn game_data_selection_projects_local_table_affordance_without_authoring_state() {
        let mut state = AetherEditorState::new();
        let table = GameDataTableDescriptor {
            name: "Items".to_owned(),
            row_type: "sample::ItemRow".to_owned(),
            source_root: "gamedata".to_owned(),
            source_path: "items.ron".to_owned(),
            owner: "sample".to_owned(),
            schema_hash: None,
            document_id: "gamedata/items.ron".to_owned(),
            schema_type: "sample::ItemRow".to_owned(),
            category: "Items".to_owned(),
            row_count: Some(2),
            families: Vec::new(),
            source_ref: az_proto_asset::WorkspaceSourceFileRef {
                source_root_key: "project:sample:assets".to_owned(),
                source_path: "gamedata/items.ron".to_owned(),
                schema_type: "azoth.gamedata.TableSource".to_owned(),
            },
        };

        assert!(state.set_game_data_search_state("iron"));
        assert!(state.select_gamedata_table_state(&table));
        let rail = state.game_data_rail();
        let selection = state.game_data_table_selection();
        assert_eq!(rail.view, "tables");
        assert_eq!(rail.tab, "table");
        assert_eq!(rail.search, "iron");
        assert_eq!(selection.table_key(), "Items");
        assert_eq!(selection.name(), "Items");
        assert_eq!(selection.category(), "Items");
        assert_eq!(
            selection.row_count().expect("selected table count").label(),
            "2"
        );
        assert_eq!(state.game_data_schema_selection().key, "sample::ItemRow");
        assert!(!state.select_gamedata_table_state(&table));

        let mut loading_table = table.clone();
        loading_table.row_count = None;
        assert!(state.select_gamedata_table_state(&loading_table));
        assert_eq!(
            state
                .game_data_table_selection()
                .row_count()
                .expect("selected table count")
                .label(),
            "loading"
        );

        assert!(state.select_gamedata_schema_state("sample::OtherRow"));
        assert_eq!(state.game_data_rail().view, "schemas");
        assert_eq!(state.game_data_schema_selection().key, "sample::OtherRow");
    }

    #[test]
    fn game_data_schema_focus_is_valid_without_a_table_selection() {
        let mut state = AetherEditorState::new();

        assert!(state.select_gamedata_schema_state("sample::ItemRow"));
        assert_eq!(state.game_data_rail().view, "schemas");
        assert_eq!(state.game_data_schema_selection().key, "sample::ItemRow");
        assert!(matches!(
            state.game_data_table_selection(),
            AetherGameDataTableSelection::Empty
        ));
    }

    #[test]
    fn row_expansion_state_persists_by_item_key() {
        let mut state = AetherEditorState::new();

        assert!(state.item_expanded("prefabs/main.prefab.ron", true));
        assert!(state.set_item_expanded("prefabs/main.prefab.ron", false));
        assert!(!state.item_expanded("prefabs/main.prefab.ron", true));
        assert!(state.set_item_expanded("prefabs/main.prefab.ron", true));
        assert!(state.item_expanded("prefabs/main.prefab.ron", true));
        assert!(!state.set_item_expanded("", false));
    }

    #[test]
    fn viewport_pill_expansion_does_not_enter_authored_content_state() {
        let mut state = AetherEditorState::new();

        assert!(state.set_item_expanded("cam", true));
        assert!(state.view_pill_menu_open());
        assert!(state.set_item_expanded("prefabs/main.prefab.ron", false));

        state.close_view_pill_menus_state();

        assert!(!state.view_pill_menu_open());
        assert!(!state.item_expanded("prefabs/main.prefab.ron", true));
    }

    #[test]
    fn asset_folder_history_tracks_back_and_forward_transitions() {
        let mut state = AetherEditorState::new();

        assert!(state.select_asset_folder_state("Materials"));
        assert_eq!(state.asset_browser_navigation().folder, "Materials");
        assert!(state.select_asset_folder_state(""));
        assert_eq!(state.asset_browser_navigation().folder, "");
        assert!(state.navigate_asset_back_state());
        assert_eq!(state.asset_browser_navigation().folder, "Materials");
        assert!(state.navigate_asset_forward_state());
        assert_eq!(state.asset_browser_navigation().folder, "");
        assert!(!state.navigate_asset_forward_state());
    }

    #[test]
    fn asset_browser_drafts_keep_validation_errors_with_their_named_transition() {
        let mut state = AetherEditorState::new();

        state.begin_asset_create_state(
            "azoth.prefab.Prefab".to_owned(),
            "prefabs".to_owned(),
            None,
        );
        assert!(state.edit_asset_create_name_state("crate"));
        state.reject_asset_create_state("name already exists".to_owned());
        assert_eq!(state.asset_create_draft().error, "name already exists");
        assert!(state.edit_asset_create_folder_state("prefabs/props"));
        assert!(state.asset_create_draft().error.is_empty());

        state.begin_asset_rename_state(
            "project:example:assets".to_owned(),
            "prefabs/crate.prefab.ron".to_owned(),
        );
        state.reject_asset_rename_state("path is not portable".to_owned());
        assert!(state.edit_asset_rename_target_state("prefabs/prop.prefab.ron"));
        assert!(state.asset_rename_draft().error.is_empty());

        state.begin_asset_delete_state(
            "project:example:assets".to_owned(),
            "prefabs/prop.prefab.ron".to_owned(),
        );
        state.reject_asset_delete_state("dependent scan is pending".to_owned());
        assert_eq!(
            state.asset_delete_draft().error,
            "dependent scan is pending"
        );
        state.clear_asset_modal_errors();
        assert!(state.asset_create_draft().error.is_empty());
        assert!(state.asset_rename_draft().error.is_empty());
        assert!(state.asset_delete_draft().error.is_empty());
    }

    #[test]
    fn asset_browser_selection_rename_delete_and_redirect_transitions_are_cohesive() {
        let mut state = AetherEditorState::new();
        let item = AetherItem {
            key: "14".to_owned(),
            src: "prefabs/crate.prefab.ron".to_owned(),
            type_label: "azoth.prefab.Prefab".to_owned(),
            name: "crate".to_owned(),
            icon: "description".to_owned(),
            color: "#ffffff".to_owned(),
            idx: "14".to_owned(),
            ..AetherItem::default()
        };
        state.select_asset_state(&item);
        assert_eq!(state.asset_selection().source_path, item.src);

        assert!(
            state.commit_asset_rename_state("prefabs/crate.prefab.ron", "prefabs/prop.prefab.ron")
        );
        state.record_authored_source_path_move(
            "prefabs/crate.prefab.ron".to_owned(),
            "prefabs/prop.prefab.ron".to_owned(),
        );
        assert_eq!(
            state.asset_selection().source_path,
            "prefabs/prop.prefab.ron"
        );
        assert_eq!(
            state.remapped_authored_source_path("prefabs/crate.prefab.ron"),
            "prefabs/prop.prefab.ron"
        );
        assert!(state.commit_asset_delete_state("prefabs/prop.prefab.ron"));
        assert!(state.asset_selection().source_path.is_empty());
    }

    #[test]
    fn asset_search_filters_current_folder_case_insensitively() {
        let status = EditorAssetBrowserStatus::new(
            "session",
            vec![WorkspaceRootData {
                workspace_root_id: 1,
                root_id: 1,
                declared_root_id: "project.assets".to_owned(),
                owner_id: "local.project".to_owned(),
                source_root: "/wt/project/assets".to_owned(),
                display_name: "Project Assets".to_owned(),
                portable_key: "project:local.project:assets".to_owned(),
                output_prefix: "assets".to_owned(),
            }],
            vec![
                AssetBrowserEntryData {
                    entry_id: 1,
                    workspace_id: 1,
                    asset_guid: "00000000-0000-0000-0000-000000000001".to_owned(),
                    root_id: 1,
                    source_path: "materials/Metal.MATERIAL.ron".to_owned(),
                    schema_type: Some("azoth.material.MaterialSource".to_owned()),
                    content_hash: "a".repeat(64),
                    status: AssetBrowserEntryStatus::Clean,
                    diagnostics_count: 0,
                    latest_job: None,
                },
                AssetBrowserEntryData {
                    entry_id: 2,
                    workspace_id: 1,
                    asset_guid: "00000000-0000-0000-0000-000000000002".to_owned(),
                    root_id: 1,
                    source_path: "textures/metal.png".to_owned(),
                    schema_type: Some("azoth.texture.TextureSource".to_owned()),
                    content_hash: "b".repeat(64),
                    status: AssetBrowserEntryStatus::Clean,
                    diagnostics_count: 0,
                    latest_job: None,
                },
            ],
            None,
        );
        let folders = asset_browser_folders(&status);
        let materials = folders
            .iter()
            .find(|folder| folder.name == "Materials")
            .expect("Materials category");
        let textures = folders
            .iter()
            .find(|folder| folder.name == "Textures")
            .expect("Textures category");

        let matches = filtered_asset_entries(&status, Some(materials), "material")
            .map(|entry| entry.source_path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(matches, vec!["materials/Metal.MATERIAL.ron"]);
        assert!(
            filtered_asset_entries(&status, Some(textures), "material")
                .next()
                .is_none()
        );
    }

    #[test]
    fn asset_folder_projection_groups_types_across_project_and_gem_roots() {
        let status = EditorAssetBrowserStatus::new(
            "session",
            vec![
                WorkspaceRootData {
                    workspace_root_id: 1,
                    root_id: 10,
                    declared_root_id: "project.assets".to_owned(),
                    owner_id: "local.project".to_owned(),
                    source_root: "/wt/project/assets".to_owned(),
                    display_name: "Project Assets".to_owned(),
                    portable_key: "project:local.project:assets".to_owned(),
                    output_prefix: "assets".to_owned(),
                },
                WorkspaceRootData {
                    workspace_root_id: 2,
                    root_id: 11,
                    declared_root_id: "gem.azoth.physics.assets".to_owned(),
                    owner_id: "azoth.physics".to_owned(),
                    source_root: "/wt/project/gems/physics/assets".to_owned(),
                    display_name: "Physics Assets".to_owned(),
                    portable_key: "gem:azoth.physics:assets".to_owned(),
                    output_prefix: "gems/azoth.physics".to_owned(),
                },
            ],
            vec![
                asset_entry_for_folder(
                    1,
                    10,
                    "materials/wood.material.ron",
                    "azoth.material.Material",
                ),
                asset_entry_for_folder(2, 10, "prefabs/player.prefab.ron", "azoth.prefab.Prefab"),
                asset_entry_for_folder(
                    3,
                    11,
                    "materials/metal.material.ron",
                    "azoth.material.Material",
                ),
                asset_entry_for_folder(4, 11, "textures/metal.dds", "azoth.texture.SourceImage"),
            ],
            None,
        );

        let folders = asset_browser_folders(&status);

        assert_eq!(
            folders
                .iter()
                .map(|folder| (folder.name.as_str(), folder.count))
                .collect::<Vec<_>>(),
            vec![("Materials", 2), ("Textures", 1), ("Prefabs", 1)]
        );
        assert_eq!(folders[0].breadcrumb(), "Materials");
        assert_eq!(asset_folder_category(&status, &folders[2]).label, "Prefabs");
        assert_eq!(
            asset_entries_for_folder(&status, Some(&folders[0]))
                .map(|entry| entry.source_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "materials/wood.material.ron",
                "materials/metal.material.ron",
            ]
        );
    }

    fn asset_entry_with_job(status: AssetBrowserJobStatus) -> AssetBrowserEntryData {
        let mut entry = asset_entry_without_job();
        entry.latest_job = Some(AssetBrowserJobData {
            job_id: 1,
            attempt_id: Some(1),
            job_key: "build".to_owned(),
            platform: "pc".to_owned(),
            ordinal: Some(1),
            status,
            error_count: 0,
            warning_count: 0,
        });
        entry
    }

    fn asset_entry_for_folder(
        entry_id: i64,
        root_id: i64,
        source_path: &str,
        schema_type: &str,
    ) -> AssetBrowserEntryData {
        AssetBrowserEntryData {
            entry_id,
            workspace_id: 1,
            asset_guid: format!("00000000-0000-0000-0000-{entry_id:012}"),
            root_id,
            source_path: source_path.to_owned(),
            schema_type: Some(schema_type.to_owned()),
            content_hash: "a".repeat(64),
            status: AssetBrowserEntryStatus::Clean,
            diagnostics_count: 0,
            latest_job: None,
        }
    }

    fn asset_entry_without_job() -> AssetBrowserEntryData {
        AssetBrowserEntryData {
            entry_id: 1,
            workspace_id: 1,
            asset_guid: "00000000-0000-0000-0000-000000000001".to_owned(),
            root_id: 1,
            source_path: "assets/test.mesh".to_owned(),
            schema_type: Some("az.mesh.Source".to_owned()),
            content_hash: "hash".to_owned(),
            status: AssetBrowserEntryStatus::Clean,
            diagnostics_count: 0,
            latest_job: None,
        }
    }

    fn authored_document(
        document_id: &str,
        source_path: &str,
        schema_type: &str,
    ) -> AuthoredDocumentOutlineData {
        AuthoredDocumentOutlineData {
            document_id: document_id.to_owned(),
            source_path: source_path.to_owned(),
            schema_type: schema_type.to_owned(),
            revision: 1,
            saved_revision: Some(1),
            unsaved_changes: false,
            object_count: 1,
            journal_entry_count: 0,
            loaded: true,
            valid: true,
            diagnostic: String::new(),
            objects: vec![AuthoredObjectOutlineData {
                object_id: "root".to_owned(),
                schema_type: schema_type.to_owned(),
                selected: false,
                display_name: None,
                prefab_parent_entity_object_id: None,
                prefab_component_object_ids: Vec::new(),
                prefab_owner_entity_object_id: None,
                prefab_source_path: None,
            }],
        }
    }

    fn runtime_status(state: EditorRuntimeStateData) -> EditorRuntimeStatus {
        EditorRuntimeStatus {
            runtime_id: "editor-world".to_owned(),
            state,
            role: None,
            project_id: None,
            session_slug: None,
            authored_revision: None,
            diagnostics: Vec::new(),
        }
    }

    fn session_process(service_name: &str, state: SessionProcessStateData) -> SessionProcessData {
        SessionProcessData {
            owner_id: "owner".to_owned(),
            owner_root: "projects\\sample".to_owned(),
            service_name: service_name.to_owned(),
            role: SessionServiceRoleData::Worker,
            run: uuid::Uuid::from_bytes([1; 16]),
            state,
            pid: None,
            exit_code: None,
            failure: None,
            structured_log: String::new(),
        }
    }

    fn editor_session_status() -> EditorSessionStatus {
        EditorSessionStatus {
            session_id: "session-1".to_owned(),
            project_id: "sample-game".to_owned(),
            session_slug: "sample-main".to_owned(),
            project_root: "projects\\sample".to_owned(),
            workspace_root: "projects\\sample".to_owned(),
            run_dir: "projects\\sample\\.az".to_owned(),
            state: EditorSessionStateData::Active,
            failure_reason: None,
            services_count: 3,
            processes: vec![
                session_process("project-host", SessionProcessStateData::Running),
                session_process("asset-processor", SessionProcessStateData::Running),
                session_process("runtime-host", SessionProcessStateData::Failed),
            ],
        }
    }
}
