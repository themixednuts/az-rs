//! Asset Browser
//!
//! Attached workspace snapshot with entries, roots, builder
//! descriptors, and filtering supplied by the editor shell.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Global, Hsla, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Render,
    StatefulInteractiveElement, Styled, StyledImage, Subscription, Window, div, img,
    prelude::FluentBuilder, px,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::{
    ActiveTheme, Icon, IconName, InteractiveElementExt as _, Sizable, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::panels::ViewportAssetDrag;
use crate::panels::kit;
use crate::status::ServiceHealthStateData;
use crate::type_iconography::{ASSET_CATEGORY_KINDS, EditorTypeKind, asset_kind};

/// Asset status snapshot supplied by the editor shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAssetBrowserStatus {
    pub session_id: String,
    pub roots: Vec<WorkspaceRootData>,
    pub entries: Vec<AssetBrowserEntryData>,
    pub next_after_entry_id: Option<i64>,
    pub status_error: Option<String>,
}

impl EditorAssetBrowserStatus {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        roots: Vec<WorkspaceRootData>,
        entries: Vec<AssetBrowserEntryData>,
        next_after_entry_id: Option<i64>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            roots,
            entries,
            next_after_entry_id,
            status_error: None,
        }
    }

    #[must_use]
    pub fn error(session_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            roots: Vec::new(),
            entries: Vec::new(),
            next_after_entry_id: None,
            status_error: Some(error.into()),
        }
    }

    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.status_error = Some(error.into());
        self
    }
}

impl Global for EditorAssetBrowserStatus {}

/// Asset-builder catalog supplied by the project-owned asset processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAssetBuilderCatalog {
    pub builders: Vec<AssetBuilderData>,
    pub source_schemas: Vec<AssetSourceSchemaData>,
}

impl EditorAssetBuilderCatalog {
    #[must_use]
    pub const fn new(
        builders: Vec<AssetBuilderData>,
        source_schemas: Vec<AssetSourceSchemaData>,
    ) -> Self {
        Self {
            builders,
            source_schemas,
        }
    }
}

impl Global for EditorAssetBuilderCatalog {}

/// Health/activity snapshot supplied by the project-owned asset processor.
///
/// `degraded` and `ready` are reported by the service alongside its state and
/// can disagree with it — a degraded processor that cannot serve at all reports
/// `ready: false`. Busyness carries no such independent signal and is read off
/// the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAssetProcessorActivity {
    pub session_id: String,
    pub state: ServiceHealthStateData,
    pub operation: String,
    pub message: String,
    pub checked_unix_ms: u64,
    pub uptime_ms: u64,
    pub last_event_seq: u64,
    pub degraded: bool,
    pub ready: bool,
}

impl EditorAssetProcessorActivity {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        state: ServiceHealthStateData,
        operation: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            state,
            operation: operation.into(),
            message: message.into(),
            checked_unix_ms: 0,
            uptime_ms: 0,
            last_event_seq: 0,
            degraded: state.degraded(),
            ready: state.ready(),
        }
    }

    #[must_use]
    pub const fn busy(&self) -> bool {
        self.state.busy()
    }

    #[must_use]
    pub const fn with_transport(
        mut self,
        checked_unix_ms: u64,
        uptime_ms: u64,
        last_event_seq: u64,
    ) -> Self {
        self.checked_unix_ms = checked_unix_ms;
        self.uptime_ms = uptime_ms;
        self.last_event_seq = last_event_seq;
        self
    }

    #[must_use]
    pub fn unavailable(session_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            state: ServiceHealthStateData::Degraded,
            operation: "health".to_owned(),
            message: message.into(),
            checked_unix_ms: 0,
            uptime_ms: 0,
            last_event_seq: 0,
            degraded: true,
            ready: false,
        }
    }
}

impl Global for EditorAssetProcessorActivity {}

/// Catalog products supplied by the project-owned asset processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCatalogProductsStatus {
    pub session_id: String,
    pub platform: String,
    pub entries: Vec<CatalogProductData>,
    pub status_error: Option<String>,
}

impl EditorCatalogProductsStatus {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        platform: impl Into<String>,
        entries: Vec<CatalogProductData>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            platform: platform.into(),
            entries,
            status_error: None,
        }
    }

    #[must_use]
    pub fn error(
        session_id: impl Into<String>,
        platform: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            platform: platform.into(),
            entries: Vec::new(),
            status_error: Some(error.into()),
        }
    }
}

impl Global for EditorCatalogProductsStatus {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProductData {
    pub job_id: i64,
    pub product_id: i64,
    pub asset_guid: String,
    pub source_path: String,
    pub builder_guid: String,
    pub job_key: String,
    pub platform: String,
    pub product_path: String,
    pub asset_type: String,
    pub sub_id: i64,
    pub product_format: String,
    pub product_format_version: u32,
    pub content_hash: String,
    pub byte_length: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAssetSourceDependentsPreview {
    pub session_id: String,
    pub source_root: String,
    pub source_path: String,
    pub source_dependents: Vec<AssetSourceDependentSourceData>,
    pub job_dependents: Vec<AssetSourceDependentJobData>,
    pub loading: bool,
    pub error: Option<String>,
}

impl EditorAssetSourceDependentsPreview {
    #[must_use]
    pub fn loading(
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            source_root: source_root.into(),
            source_path: source_path.into(),
            source_dependents: Vec::new(),
            job_dependents: Vec::new(),
            loading: true,
            error: None,
        }
    }

    #[must_use]
    pub fn failed(
        session_id: impl Into<String>,
        source_root: impl Into<String>,
        source_path: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            source_root: source_root.into(),
            source_path: source_path.into(),
            source_dependents: Vec::new(),
            job_dependents: Vec::new(),
            loading: false,
            error: Some(error.into()),
        }
    }

    #[must_use]
    pub const fn total_dependents(&self) -> usize {
        self.source_dependents.len() + self.job_dependents.len()
    }
}

impl Global for EditorAssetSourceDependentsPreview {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSourceDependentSourceData {
    pub source_path: String,
    pub builder_guid: String,
    pub relation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSourceDependentJobData {
    pub job_edge_id: i64,
    pub job_id: i64,
    pub latest_attempt_id: Option<i64>,
    pub source_path: String,
    pub owner: String,
    pub job_key: String,
    pub platform: String,
    pub dependency_job_key: String,
    pub dependency_platform: String,
    pub dependency_kind: i64,
    pub product_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBuilderData {
    pub name: String,
    pub builder_guid: String,
    pub version: u32,
    pub patterns: Vec<AssetBuilderPatternData>,
    pub source_schema_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBuilderPatternData {
    pub kind: AssetBuilderPatternKindData,
    pub pattern: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetBuilderPatternKindData {
    Wildcard,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSourceSchemaData {
    pub schema_type: String,
    pub owner: String,
    pub label: String,
    pub category: String,
    pub authoring: AssetSourceSchemaAuthoringData,
    pub file_templates: Vec<AssetSourceFileTemplateData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSourceFileTemplateData {
    pub owner: String,
    pub source_path: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetSourceSchemaAuthoringData {
    File {
        workflow: AssetSourceFileWorkflowData,
    },
    ProjectDocument {
        schema_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSourceFileWorkflowData {
    pub source_root: String,
    pub default_path_prefix: String,
    pub extensions: Vec<String>,
    pub can_create: bool,
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRootData {
    pub workspace_root_id: i64,
    pub root_id: i64,
    pub declared_root_id: String,
    pub owner_id: String,
    pub source_root: String,
    pub display_name: String,
    pub portable_key: String,
    pub output_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBrowserFolderData {
    pub key: String,
    /// Resolved asset type shared by the category and every folder beneath it.
    pub category_kind: EditorTypeKind,
    pub name: String,
    /// Normalized folder path within the category. `None` is the category row.
    pub relative_path: Option<String>,
    pub depth: usize,
    pub has_children: bool,
    /// Precomputed category-to-parent keys keep row visibility allocation-free.
    pub ancestor_keys: Vec<String>,
    pub count: usize,
}

impl AssetBrowserFolderData {
    #[must_use]
    pub fn breadcrumb(&self) -> String {
        let category = self
            .category_kind
            .asset_category_label()
            .unwrap_or("Assets");
        if self.relative_path.is_none() {
            return category.to_string();
        }
        format!(
            "{} / {}",
            category,
            self.relative_path.as_deref().unwrap_or_default()
        )
    }

    #[must_use]
    pub const fn is_category(&self) -> bool {
        self.relative_path.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBrowserEntryData {
    pub entry_id: i64,
    pub workspace_id: i64,
    pub asset_guid: String,
    pub root_id: i64,
    pub source_path: String,
    pub schema_type: Option<String>,
    pub content_hash: String,
    pub status: AssetBrowserEntryStatus,
    pub diagnostics_count: i64,
    pub latest_job: Option<AssetBrowserJobData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetBrowserEntryStatus {
    Clean,
    Added,
    Modified,
    Deleted,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBrowserJobData {
    pub job_id: i64,
    pub attempt_id: Option<i64>,
    pub job_key: String,
    pub platform: String,
    pub ordinal: Option<i64>,
    pub status: AssetBrowserJobStatus,
    pub error_count: i64,
    pub warning_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetBrowserJobStatus {
    Queued,
    Leased,
    Succeeded,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAssetSourcePreview {
    pub source_path: String,
    pub source_root: Option<String>,
    pub locator: Option<AssetSourcePreviewLocator>,
    pub schema_type: Option<String>,
    pub status: AssetBrowserEntryStatus,
    pub latest_job_status: Option<AssetBrowserJobStatus>,
    pub preview_kind: AssetSourcePreviewKind,
}

impl Global for EditorAssetSourcePreview {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetSourcePreviewLocator {
    SourceFile {
        source_root: String,
        source_path: String,
    },
}

impl AssetSourcePreviewLocator {
    #[must_use]
    pub fn source_path(&self) -> String {
        match self {
            Self::SourceFile {
                source_root,
                source_path,
            } => join_source_root_path(source_root, source_path),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetSourcePreviewKind {
    Image,
    Document,
    Model,
    Motion,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorJobInspection {
    pub job_id: i64,
    pub attempt_id: Option<i64>,
    pub source_path: Option<String>,
    pub job_key: Option<String>,
    pub platform: Option<String>,
    pub ordinal: Option<i64>,
    pub status: Option<AssetBrowserJobStatus>,
    pub dependencies: Vec<JobDependencyData>,
    pub products: Vec<JobProductData>,
    pub status_error: Option<String>,
}

impl EditorJobInspection {
    #[must_use]
    #[allow(clippy::too_many_arguments)] // mirrors the typed job inspection shape
    pub fn new(
        job_id: i64,
        attempt_id: Option<i64>,
        source_path: impl Into<String>,
        job_key: impl Into<String>,
        platform: impl Into<String>,
        ordinal: Option<i64>,
        status: AssetBrowserJobStatus,
        dependencies: Vec<JobDependencyData>,
        products: Vec<JobProductData>,
    ) -> Self {
        Self {
            job_id,
            attempt_id,
            source_path: Some(source_path.into()),
            job_key: Some(job_key.into()),
            platform: Some(platform.into()),
            ordinal,
            status: Some(status),
            dependencies,
            products,
            status_error: None,
        }
    }

    #[must_use]
    pub fn error(job_id: i64, error: impl Into<String>) -> Self {
        Self {
            job_id,
            attempt_id: None,
            source_path: None,
            job_key: None,
            platform: None,
            ordinal: None,
            status: None,
            dependencies: Vec::new(),
            products: Vec::new(),
            status_error: Some(error.into()),
        }
    }
}

impl Global for EditorJobInspection {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProductData {
    pub product_id: i64,
    pub product_path: String,
    pub asset_type: String,
    pub sub_id: i64,
    pub product_format: String,
    pub product_format_version: u32,
    pub content_hash: String,
    pub byte_length: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDependencyData {
    pub job_edge_id: i64,
    pub target: String,
    pub job_key: String,
    pub platform: String,
    pub dependency_kind: String,
}

/// Asset browser panel
///
/// Displays the session's attached workspace snapshot.
pub struct AssetBrowser {
    filter_input: Entity<InputState>,

    /// Search filter
    filter: String,

    /// View mode (tree vs grid)
    view_mode: ViewMode,

    selected_folder_key: Option<String>,

    /// Collapsed source-tree nodes. Empty means the full source tree is open.
    collapsed_folders: BTreeSet<String>,

    _subscriptions: Vec<Subscription>,
}

/// Asset browser view mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Tree view (hierarchical)
    Tree,
    /// Grid view (thumbnails)
    Grid,
}

impl AssetBrowser {
    pub const NAME: &'static str = "asset_browser";

    #[must_use]
    pub const fn new(filter_input: Entity<InputState>, subscriptions: Vec<Subscription>) -> Self {
        Self {
            filter_input,
            filter: String::new(),
            view_mode: ViewMode::Grid,
            selected_folder_key: None,
            collapsed_folders: BTreeSet::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search assets..."));
        let subscriptions = vec![cx.subscribe_in(&filter_input, window, Self::on_filter_input)];
        Self::new(filter_input, subscriptions)
    }

    /// Set search filter
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
    }

    /// Toggle view mode
    pub const fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Tree => ViewMode::Grid,
            ViewMode::Grid => ViewMode::Tree,
        };
    }

    fn select_folder(&mut self, key: String, cx: &mut Context<'_, Self>) {
        if self.selected_folder_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.selected_folder_key = Some(key);
        cx.emit(PanelEvent::LayoutChanged);
        cx.notify();
    }

    fn toggle_folder(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        if !self.collapsed_folders.remove(key) {
            self.collapsed_folders.insert(key.to_owned());
        }
        cx.notify();
    }

    fn set_grid_view(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.view_mode != ViewMode::Grid {
            self.view_mode = ViewMode::Grid;
            cx.emit(PanelEvent::LayoutChanged);
            cx.notify();
        }
    }

    fn set_list_view(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.view_mode != ViewMode::Tree {
            self.view_mode = ViewMode::Tree;
            cx.emit(PanelEvent::LayoutChanged);
            cx.notify();
        }
    }

    #[must_use]
    pub const fn persisted_view_mode(&self) -> &'static str {
        match self.view_mode {
            ViewMode::Tree => "list",
            ViewMode::Grid => "grid",
        }
    }

    #[must_use]
    pub fn persisted_folder_key(&self) -> Option<&str> {
        self.selected_folder_key.as_deref()
    }

    pub fn restore_persisted_state(
        &mut self,
        view_mode: &str,
        folder_key: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.view_mode = if view_mode == "list" {
            ViewMode::Tree
        } else {
            ViewMode::Grid
        };
        self.selected_folder_key = folder_key;
        cx.notify();
    }

    fn on_filter_input(
        &mut self,
        state: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.filter = state.read(cx).value().to_string();
        }
    }

    fn focus_filter_input(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.filter_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn sync_filter_input(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let focused = self
            .filter_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !focused && self.filter_input.read(cx).value() != self.filter {
            let filter = self.filter.clone();
            self.filter_input
                .update(cx, |input, cx| input.set_value(filter, window, cx));
        }
    }
    /// Top strip: the filter input, the source-type chips, and the
    /// grid/list view switch.
    fn render_asset_browser_toolbar(
        &self,
        selected_folder: Option<&AssetBrowserFolderData>,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        h_flex()
            .h(px(31.0))
            .flex_none()
            .items_center()
            .gap_1p5()
            .px_2()
            .bg(theme.tab_bar)
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::new(IconName::ArrowLeft)
                    .with_size(px(15.0))
                    .text_color(theme.muted_foreground),
            )
            .child(
                Icon::new(IconName::ArrowRight)
                    .with_size(px(15.0))
                    .text_color(theme.muted_foreground),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child("Assets")
                    .child(Icon::new(IconName::ChevronRight).with_size(px(14.0)))
                    .children(selected_folder.map(|folder| {
                        Icon::new(folder.category_kind.icon())
                            .with_size(px(14.0))
                            .text_color(folder.category_kind.tint(theme))
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(theme.foreground)
                            .child(selected_folder.map_or_else(
                                || "No typed assets".to_string(),
                                AssetBrowserFolderData::breadcrumb,
                            )),
                    ),
            )
            .child(self.render_asset_browser_controls(theme, cx))
    }

    /// Right-hand toolbar cluster: the filter field and the grid/list view
    /// switch.
    fn render_asset_browser_controls(
        &self,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        h_flex()
            .ml_auto()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .w(px(176.0))
                    .h(px(23.0))
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .rounded(px(5.0))
                    .bg(theme.input_background())
                    .border_1()
                    .border_color(theme.border)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_filter_input))
                    .child(
                        Icon::new(IconName::Search)
                            .with_size(px(14.0))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.filter_input)
                                .small()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                    ),
            )
            .child(self.render_view_mode_switch(theme, cx))
    }

    /// The grid/list segmented control on the toolbar's trailing edge.
    fn render_view_mode_switch(
        &self,
        theme: &gpui_component::theme::Theme,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap(px(1.0))
            .p(px(2.0))
            .rounded(px(5.0))
            .bg(theme.tab_bar_segmented)
            .child(
                asset_browser_mode_button(
                    "asset-browser-grid",
                    IconName::LayoutDashboard,
                    self.view_mode == ViewMode::Grid,
                    theme,
                )
                .on_mouse_down(MouseButton::Left, cx.listener(Self::set_grid_view)),
            )
            .child(
                asset_browser_mode_button(
                    "asset-browser-list",
                    IconName::Menu,
                    self.view_mode == ViewMode::Tree,
                    theme,
                )
                .on_mouse_down(MouseButton::Left, cx.listener(Self::set_list_view)),
            )
    }

    /// Body: the source rail beside the entry grid/list, the selected-asset
    /// header, and the asset-processor error banner when status carries one.
    fn render_asset_browser_content(
        &self,
        view: AssetBrowserView<'_>,
        status_error: Option<String>,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        h_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .items_stretch()
            .child(render_asset_source_rail(
                view.folders,
                view.selected_folder.map(|folder| folder.key.as_str()),
                &self.collapsed_folders,
                cx,
                view.theme,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .bg(view.theme.input_background())
                    .children(view.selected_source_path.and_then(|source_path| {
                        view.status.and_then(|status| {
                            status
                                .entries
                                .iter()
                                .find(|entry| entry.source_path == source_path)
                                .map(|entry| {
                                    render_selected_asset_header(
                                        entry,
                                        view.builder_catalog,
                                        view.catalog_products,
                                        view.theme,
                                    )
                                })
                        })
                    }))
                    .child(
                        div()
                            .flex_1()
                            .w_full()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(render_asset_entries_browser(
                                view,
                                &self.filter,
                                self.view_mode,
                            )),
                    )
                    // Design HTML: asset browser is folders + grid/list only.
                    // Pipeline/job diagnostics belong in Asset Processor window /
                    // status-bar drawers — not a permanent bottom stack here.
                    .children(status_error.map(|error| {
                        div()
                            .flex_none()
                            .border_t_1()
                            .border_color(view.theme.border)
                            .bg(view.theme.input_background())
                            .p_2()
                            .child(render_asset_browser_error(&error, view.theme))
                    })), // Selection belongs to the card/row itself. Keeping
                         // the DC's folders + grid/list composition lets the
                         // browser content consume the full panel height.
            )
    }
}

impl Render for AssetBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) =
            crate::panels::render_project_host_connection_placeholder("Asset Browser", cx)
        {
            return placeholder;
        }
        self.sync_filter_input(window, cx);
        let theme = cx.theme().clone();
        let theme = &theme;
        let status = cx.try_global::<EditorAssetBrowserStatus>().cloned();
        let builder_catalog = cx.try_global::<EditorAssetBuilderCatalog>().cloned();
        let catalog_products = cx.try_global::<EditorCatalogProductsStatus>().cloned();
        let selected_source_preview = cx.try_global::<EditorAssetSourcePreview>().cloned();
        let has_more_assets = status.as_ref().is_some_and(asset_browser_has_more);
        let status_error = status
            .as_ref()
            .and_then(|status| status.status_error.clone());
        let roots = status
            .as_ref()
            .map(|status| status.roots.clone())
            .unwrap_or_default();
        let folders = status.as_ref().map_or_else(Vec::new, |status| {
            asset_browser_folders_with_types(
                status,
                builder_catalog.as_ref(),
                catalog_products.as_ref(),
            )
        });
        let selected_folder = self
            .selected_folder_key
            .as_deref()
            .and_then(|key| asset_browser_folder_for_key(&folders, key))
            .or_else(|| folders.first());
        let entries = status.as_ref().map_or_else(Vec::new, |status| {
            selected_folder.map_or_else(Vec::new, |selected_folder| {
                filtered_asset_entries(
                    status,
                    Some(selected_folder),
                    &self.filter,
                    builder_catalog.as_ref(),
                    catalog_products.as_ref(),
                )
            })
        });
        let root_by_id = roots
            .iter()
            .map(|root| (root.root_id, root.clone()))
            .collect::<BTreeMap<_, _>>();
        let total_entries = folders
            .iter()
            .filter(|folder| folder.is_category())
            .map(|folder| folder.count)
            .sum::<usize>();
        let visible_entries = entries.len();
        let root_count = roots.len();

        let source_type_count = builder_catalog
            .as_ref()
            .map_or(0, |catalog| catalog.source_schemas.len());
        let view = AssetBrowserView {
            status: status.as_ref(),
            folders: &folders,
            selected_folder,
            entries: &entries,
            root_by_id: &root_by_id,
            builder_catalog: builder_catalog.as_ref(),
            catalog_products: catalog_products.as_ref(),
            selected_source_path: selected_source_preview
                .as_ref()
                .map(|preview| preview.source_path.as_str()),
            has_more_assets,
            theme,
        };

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.input_background())
            .child(self.render_asset_browser_toolbar(selected_folder, theme, cx))
            .child(self.render_asset_browser_content(view, status_error, cx))
            .child(kit::count_footer(theme).child(format!(
                "{visible_entries}/{total_entries} assets · {root_count} roots · {source_type_count} types"
            )))
            .into_any_element()
    }
}

impl Focusable for AssetBrowser {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter_input.read(cx).focus_handle(cx)
    }
}

impl Panel for AssetBrowser {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("folder"), "Asset Browser", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &gpui::App) -> bool {
        false
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> Option<Vec<gpui_component::button::Button>> {
        use gpui_component::button::{Button, ButtonVariants as _};
        Some(vec![
            Button::new("asset-browser-refresh")
                .icon(IconName::Replace)
                .ghost()
                .small()
                .tooltip("Refresh assets")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::RefreshAssets), cx);
                }),
        ])
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for AssetBrowser {}

fn asset_browser_mode_button(
    id: &'static str,
    icon: IconName,
    active: bool,
    theme: &gpui_component::theme::Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(21.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .text_color(if active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .when(active, |this| this.bg(theme.secondary))
        .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
        .cursor_pointer()
        .child(Icon::new(icon).with_size(px(15.0)))
}

fn render_asset_source_rail(
    folders: &[AssetBrowserFolderData],
    selected_folder_key: Option<&str>,
    collapsed_folders: &BTreeSet<String>,
    cx: &Context<'_, AssetBrowser>,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    v_flex()
        .w(px(172.0))
        .h_full()
        .flex_none()
        .min_h_0()
        .bg(theme.sidebar)
        .border_r_1()
        .border_color(theme.border)
        .py_1()
        .when(folders.is_empty(), |this| {
            this.child(
                div()
                    .p_2()
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child("No typed assets"),
            )
        })
        .children(
            visible_asset_browser_folders(folders, collapsed_folders).map(|folder| {
                render_asset_source_rail_row(
                    folder,
                    selected_folder_key == Some(folder.key.as_str()),
                    !collapsed_folders.contains(&folder.key),
                    cx,
                    theme,
                )
            }),
        )
        .into_any_element()
}

fn render_asset_source_rail_row(
    folder: &AssetBrowserFolderData,
    selected: bool,
    expanded: bool,
    cx: &Context<'_, AssetBrowser>,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let key = folder.key.clone();
    let caret_key = key.clone();
    let kind = folder.category_kind;
    kit::list_row(
        gpui::SharedString::from(format!("asset-browser-folder-{}", folder.key)),
        theme,
        selected,
    )
    .h(px(28.0))
    .pl(kit::indent(folder.depth, 13.0, 5.0))
    .on_click(cx.listener(move |this, _, _, cx| {
        this.select_folder(key.clone(), cx);
    }))
    .child(
        kit::row_caret(folder.has_children.then_some(expanded), theme)
            .id(format!("asset-browser-folder-caret-{}", folder.key))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_folder(&caret_key, cx);
            })),
    )
    .child(kit::row_icon(
        if folder.is_category() {
            kind.icon()
        } else {
            IconName::FolderClosed
        },
        kind.tint(theme),
    ))
    .child(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .text_size(px(11.0))
            .child(folder.name.clone()),
    )
    .child(
        div()
            .font_family("monospace")
            .text_size(px(10.0))
            .text_color(theme.muted_foreground)
            .child(folder.count.to_string()),
    )
    .into_any_element()
}

/// Everything one asset-browser paint reads from: the asset-processor status
/// and the catalogs that resolve an entry's type, the workspace roots that
/// name its source, the folder tree and the entries selected out of it, and
/// the theme it all paints in. Every entry renderer takes the same one.
#[derive(Clone, Copy)]
struct AssetBrowserView<'a> {
    status: Option<&'a EditorAssetBrowserStatus>,
    folders: &'a [AssetBrowserFolderData],
    selected_folder: Option<&'a AssetBrowserFolderData>,
    entries: &'a [&'a AssetBrowserEntryData],
    root_by_id: &'a BTreeMap<i64, WorkspaceRootData>,
    builder_catalog: Option<&'a EditorAssetBuilderCatalog>,
    catalog_products: Option<&'a EditorCatalogProductsStatus>,
    selected_source_path: Option<&'a str>,
    has_more_assets: bool,
    theme: &'a gpui_component::theme::Theme,
}

fn render_asset_entries_browser(
    view: AssetBrowserView<'_>,
    filter: &str,
    view_mode: ViewMode,
) -> gpui::AnyElement {
    let AssetBrowserView {
        status,
        entries,
        root_by_id,
        has_more_assets,
        theme,
        ..
    } = view;
    let filtered = !filter.trim().is_empty();
    let show_roots = root_by_id.len() > 1;
    match status {
        None => kit::empty_state(
            "Asset status unavailable",
            Some("waiting for asset-processor".to_string()),
            theme,
        )
        .into_any_element(),
        Some(_) if entries.is_empty() && filtered => {
            kit::empty_state("No matching source assets", Some(filter.to_string()), theme)
                .into_any_element()
        }
        Some(_) if entries.is_empty() => kit::empty_state(
            "No indexed source assets",
            Some("source roots are mounted, but no assets are published yet".to_string()),
            theme,
        )
        .into_any_element(),
        Some(_) => match view_mode {
            ViewMode::Grid => div()
                .w_full()
                .min_h_0()
                .flex()
                .flex_wrap()
                .gap_3()
                .p_3()
                .children(entries.iter().map(|entry| {
                    render_asset_grid_card(
                        entry,
                        show_roots,
                        view.selected_source_path == Some(entry.source_path.as_str()),
                        view,
                    )
                }))
                .when(has_more_assets, |this| {
                    this.child(asset_browser_load_more_button(theme).into_any_element())
                })
                .into_any_element(),
            ViewMode::Tree => v_flex()
                .w_full()
                .child(
                    h_flex()
                        .h(px(24.0))
                        .flex_none()
                        .items_center()
                        .px_3()
                        .border_b_1()
                        .border_color(theme.border)
                        .bg(theme.sidebar)
                        .text_size(px(10.0))
                        .text_color(theme.muted_foreground)
                        .child(div().flex_1().child("Name"))
                        .child(div().w(px(88.0)).child("Type"))
                        .child(
                            div()
                                .w(px(88.0))
                                .text_align(gpui::TextAlign::Right)
                                .child("Status"),
                        )
                        .child(
                            div()
                                .w(px(112.0))
                                .text_align(gpui::TextAlign::Right)
                                .child("Job"),
                        ),
                )
                .children(entries.iter().map(|entry| {
                    render_asset_list_row(
                        entry,
                        view.selected_source_path == Some(entry.source_path.as_str()),
                        view,
                    )
                }))
                .when(has_more_assets, |this| {
                    this.child(asset_browser_load_more_button(theme).into_any_element())
                })
                .into_any_element(),
        },
    }
}

#[derive(Clone)]
struct AssetTypeIdentity {
    label: String,
    kind: EditorTypeKind,
}

fn render_selected_asset_header(
    entry: &AssetBrowserEntryData,
    builder_catalog: Option<&EditorAssetBuilderCatalog>,
    catalog_products: Option<&EditorCatalogProductsStatus>,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let asset_type = asset_type_identity(entry, builder_catalog, catalog_products);
    let raw_file = raw_asset_file_name(&entry.source_path);
    let tooltip = format!("Type: {}\nFile: {raw_file}", asset_type.label);
    h_flex()
        .id("asset-browser-selected-source-header")
        .h(px(28.0))
        .flex_none()
        .items_center()
        .gap_2()
        .px_3()
        .bg(theme.sidebar)
        .border_b_1()
        .border_color(theme.border)
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .child(
            Icon::new(asset_type.kind.icon())
                .with_size(px(15.0))
                .text_color(asset_type.kind.tint(theme)),
        )
        .child(
            div()
                .min_w_0()
                .text_size(px(11.0))
                .text_color(theme.foreground)
                .child(asset_file_name(&entry.source_path)),
        )
        .child(
            div()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(asset_type.kind.tint(theme).opacity(0.16))
                .text_size(px(9.5))
                .text_color(asset_type.kind.tint(theme))
                .child(asset_type.label),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .font_family("monospace")
                .text_size(px(9.5))
                .text_color(theme.muted_foreground)
                .child(raw_file),
        )
        .into_any_element()
}

fn asset_type_identity(
    entry: &AssetBrowserEntryData,
    builder_catalog: Option<&EditorAssetBuilderCatalog>,
    catalog_products: Option<&EditorCatalogProductsStatus>,
) -> AssetTypeIdentity {
    let identity_for = |raw_type: &str| {
        let source_schema = builder_catalog.and_then(|catalog| {
            catalog
                .source_schemas
                .iter()
                .find(|schema| schema.schema_type == raw_type)
        });
        let explicit_label = source_schema.map(|schema| schema.label.as_str());
        let label = crate::naming::schema_display_name(raw_type, explicit_label).into_owned();
        let label = canonical_asset_type_label(label);
        let semantic_label = source_schema.map_or_else(
            || label.clone(),
            |schema| format!("{} {}", label, schema.category),
        );
        AssetTypeIdentity {
            kind: asset_kind(raw_type, &semantic_label),
            label,
        }
    };

    let schema_identity = entry.schema_type.as_deref().map(identity_for);
    if let Some(identity) = schema_identity.as_ref()
        && identity.kind != EditorTypeKind::Source
    {
        return identity.clone();
    }

    let builder_identity = builder_catalog.and_then(|catalog| {
        catalog
            .builders
            .iter()
            .filter(|builder| builder_matches_source(builder, &entry.source_path))
            .find_map(|builder| {
                builder
                    .source_schema_types
                    .iter()
                    .map(|raw_type| identity_for(raw_type))
                    .find(|identity| identity.kind != EditorTypeKind::Source)
                    .or_else(|| {
                        let label = builder
                            .name
                            .strip_suffix(" Builder")
                            .unwrap_or(&builder.name)
                            .to_string();
                        let raw_type = builder
                            .source_schema_types
                            .first()
                            .map_or(builder.builder_guid.as_str(), String::as_str);
                        let kind = asset_kind(raw_type, &label);
                        (kind != EditorTypeKind::Source)
                            .then_some(AssetTypeIdentity { label, kind })
                    })
            })
    });
    if let Some(identity) = builder_identity {
        return identity;
    }

    let product_type = catalog_products.and_then(|products| {
        products
            .entries
            .iter()
            .find(|product| {
                product.asset_guid == entry.asset_guid || product.source_path == entry.source_path
            })
            .map(|product| product.asset_type.as_str())
            .filter(|asset_type| !asset_type.trim().is_empty())
    });
    if let Some(identity) = product_type.map(identity_for)
        && identity.kind != EditorTypeKind::Source
    {
        return identity;
    }

    schema_identity.unwrap_or_else(|| AssetTypeIdentity {
        kind: EditorTypeKind::Source,
        label: "Source Asset".to_string(),
    })
}

fn canonical_asset_type_label(mut label: String) -> String {
    for suffix in [" Source", " Document", " Asset"] {
        if label.len() > suffix.len() && label.ends_with(suffix) {
            label.truncate(label.len() - suffix.len());
            break;
        }
    }
    label
}

fn builder_matches_source(builder: &AssetBuilderData, source_path: &str) -> bool {
    let source_path = source_path.replace('\\', "/").to_ascii_lowercase();
    builder.patterns.iter().any(|pattern| {
        matches!(pattern.kind, AssetBuilderPatternKindData::Wildcard)
            && wildcard_matches(&pattern.pattern.to_ascii_lowercase(), &source_path)
    })
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut star_index, mut star_value_index) = (None, 0);
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_value_index = value_index;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn render_asset_grid_card(
    entry: &AssetBrowserEntryData,
    show_source_root: bool,
    selected: bool,
    view: AssetBrowserView<'_>,
) -> gpui::AnyElement {
    let theme = view.theme;
    let source_root = view.root_by_id.get(&entry.root_id);
    let file_name = asset_file_name(&entry.source_path);
    let asset_type = asset_type_identity(entry, view.builder_catalog, view.catalog_products);
    let root_label = source_root.map(source_root_label);
    let tooltip = asset_grid_card_tooltip(entry, &asset_type, root_label.as_ref());
    let tint = asset_type.kind.tint(theme);
    let drag_source_path = entry.source_path.clone();
    let open_source_path = entry.source_path.clone();
    let preview = asset_source_preview_for_entry(entry, source_root);

    v_flex()
        .id(("asset-grid-card", entry.entry_id.cast_unsigned()))
        .w(px(104.0))
        .min_h(px(124.0))
        .items_center()
        .gap_1p5()
        .p_2()
        .rounded(px(7.0))
        .border_1()
        .border_color(if selected {
            theme.list_active_border
        } else {
            theme.border
        })
        .bg(if selected {
            theme.list_active
        } else {
            theme.tiles
        })
        .hover(|this| {
            this.border_color(theme.list_active_border)
                .bg(theme.list_hover)
        })
        .cursor_pointer()
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .on_drag(
            ViewportAssetDrag::new(drag_source_path),
            |drag, _, _, cx| cx.new(|_| drag.clone()),
        )
        .on_click(move |_, _window, cx| {
            cx.stop_propagation();
            cx.set_global(preview.clone());
        })
        .on_double_click(move |_, window, cx| {
            dispatch_asset_source_open_actions(&open_source_path, window, cx);
        })
        .child(render_asset_card_preview(
            entry,
            source_root,
            &asset_type,
            tint,
            theme,
        ))
        .child(
            div()
                .w_full()
                .text_align(gpui::TextAlign::Center)
                .text_size(px(10.5))
                .text_color(theme.foreground)
                .child(file_name),
        )
        .children(show_source_root.then(|| render_asset_card_root_chip(root_label, theme)))
        .child(
            div()
                .font_family("monospace")
                .text_size(px(9.5))
                .text_color(theme.muted_foreground)
                .child(entry.status.label()),
        )
        .into_any_element()
}

/// Hover tooltip for one grid card: type and file, plus the source root when
/// the card is disambiguating roots.
fn asset_grid_card_tooltip(
    entry: &AssetBrowserEntryData,
    asset_type: &AssetTypeIdentity,
    root_label: Option<&String>,
) -> String {
    let file = raw_asset_file_name(&entry.source_path);
    root_label.map_or_else(
        || format!("Type: {}\nFile: {}", asset_type.label, file),
        |root| format!("Type: {}\nFile: {}\nRoot: {root}", asset_type.label, file),
    )
}

/// The source-root chip under a grid card, shown only when entries from more
/// than one source root are on screen together.
fn render_asset_card_root_chip(
    root_label: Option<String>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .max_w_full()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .px(px(4.0))
        .rounded(px(3.0))
        .bg(theme.sidebar)
        .font_family("monospace")
        .text_size(px(8.5))
        .text_color(theme.muted_foreground)
        .child(root_label.unwrap_or_else(|| "unknown root".to_string()))
}

/// The card's preview tile: the thumbnail (or its type icon) with the type
/// label pinned to the corner.
fn render_asset_card_preview(
    entry: &AssetBrowserEntryData,
    source_root: Option<&WorkspaceRootData>,
    asset_type: &AssetTypeIdentity,
    tint: gpui::Hsla,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .relative()
        .w_full()
        .h(px(72.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(theme.input_background())
        .border_1()
        .border_color(theme.border)
        .child(render_asset_card_thumbnail(
            entry,
            source_root,
            asset_type.kind.icon(),
            tint,
        ))
        .child(
            div()
                .absolute()
                .bottom(px(4.0))
                .right(px(5.0))
                .px(px(4.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(theme.input_background().opacity(0.82))
                .font_family("monospace")
                .text_size(px(9.0))
                .text_color(tint)
                .child(asset_type.label.clone()),
        )
}

fn source_root_label(root: &WorkspaceRootData) -> String {
    let display_name = root.display_name.trim();
    if display_name.is_empty() {
        root.portable_key.clone()
    } else {
        display_name.to_string()
    }
}

fn render_asset_list_row(
    entry: &AssetBrowserEntryData,
    selected: bool,
    view: AssetBrowserView<'_>,
) -> gpui::AnyElement {
    let theme = view.theme;
    let source_root = view.root_by_id.get(&entry.root_id);
    let file_name = asset_file_name(&entry.source_path);
    let asset_type = asset_type_identity(entry, view.builder_catalog, view.catalog_products);
    let tooltip = format!(
        "Type: {}\nFile: {}",
        asset_type.label,
        raw_asset_file_name(&entry.source_path)
    );
    let drag_source_path = entry.source_path.clone();
    let open_source_path = entry.source_path.clone();
    let preview = asset_source_preview_for_entry(entry, source_root);
    let job_label = entry
        .latest_job
        .as_ref()
        .map_or_else(|| "none".to_string(), |job| job.status.label().to_string());

    kit::list_row(
        ("asset-list-row", entry.entry_id.cast_unsigned()),
        theme,
        selected,
    )
    .h(px(28.0))
    .items_center()
    .px_3()
    .border_b_1()
    .border_color(theme.border.opacity(0.65))
    .tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
    })
    .on_drag(
        ViewportAssetDrag::new(drag_source_path),
        |drag, _, _, cx| cx.new(|_| drag.clone()),
    )
    .on_click(move |_, _window, cx| {
        cx.stop_propagation();
        cx.set_global(preview.clone());
    })
    .on_double_click(move |_, window, cx| {
        dispatch_asset_source_open_actions(&open_source_path, window, cx);
    })
    .child(render_asset_row_name_cell(
        entry,
        source_root,
        &asset_type,
        file_name,
        theme,
    ))
    .child(
        div()
            .w(px(88.0))
            .font_family("monospace")
            .text_size(px(10.5))
            .text_color(theme.muted_foreground)
            .child(asset_type.label),
    )
    .child(
        div()
            .w(px(88.0))
            .text_align(gpui::TextAlign::Right)
            .font_family("monospace")
            .text_size(px(10.5))
            .text_color(entry.status.tone().color(theme))
            .child(entry.status.label()),
    )
    .child(
        div()
            .w(px(112.0))
            .text_align(gpui::TextAlign::Right)
            .font_family("monospace")
            .text_size(px(10.5))
            .text_color(theme.muted_foreground)
            .child(job_label),
    )
    .into_any_element()
}

/// The list row's name cell: type icon, file name, and the source-path
/// subtitle.
fn render_asset_row_name_cell(
    entry: &AssetBrowserEntryData,
    source_root: Option<&WorkspaceRootData>,
    asset_type: &AssetTypeIdentity,
    file_name: String,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap_2()
        .child(
            Icon::new(asset_type.kind.icon())
                .with_size(px(16.0))
                .text_color(asset_type.kind.tint(theme)),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(11.0))
                .text_color(theme.foreground)
                .child(file_name),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .font_family("monospace")
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(source_root.map_or_else(
                    || entry.source_path.clone(),
                    |_| format!("@assets@/{}", entry.source_path),
                )),
        )
}

fn render_asset_card_thumbnail(
    entry: &AssetBrowserEntryData,
    source_root: Option<&WorkspaceRootData>,
    type_icon: IconName,
    tint: Hsla,
) -> gpui::AnyElement {
    let preview = asset_source_preview_for_entry(entry, source_root);
    if preview.preview_kind == AssetSourcePreviewKind::Image
        && let Some(locator) = preview.locator.as_ref()
    {
        let source_path = locator.source_path();
        return img(Path::new(&source_path))
            .w_full()
            .h_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element();
    }

    Icon::new(type_icon)
        .with_size(px(30.0))
        .text_color(tint.opacity(0.85))
        .into_any_element()
}

fn asset_browser_load_more_button(theme: &gpui_component::theme::Theme) -> impl IntoElement {
    div()
        .id("asset-browser-load-more")
        .mt_2()
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .hover(|this| this.bg(theme.muted))
        .cursor_pointer()
        .text_xs()
        .text_color(theme.foreground)
        .child("Load more")
        .on_click(|_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(crate::actions::LoadMoreAssets), cx);
        })
}

fn render_asset_browser_error(
    error: &str,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    div()
        .w_full()
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.foreground)
        .child(format!("Asset processor error: {error}"))
        .into_any_element()
}

#[cfg(test)]
fn asset_builder_patterns_label(patterns: &[AssetBuilderPatternData]) -> String {
    if patterns.is_empty() {
        return "patterns: none".to_string();
    }

    patterns
        .iter()
        .map(|pattern| {
            format!(
                "{}:{}",
                asset_builder_pattern_kind_label(pattern.kind),
                pattern.pattern
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
fn asset_builder_source_schema_label(source_schema_types: &[String]) -> String {
    if source_schema_types.is_empty() {
        return "schemas: any".to_string();
    }

    format!("schemas: {}", source_schema_types.join(", "))
}

#[cfg(test)]
fn asset_source_schema_title(source_schema: &AssetSourceSchemaData) -> String {
    if source_schema.category.is_empty() {
        return asset_source_schema_label(source_schema);
    }
    format!(
        "{} / {}",
        source_schema.category,
        asset_source_schema_label(source_schema)
    )
}

#[cfg(test)]
fn asset_source_schema_label(source_schema: &AssetSourceSchemaData) -> String {
    if source_schema.label.is_empty() {
        return source_schema
            .schema_type
            .rsplit(['.', ':'])
            .find(|part| !part.is_empty())
            .unwrap_or(&source_schema.schema_type)
            .to_string();
    }
    source_schema.label.clone()
}

#[cfg(test)]
fn asset_source_schema_authoring_label(source_schema: &AssetSourceSchemaData) -> String {
    match &source_schema.authoring {
        AssetSourceSchemaAuthoringData::File { workflow } if source_schema.owner.is_empty() => {
            format_file_workflow_label(workflow)
        }
        AssetSourceSchemaAuthoringData::File { workflow } => {
            format!(
                "{} ({})",
                format_file_workflow_label(workflow),
                source_schema.owner
            )
        }
        AssetSourceSchemaAuthoringData::ProjectDocument { schema_type } => {
            format!("project document: {schema_type}")
        }
    }
}

#[cfg(test)]
fn asset_source_schema_catalog_label(source_schema: &AssetSourceSchemaData) -> String {
    let authoring = asset_source_schema_authoring_label(source_schema);
    if source_schema.file_templates.is_empty()
        || !source_schema_supports_default_create_templates(source_schema)
    {
        return authoring;
    }
    let template_count = source_schema.file_templates.len();
    format!("{authoring}; {template_count} create template(s)")
}

#[cfg(test)]
const MAX_VISIBLE_SOURCE_FILE_TEMPLATES: usize = 8;

#[cfg(test)]
fn visible_source_file_templates<'a>(
    source_schema: &'a AssetSourceSchemaData,
    filter: &str,
) -> Vec<&'a AssetSourceFileTemplateData> {
    if !source_schema_supports_default_create_templates(source_schema) {
        return Vec::new();
    }
    let filter = filter.trim().to_ascii_lowercase();
    if filter.is_empty() {
        return Vec::new();
    }
    source_schema
        .file_templates
        .iter()
        .filter(|template| {
            asset_source_file_template_matches_filter(source_schema, template, &filter)
        })
        .take(MAX_VISIBLE_SOURCE_FILE_TEMPLATES)
        .collect()
}

#[cfg(test)]
fn asset_source_file_template_matches_filter(
    source_schema: &AssetSourceSchemaData,
    template: &AssetSourceFileTemplateData,
    filter: &str,
) -> bool {
    asset_source_schema_label(source_schema)
        .to_ascii_lowercase()
        .contains(filter)
        || source_schema
            .schema_type
            .to_ascii_lowercase()
            .contains(filter)
        || template.source_path.to_ascii_lowercase().contains(filter)
        || template.label.to_ascii_lowercase().contains(filter)
        || template.description.to_ascii_lowercase().contains(filter)
}

#[cfg(test)]
const fn source_schema_supports_default_create_templates(
    source_schema: &AssetSourceSchemaData,
) -> bool {
    matches!(
        &source_schema.authoring,
        AssetSourceSchemaAuthoringData::File { workflow } if workflow.can_create
    )
}

#[cfg(test)]
fn asset_source_file_template_label(template: &AssetSourceFileTemplateData) -> String {
    if template.label.is_empty() {
        return raw_asset_file_name(&template.source_path);
    }
    template.label.clone()
}

#[cfg(test)]
fn format_file_workflow_label(workflow: &AssetSourceFileWorkflowData) -> String {
    let extensions = if workflow.extensions.is_empty() {
        "unknown".to_string()
    } else {
        workflow
            .extensions
            .iter()
            .map(|extension| {
                if extension == "*" {
                    "*".to_string()
                } else {
                    format!(".{extension}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (workflow.can_create, workflow.can_edit) {
        (true, true) => format!("create/edit file: {extensions}"),
        (true, false) => format!("create file: {extensions}"),
        (false, true) => format!("import/edit file: {extensions}"),
        (false, false) => format!("import file: {extensions}"),
    }
}

#[cfg(test)]
const fn asset_builder_pattern_kind_label(kind: AssetBuilderPatternKindData) -> &'static str {
    match kind {
        AssetBuilderPatternKindData::Wildcard => "wildcard",
        AssetBuilderPatternKindData::Regex => "regex",
    }
}

#[must_use]
pub fn asset_browser_folders(status: &EditorAssetBrowserStatus) -> Vec<AssetBrowserFolderData> {
    asset_browser_folders_with_types(status, None, None)
}

/// Build the type-first Asset Browser tree using the same resolved identity
/// used by entry icons and labels.
///
/// Categories aggregate entries from every source root; source-root identity
/// remains on each entry for secondary UI.
#[must_use]
pub fn asset_browser_folders_with_types(
    status: &EditorAssetBrowserStatus,
    builder_catalog: Option<&EditorAssetBuilderCatalog>,
    catalog_products: Option<&EditorCatalogProductsStatus>,
) -> Vec<AssetBrowserFolderData> {
    struct FolderBuild {
        folder: AssetBrowserFolderData,
    }

    let mut folders = BTreeMap::<(usize, String), FolderBuild>::new();

    for entry in &status.entries {
        let kind = asset_type_identity(entry, builder_catalog, catalog_products).kind;
        let Some(category_name) = kind.asset_category_label() else {
            continue;
        };
        let order = ASSET_CATEGORY_KINDS
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(usize::MAX);
        let category_key = asset_browser_folder_key(kind, None);
        folders
            .entry((order, String::new()))
            .or_insert_with(|| FolderBuild {
                folder: AssetBrowserFolderData {
                    key: category_key.clone(),
                    category_kind: kind,
                    name: category_name.to_string(),
                    relative_path: None,
                    depth: 0,
                    has_children: false,
                    ancestor_keys: Vec::new(),
                    count: 0,
                },
            })
            .folder
            .count += 1;

        if let Some(parent_path) = asset_category_parent_path(kind, &entry.source_path) {
            let mut prefix = String::new();
            for segment in parent_path.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(segment);
                let key = asset_browser_folder_key(kind, Some(&prefix));
                let mut ancestor_keys = vec![category_key.clone()];
                let mut ancestor_prefix = String::new();
                let mut ancestors = prefix.split('/').peekable();
                while let Some(ancestor) = ancestors.next() {
                    if ancestors.peek().is_none() {
                        break;
                    }
                    if !ancestor_prefix.is_empty() {
                        ancestor_prefix.push('/');
                    }
                    ancestor_prefix.push_str(ancestor);
                    ancestor_keys.push(asset_browser_folder_key(kind, Some(&ancestor_prefix)));
                }

                let build = folders
                    .entry((order, prefix.clone()))
                    .or_insert_with(|| FolderBuild {
                        folder: AssetBrowserFolderData {
                            key,
                            category_kind: kind,
                            name: segment.to_string(),
                            relative_path: Some(prefix.clone()),
                            depth: prefix.bytes().filter(|byte| *byte == b'/').count() + 1,
                            has_children: false,
                            count: 0,
                            ancestor_keys,
                        },
                    });
                if prefix == parent_path {
                    build.folder.count += 1;
                }
            }
        }
    }

    let parent_keys = folders
        .values()
        .filter_map(|build| build.folder.ancestor_keys.last().cloned())
        .collect::<BTreeSet<_>>();
    folders
        .into_values()
        .map(|mut build| {
            build.folder.has_children = parent_keys.contains(&build.folder.key);
            build.folder
        })
        .collect()
}

fn visible_asset_browser_folders<'a>(
    folders: &'a [AssetBrowserFolderData],
    collapsed: &'a BTreeSet<String>,
) -> impl Iterator<Item = &'a AssetBrowserFolderData> + 'a {
    folders.iter().filter(move |folder| {
        folder
            .ancestor_keys
            .iter()
            .all(|ancestor| !collapsed.contains(ancestor))
    })
}

#[must_use]
pub fn asset_browser_folder_for_key<'a>(
    folders: &'a [AssetBrowserFolderData],
    key: &str,
) -> Option<&'a AssetBrowserFolderData> {
    folders.iter().find(|folder| folder.key == key)
}

#[must_use]
pub fn asset_browser_entry_matches_folder(
    entry: &AssetBrowserEntryData,
    folder: &AssetBrowserFolderData,
) -> bool {
    asset_browser_entry_matches_folder_with_types(entry, folder, None, None)
}

fn asset_browser_entry_matches_folder_with_types(
    entry: &AssetBrowserEntryData,
    folder: &AssetBrowserFolderData,
    builder_catalog: Option<&EditorAssetBuilderCatalog>,
    catalog_products: Option<&EditorCatalogProductsStatus>,
) -> bool {
    let kind = asset_type_identity(entry, builder_catalog, catalog_products).kind;
    if kind != folder.category_kind {
        return false;
    }
    folder.relative_path.as_deref().is_none_or(|folder_path| {
        asset_category_parent_path(kind, &entry.source_path).as_deref() == Some(folder_path)
    })
}

fn asset_browser_folder_key(kind: EditorTypeKind, relative_path: Option<&str>) -> String {
    let category = kind
        .asset_category_label()
        .unwrap_or("assets")
        .to_ascii_lowercase();
    relative_path.map_or_else(
        || format!("asset-category:{category}"),
        |path| format!("asset-category:{category}:{path}"),
    )
}

fn asset_category_parent_path(kind: EditorTypeKind, source_path: &str) -> Option<String> {
    let parent = asset_parent_path(source_path)?;
    let (first, remainder) = parent
        .split_once('/')
        .map_or((parent.as_str(), None), |(first, rest)| (first, Some(rest)));
    if kind
        .asset_route_prefixes()
        .iter()
        .any(|prefix| first.eq_ignore_ascii_case(prefix))
    {
        return remainder
            .filter(|path| !path.is_empty())
            .map(str::to_string);
    }
    Some(parent)
}

fn asset_parent_path(source_path: &str) -> Option<String> {
    let normalized = source_path.replace('\\', "/");
    let (parent, _) = normalized.rsplit_once('/')?;
    let parent = parent.trim_matches('/');
    (!parent.is_empty()).then(|| parent.to_string())
}

fn filtered_asset_entries<'a>(
    status: &'a EditorAssetBrowserStatus,
    folder: Option<&AssetBrowserFolderData>,
    filter: &str,
    builder_catalog: Option<&EditorAssetBuilderCatalog>,
    catalog_products: Option<&EditorCatalogProductsStatus>,
) -> Vec<&'a AssetBrowserEntryData> {
    let filter = filter.trim().to_ascii_lowercase();
    let mut entries = status
        .entries
        .iter()
        .filter(|entry| {
            folder.is_none_or(|folder| {
                asset_browser_entry_matches_folder_with_types(
                    entry,
                    folder,
                    builder_catalog,
                    catalog_products,
                )
            }) && (filter.is_empty()
                || entry
                    .source_path
                    .to_ascii_lowercase()
                    .contains(filter.as_str()))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    entries
}

const fn asset_browser_has_more(status: &EditorAssetBrowserStatus) -> bool {
    status.next_after_entry_id.is_some()
}

fn asset_file_name(source_path: &str) -> String {
    crate::naming::display_name(source_path).into_owned()
}

fn raw_asset_file_name(source_path: &str) -> String {
    source_path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(source_path)
        .to_string()
}

fn asset_source_preview_for_entry(
    entry: &AssetBrowserEntryData,
    source_root: Option<&WorkspaceRootData>,
) -> EditorAssetSourcePreview {
    EditorAssetSourcePreview {
        source_path: entry.source_path.clone(),
        source_root: source_root.map(|root| root.source_root.clone()),
        locator: source_root.map(|root| AssetSourcePreviewLocator::SourceFile {
            source_root: root.source_root.clone(),
            source_path: entry.source_path.clone(),
        }),
        schema_type: entry.schema_type.clone(),
        status: entry.status,
        latest_job_status: entry.latest_job.as_ref().map(|job| job.status),
        preview_kind: asset_source_preview_kind(&entry.source_path),
    }
}

fn join_source_root_path(source_root: &str, source_path: &str) -> String {
    let normalized_source_path = source_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    let normalized_source_root = source_root.replace('\\', "/");
    let source_root = normalized_source_root.trim_end_matches('/');

    if source_root.is_empty() {
        normalized_source_path
    } else if normalized_source_path.is_empty() {
        source_root.to_string()
    } else {
        format!("{source_root}/{normalized_source_path}")
    }
}

/// Case-insensitive suffix test for the compound asset extensions this panel
/// routes on (`.anim.glb`, `.bspace.ron`, ...).
///
/// `Path::extension` only ever yields the last dot-segment, so it cannot tell
/// `.anim.glb` from a plain `.glb`; folding ASCII case over the raw suffix
/// keeps both spellings on one code path without a lowercased copy.
fn has_extension_suffix(path: &str, suffix: &str) -> bool {
    let (path, suffix) = (path.as_bytes(), suffix.as_bytes());
    path.len() >= suffix.len() && path[path.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn asset_source_preview_kind(source_path: &str) -> AssetSourcePreviewKind {
    let normalized = source_path.replace('\\', "/");

    if has_extension_suffix(&normalized, ".anim.glb") {
        return AssetSourcePreviewKind::Motion;
    }

    if has_extension_suffix(&normalized, ".glb") {
        return AssetSourcePreviewKind::Model;
    }

    if has_extension_suffix(&normalized, ".ron") {
        return AssetSourcePreviewKind::Document;
    }

    let lower = normalized.to_ascii_lowercase();
    if let Some(ext) = lower.rsplit('.').next()
        && matches!(
            ext,
            "avif"
                | "jpg"
                | "jpeg"
                | "png"
                | "gif"
                | "webp"
                | "tif"
                | "tiff"
                | "tga"
                | "dds"
                | "bmp"
                | "ico"
                | "hdr"
                | "exr"
                | "pbm"
                | "pam"
                | "ppm"
                | "pgm"
                | "ff"
                | "farbfeld"
                | "qoi"
                | "svg"
        )
    {
        return AssetSourcePreviewKind::Image;
    }

    AssetSourcePreviewKind::Source
}

/// One editor selection a double-clicked asset source should dispatch. Each
/// variant names the document kind, not the verb; dispatch is always a select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssetSourceOpenAction {
    AuthoredDocument,
    AnimationCharacter,
    AnimationMotion,
    AnimationBlendSpace,
}

fn asset_source_open_actions(source_path: &str) -> Vec<AssetSourceOpenAction> {
    let normalized = source_path.replace('\\', "/");
    let mut actions = Vec::new();

    if has_extension_suffix(&normalized, ".ron") {
        actions.push(AssetSourceOpenAction::AuthoredDocument);
    }

    if has_extension_suffix(&normalized, ".anim.glb") {
        actions.push(AssetSourceOpenAction::AnimationMotion);
    } else if has_extension_suffix(&normalized, ".glb") {
        actions.push(AssetSourceOpenAction::AnimationCharacter);
    }

    if has_extension_suffix(&normalized, ".bspace.ron")
        || has_extension_suffix(&normalized, ".comb.ron")
    {
        actions.push(AssetSourceOpenAction::AnimationBlendSpace);
    }

    actions
}

fn dispatch_asset_source_open_actions(source_path: &str, window: &mut Window, cx: &mut App) {
    for action in asset_source_open_actions(source_path) {
        match action {
            AssetSourceOpenAction::AuthoredDocument => window.dispatch_action(
                Box::new(crate::actions::SelectAuthoredDocument {
                    document_id: source_path.to_string(),
                }),
                cx,
            ),
            AssetSourceOpenAction::AnimationCharacter => window.dispatch_action(
                Box::new(crate::actions::SelectAnimationCharacter {
                    character_glb: source_path.to_string(),
                }),
                cx,
            ),
            AssetSourceOpenAction::AnimationMotion => window.dispatch_action(
                Box::new(crate::actions::SelectAnimationMotion {
                    motion_glb: source_path.to_string(),
                }),
                cx,
            ),
            AssetSourceOpenAction::AnimationBlendSpace => window.dispatch_action(
                Box::new(crate::actions::SelectAnimationBlendSpace {
                    bspace_ron_path: source_path.to_string(),
                }),
                cx,
            ),
        }
    }
}

#[cfg(test)]
const fn asset_content_hash_label(content_hash: &str) -> &str {
    content_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_and_sorts_asset_entries_by_source_path() {
        let status = EditorAssetBrowserStatus::new(
            "session-a",
            Vec::new(),
            vec![
                AssetBrowserEntryData {
                    entry_id: 2,
                    workspace_id: 1,
                    asset_guid: "00000000-0000-0000-0000-000000000002".to_string(),
                    root_id: 10,
                    source_path: "textures/stone.png".to_string(),
                    schema_type: Some("az.test.Texture".to_string()),
                    content_hash: "b".repeat(32),
                    status: AssetBrowserEntryStatus::Clean,
                    diagnostics_count: 0,
                    latest_job: None,
                },
                AssetBrowserEntryData {
                    entry_id: 1,
                    workspace_id: 1,
                    asset_guid: "00000000-0000-0000-0000-000000000001".to_string(),
                    root_id: 11,
                    source_path: "materials/stone.mat.ron".to_string(),
                    schema_type: Some("az.test.Material".to_string()),
                    content_hash: "a".repeat(32),
                    status: AssetBrowserEntryStatus::Modified,
                    diagnostics_count: 1,
                    latest_job: Some(AssetBrowserJobData {
                        job_id: 8,
                        attempt_id: Some(9),
                        job_key: "default".to_string(),
                        platform: "pc".to_string(),
                        ordinal: Some(1),
                        status: AssetBrowserJobStatus::Failed,
                        error_count: 1,
                        warning_count: 0,
                    }),
                },
            ],
            None,
        );

        let entries = filtered_asset_entries(&status, None, "stone", None, None);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source_path, "materials/stone.mat.ron");
        assert_eq!(asset_file_name(&entries[0].source_path), "stone");
        assert_eq!(asset_file_name(&entries[1].source_path), "stone.png");
        assert_eq!(entries[0].status.label(), "modified");
        assert_eq!(
            entries[0].latest_job.as_ref().unwrap().status.label(),
            "failed"
        );
    }

    #[test]
    fn asset_identity_uses_catalog_type_instead_of_ron_extension() {
        let entry = test_asset_entry(1, 10, "prefabs/player.prefab.ron");
        let mut entry = entry;
        entry.schema_type = Some("azoth.prefab.Prefab".to_owned());
        let catalog = EditorAssetBuilderCatalog::new(
            Vec::new(),
            vec![AssetSourceSchemaData {
                schema_type: "azoth.prefab.Prefab".to_owned(),
                owner: "azoth.prefab".to_owned(),
                label: "Prefab".to_owned(),
                category: "Scene".to_owned(),
                authoring: AssetSourceSchemaAuthoringData::ProjectDocument {
                    schema_type: "azoth.prefab.Prefab".to_owned(),
                },
                file_templates: Vec::new(),
            }],
        );

        let identity = asset_type_identity(&entry, Some(&catalog), None);

        assert_eq!(asset_file_name(&entry.source_path), "player");
        assert_eq!(identity.label, "Prefab");
        assert_eq!(identity.kind, EditorTypeKind::Prefab);
        assert_ne!(identity.label.to_ascii_lowercase(), "ron");
    }

    #[test]
    fn asset_identity_uses_catalog_category_for_generic_graph_schema() {
        let mut entry = test_asset_entry(1, 10, "routes/surface.route");
        entry.schema_type = Some("azoth.graph.Document".to_owned());
        let catalog = EditorAssetBuilderCatalog::new(
            Vec::new(),
            vec![AssetSourceSchemaData {
                schema_type: "azoth.graph.Document".to_owned(),
                owner: "azoth.material".to_owned(),
                label: "Visual Graph".to_owned(),
                category: "Materials".to_owned(),
                authoring: AssetSourceSchemaAuthoringData::ProjectDocument {
                    schema_type: "azoth.graph.Document".to_owned(),
                },
                file_templates: Vec::new(),
            }],
        );

        let identity = asset_type_identity(&entry, Some(&catalog), None);

        assert_eq!(identity.label, "Visual Graph");
        assert_eq!(identity.kind, EditorTypeKind::Material);
    }

    #[test]
    fn wildcard_builder_patterns_resolve_serialized_source_type() {
        let builder = AssetBuilderData {
            name: "Material Builder".to_owned(),
            builder_guid: "builder".to_owned(),
            version: 1,
            patterns: vec![AssetBuilderPatternData {
                kind: AssetBuilderPatternKindData::Wildcard,
                pattern: "*.material.ron".to_owned(),
            }],
            source_schema_types: vec!["azoth.material.Material".to_owned()],
        };

        assert!(builder_matches_source(
            &builder,
            "materials/stone.material.ron"
        ));
        assert!(!builder_matches_source(
            &builder,
            "prefabs/stone.prefab.ron"
        ));
    }

    /// Two workspace roots carrying meshes, materials, nested material
    /// graphs and textures — the fixture the type-first tree test reads.
    fn typed_tree_fixture_status() -> EditorAssetBrowserStatus {
        EditorAssetBrowserStatus::new(
            "session-a",
            vec![
                WorkspaceRootData {
                    workspace_root_id: 1,
                    root_id: 10,
                    declared_root_id: "project.assets".to_string(),
                    owner_id: "local.project".to_string(),
                    source_root: "/wt/project/assets".to_string(),
                    display_name: "Project Assets".to_string(),
                    portable_key: "project:local.project:assets".to_string(),
                    output_prefix: "assets".to_string(),
                },
                WorkspaceRootData {
                    workspace_root_id: 2,
                    root_id: 11,
                    declared_root_id: "gem.azoth.physics.assets".to_string(),
                    owner_id: "azoth.physics".to_string(),
                    source_root: "/wt/project/gems/physics/assets".to_string(),
                    display_name: "Physics Assets".to_string(),
                    portable_key: "gem:azoth.physics:assets".to_string(),
                    output_prefix: "gems/azoth.physics".to_string(),
                },
            ],
            vec![
                test_asset_entry_with_schema(1, 10, "meshes/player.gltf", "azoth.mesh.SourceModel"),
                test_asset_entry_with_schema(
                    2,
                    10,
                    "materials/base.material.ron",
                    "azoth.material.Material",
                ),
                test_asset_entry_with_schema(
                    3,
                    10,
                    "materials/graphs/paint.azmat.ron",
                    "azoth.material.Graph",
                ),
                test_asset_entry_with_schema(
                    4,
                    11,
                    "materials/graphs/metal.azmat.ron",
                    "azoth.material.Graph",
                ),
                test_asset_entry_with_schema(
                    5,
                    11,
                    "materials/types/surface.azmaterialtype.ron",
                    "azoth.material.Type",
                ),
                test_asset_entry_with_schema(
                    6,
                    11,
                    "textures/metal.dds",
                    "azoth.texture.SourceImage",
                ),
            ],
            None,
        )
    }
    #[test]
    fn asset_browser_folders_build_type_first_tree_across_workspace_roots() {
        let status = typed_tree_fixture_status();

        let folders = asset_browser_folders(&status);

        assert_eq!(
            folders
                .iter()
                .map(|folder| (folder.name.as_str(), folder.count, folder.depth))
                .collect::<Vec<_>>(),
            vec![
                ("Meshes", 1, 0),
                ("Materials", 4, 0),
                ("graphs", 2, 1),
                ("types", 1, 1),
                ("Textures", 1, 0),
            ]
        );
        assert_eq!(folders[2].breadcrumb(), "Materials / graphs");
        assert!(!folders[0].has_children);
        assert!(folders[1].has_children);
        assert_eq!(folders[1].category_kind, EditorTypeKind::Material);

        let material_entries = filtered_asset_entries(&status, Some(&folders[1]), "", None, None)
            .into_iter()
            .map(|entry| entry.source_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            material_entries,
            vec![
                "materials/base.material.ron",
                "materials/graphs/metal.azmat.ron",
                "materials/graphs/paint.azmat.ron",
                "materials/types/surface.azmaterialtype.ron",
            ]
        );

        let graph_entries = filtered_asset_entries(&status, Some(&folders[2]), "", None, None)
            .into_iter()
            .map(|entry| entry.source_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            graph_entries,
            vec![
                "materials/graphs/metal.azmat.ron",
                "materials/graphs/paint.azmat.ron",
            ]
        );

        let mut collapsed = BTreeSet::new();
        collapsed.insert(folders[1].key.clone());
        assert_eq!(
            visible_asset_browser_folders(&folders, &collapsed)
                .map(|folder| folder.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Meshes", "Materials", "Textures"]
        );
    }

    #[test]
    fn asset_browser_categories_use_builder_resolution_for_generic_sources() {
        let status = EditorAssetBrowserStatus::new(
            "session-a",
            Vec::new(),
            vec![test_asset_entry_with_schema(
                1,
                10,
                "routes/paint.route",
                "azoth.asset.Source",
            )],
            None,
        );
        let catalog = EditorAssetBuilderCatalog::new(
            vec![AssetBuilderData {
                name: "Material Route Builder".to_owned(),
                builder_guid: "material-route".to_owned(),
                version: 1,
                patterns: vec![AssetBuilderPatternData {
                    kind: AssetBuilderPatternKindData::Wildcard,
                    pattern: "*.route".to_owned(),
                }],
                source_schema_types: vec!["azoth.material.Route".to_owned()],
            }],
            Vec::new(),
        );

        let folders = asset_browser_folders_with_types(&status, Some(&catalog), None);

        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].name, "Materials");
        assert_eq!(folders[0].count, 1);
        assert_eq!(folders[1].name, "routes");
        assert!(asset_browser_entry_matches_folder_with_types(
            &status.entries[0],
            &folders[0],
            Some(&catalog),
            None,
        ));
    }

    #[test]
    fn has_more_when_asset_status_contains_cursor() {
        let mut status = EditorAssetBrowserStatus::new("session-a", Vec::new(), Vec::new(), None);
        assert!(!asset_browser_has_more(&status));

        status.next_after_entry_id = Some(42);
        assert!(asset_browser_has_more(&status));
    }

    fn test_asset_entry(entry_id: i64, root_id: i64, source_path: &str) -> AssetBrowserEntryData {
        AssetBrowserEntryData {
            entry_id,
            workspace_id: 1,
            asset_guid: format!("00000000-0000-0000-0000-{entry_id:012}"),
            root_id,
            source_path: source_path.to_string(),
            schema_type: Some("az.test.Source".to_string()),
            content_hash: "a".repeat(64),
            status: AssetBrowserEntryStatus::Clean,
            diagnostics_count: 0,
            latest_job: None,
        }
    }

    fn test_asset_entry_with_schema(
        entry_id: i64,
        root_id: i64,
        source_path: &str,
        schema_type: &str,
    ) -> AssetBrowserEntryData {
        let mut entry = test_asset_entry(entry_id, root_id, source_path);
        entry.schema_type = Some(schema_type.to_string());
        entry
    }

    #[test]
    fn asset_builder_patterns_label_preserves_pattern_kinds() {
        let patterns = vec![
            AssetBuilderPatternData {
                kind: AssetBuilderPatternKindData::Wildcard,
                pattern: "*.prefab.ron".to_string(),
            },
            AssetBuilderPatternData {
                kind: AssetBuilderPatternKindData::Regex,
                pattern: r"^levels/.+\.scene\.ron$".to_string(),
            },
        ];

        assert_eq!(
            asset_builder_patterns_label(&patterns),
            r"wildcard:*.prefab.ron, regex:^levels/.+\.scene\.ron$"
        );
    }

    #[test]
    fn asset_builder_source_schema_label_lists_declared_schemas() {
        assert_eq!(
            asset_builder_source_schema_label(&[
                "az.test.Prefab".to_string(),
                "az.test.Scene".to_string()
            ]),
            "schemas: az.test.Prefab, az.test.Scene"
        );
        assert_eq!(asset_builder_source_schema_label(&[]), "schemas: any");
    }

    #[test]
    fn asset_source_schema_labels_make_authoring_workflow_visible() {
        let creatable = AssetSourceSchemaData {
            schema_type: "az.test.Prefab".to_string(),
            owner: "az-test".to_string(),
            label: "Prefab".to_string(),
            category: "Authoring".to_string(),
            authoring: AssetSourceSchemaAuthoringData::ProjectDocument {
                schema_type: "az.test.Prefab".to_string(),
            },
            file_templates: Vec::new(),
        };
        let imported = AssetSourceSchemaData {
            schema_type: "az.compat.LegacyMaterialSource".to_string(),
            owner: "legacy-materials".to_string(),
            label: String::new(),
            category: "Compatibility".to_string(),
            authoring: AssetSourceSchemaAuthoringData::File {
                workflow: AssetSourceFileWorkflowData {
                    source_root: "project:source-root".to_string(),
                    default_path_prefix: "materials".to_string(),
                    extensions: vec!["mtl".to_string()],
                    can_create: false,
                    can_edit: false,
                },
            },
            file_templates: vec![AssetSourceFileTemplateData {
                owner: "legacy-materials".to_string(),
                source_path: "materials/default.mtl".to_string(),
                label: "Default Material".to_string(),
                description: "Empty material".to_string(),
            }],
        };

        assert_eq!(asset_source_schema_title(&creatable), "Authoring / Prefab");
        assert_eq!(
            asset_source_schema_authoring_label(&creatable),
            "project document: az.test.Prefab"
        );
        assert_eq!(
            asset_source_schema_title(&imported),
            "Compatibility / LegacyMaterialSource"
        );
        assert_eq!(
            asset_source_schema_authoring_label(&imported),
            "import file: .mtl (legacy-materials)"
        );
        assert_eq!(
            asset_source_schema_catalog_label(&imported),
            "import file: .mtl (legacy-materials)"
        );
        assert!(visible_source_file_templates(&imported, "material").is_empty());
    }

    #[test]
    fn asset_source_open_actions_route_source_assets_to_existing_editor_views() {
        assert_eq!(
            asset_source_open_actions("prefabs/door.prefab.ron"),
            vec![AssetSourceOpenAction::AuthoredDocument]
        );
        assert_eq!(
            asset_source_open_actions("objects/props/crate.glb"),
            vec![AssetSourceOpenAction::AnimationCharacter]
        );
        assert_eq!(
            asset_source_open_actions("animations/walk.anim.glb"),
            vec![AssetSourceOpenAction::AnimationMotion]
        );
        assert_eq!(
            asset_source_open_actions("animations/locomotion/speed.bspace.ron"),
            vec![
                AssetSourceOpenAction::AuthoredDocument,
                AssetSourceOpenAction::AnimationBlendSpace,
            ]
        );
        assert_eq!(
            asset_source_open_actions("animations/locomotion/fullbody.comb.ron"),
            vec![
                AssetSourceOpenAction::AuthoredDocument,
                AssetSourceOpenAction::AnimationBlendSpace,
            ]
        );
        assert!(asset_source_open_actions("textures/albedo.png").is_empty());
    }

    #[test]
    fn asset_source_preview_resolves_file_roots_and_classifies_images() {
        let source_root = std::env::temp_dir().join("azoth-asset-browser/assets");
        let root = WorkspaceRootData {
            workspace_root_id: 1,
            root_id: 10,
            declared_root_id: "project.assets".to_string(),
            owner_id: "local.project".to_string(),
            source_root: source_root.to_string_lossy().into_owned(),
            display_name: "Project Assets".to_string(),
            portable_key: "project:local.project:assets".to_string(),
            output_prefix: "assets".to_string(),
        };
        let mut entry = test_asset_entry(1, 10, "textures/albedo.png");
        entry.schema_type = Some("azoth.texture.SourceImage".to_string());

        let preview = asset_source_preview_for_entry(&entry, Some(&root));

        assert_eq!(preview.preview_kind, AssetSourcePreviewKind::Image);
        assert_eq!(
            preview.source_root.as_deref(),
            Some(root.source_root.as_str())
        );
        let preview_path = preview
            .locator
            .as_ref()
            .map(AssetSourcePreviewLocator::source_path)
            .expect("file-backed preview path");
        let expected_path = source_root.join("textures/albedo.png");
        assert_eq!(std::path::Path::new(&preview_path), expected_path);
        assert_eq!(
            preview.schema_type.as_deref(),
            Some("azoth.texture.SourceImage")
        );

        assert_eq!(
            asset_source_preview_kind("animations/walk.anim.glb"),
            AssetSourcePreviewKind::Motion
        );
        assert_eq!(
            asset_source_preview_kind("objects/crate.glb"),
            AssetSourcePreviewKind::Model
        );
        assert_eq!(
            asset_source_preview_kind("materials/stone.material.ron"),
            AssetSourcePreviewKind::Document
        );
    }

    #[test]
    fn source_file_templates_are_search_filtered_before_rendering() {
        let source_schema = AssetSourceSchemaData {
            schema_type: "azoth.gamedata.TableSource".to_string(),
            owner: "gamedata".to_string(),
            label: "GameData Table".to_string(),
            category: "GameData".to_string(),
            authoring: AssetSourceSchemaAuthoringData::File {
                workflow: AssetSourceFileWorkflowData {
                    source_root: "project:source-root".to_string(),
                    default_path_prefix: "gamedata/tables".to_string(),
                    extensions: vec!["ron".to_string()],
                    can_create: true,
                    can_edit: true,
                },
            },
            file_templates: vec![
                AssetSourceFileTemplateData {
                    owner: "sample-project".to_string(),
                    source_path: "data/achievements.ron".to_string(),
                    label: "achievements".to_string(),
                    description: "Create an empty table".to_string(),
                },
                AssetSourceFileTemplateData {
                    owner: "sample-project".to_string(),
                    source_path: "data/abilities.ron".to_string(),
                    label: String::new(),
                    description: "Create an empty table".to_string(),
                },
            ],
        };

        assert!(visible_source_file_templates(&source_schema, "").is_empty());
        let visible = visible_source_file_templates(&source_schema, "achievement");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].source_path, "data/achievements.ron");
        assert_eq!(
            asset_source_file_template_label(&source_schema.file_templates[1]),
            "abilities.ron"
        );
    }

    #[test]
    fn asset_content_hash_label_preserves_full_hash() {
        let hash = format!("{}{}", "a".repeat(32), "b".repeat(32));

        assert_eq!(asset_content_hash_label(&hash), hash.as_str());
        assert_eq!(asset_content_hash_label(&hash).len(), 64);
    }
}
