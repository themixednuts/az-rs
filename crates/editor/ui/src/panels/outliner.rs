//! Authored object outliner.
//!
//! This panel renders editor-owned authored selection data supplied by
//! project-host. It deliberately does not inspect a runtime world or keep
//! a placeholder ECS entity hierarchy in the universal editor UI.

use std::collections::{BTreeMap, BTreeSet};

use az_editor_inspector::{
    ReflectedEntityInspection, ReflectedInspectionChild, ReflectedInspectionField,
    ReflectedValueNode,
};
use az_proto_project::vnext::PrefabEditCommand;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Global, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Render, StatefulInteractiveElement,
    Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::dock::Panel;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use super::inspector::EditorReflectedSelectionState;
use crate::panels::{EditorLayerVisibility, authored_object_visibility_key, kit};
use crate::type_iconography::{EditorTypeKind, entity_kind};

/// Project-host authored document/object outline available to UI panels.
#[derive(Debug, Clone)]
pub struct EditorAuthoredOutline {
    pub data: AuthoredOutlineData,
    pub status_error: Option<String>,
}

impl EditorAuthoredOutline {
    #[must_use]
    pub const fn new(data: AuthoredOutlineData) -> Self {
        Self {
            data,
            status_error: None,
        }
    }

    #[must_use]
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            data: AuthoredOutlineData {
                documents: Vec::new(),
            },
            status_error: Some(error.into()),
        }
    }

    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.status_error = Some(error.into());
        self
    }
}

impl Global for EditorAuthoredOutline {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredOutlineData {
    pub documents: Vec<AuthoredDocumentOutlineData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredDocumentOutlineData {
    pub document_id: String,
    pub source_path: String,
    pub schema_type: String,
    pub revision: u64,
    pub saved_revision: Option<u64>,
    pub unsaved_changes: bool,
    pub object_count: u32,
    pub journal_entry_count: u32,
    pub loaded: bool,
    pub valid: bool,
    pub diagnostic: String,
    pub objects: Vec<AuthoredObjectOutlineData>,
}

/// Engine-authored scene and prefab structural schemas.
pub const ENGINE_SCENE_ROOT_SCHEMA_TYPE: &str = "azoth.scene.Scene";
pub const ENGINE_PREFAB_ROOT_SCHEMA_TYPE: &str = "azoth.prefab.Prefab";
pub const ENGINE_PREFAB_ENTITY_SCHEMA_TYPE: &str = "azoth.prefab.Entity";
pub const ENGINE_PREFAB_INSTANCE_SCHEMA_TYPE: &str = "azoth.prefab.Instance";

#[must_use]
pub fn is_scene_document_schema(schema_type: &str) -> bool {
    schema_type == ENGINE_SCENE_ROOT_SCHEMA_TYPE
}

#[must_use]
pub fn is_scene_document(document: &AuthoredDocumentOutlineData) -> bool {
    is_scene_document_schema(&document.schema_type)
}

/// Project-scoped level authority shared by hierarchy, titlebar, and viewport.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorActiveLevel {
    pub document_id: Option<String>,
}

impl EditorActiveLevel {
    #[must_use]
    pub const fn new(document_id: Option<String>) -> Self {
        Self { document_id }
    }
}

impl Global for EditorActiveLevel {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredObjectOutlineData {
    pub object_id: String,
    pub schema_type: String,
    pub selected: bool,
    pub display_name: Option<String>,
    pub prefab_parent_entity_object_id: Option<String>,
    pub prefab_component_object_ids: Vec<String>,
    pub prefab_owner_entity_object_id: Option<String>,
    /// Referenced prefab source for a scene root or placed prefab instance.
    pub prefab_source_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatableAuthoredSchemaData {
    pub schema_type: String,
    pub label: String,
    pub category: Option<String>,
    /// Schema-declared editor icon. Presentation never infers this from the
    /// schema name.
    pub icon: Option<String>,
    /// Project Host-published composition metadata for an attachable
    /// component. Kept as UI data so menus can disable illegal additions.
    pub component_capabilities: Option<ComponentCapabilityData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentCapabilityData {
    pub provides: Vec<String>,
    pub requires: Vec<String>,
    pub incompatible: Vec<String>,
}

/// Editor-owned authored document creation options available to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCreatableAuthoredSchemas {
    pub schemas: Vec<CreatableAuthoredSchemaData>,
}

impl EditorCreatableAuthoredSchemas {
    #[must_use]
    pub const fn new(schemas: Vec<CreatableAuthoredSchemaData>) -> Self {
        Self { schemas }
    }
}

impl Global for EditorCreatableAuthoredSchemas {}

/// Authored object outliner panel.
///
/// Displays the currently selected authored document/object projection from
/// project-host. The selection controller in `az-editor` owns loading and
/// mutation; this UI crate only renders the data it receives.
pub struct AuthoredOutliner {
    filter_input: Entity<InputState>,
    filter: String,
    collapsed: BTreeSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl AuthoredOutliner {
    pub const NAME: &'static str = "outliner";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search hierarchy..."));
        let subscriptions = vec![cx.subscribe_in(&filter_input, window, Self::on_filter_input)];
        Self {
            filter_input,
            filter: String::new(),
            collapsed: BTreeSet::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Set search filter text.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
    }

    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
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
}

/// Borrowed stand-ins for absent globals, so a repaint never allocates an
/// empty collection just to have something to reference.
static NO_HIDDEN_KEYS: BTreeSet<String> = BTreeSet::new();
const NO_CREATABLE_SCHEMAS: &[CreatableAuthoredSchemaData] = &[];

impl Render for AuthoredOutliner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(reason) = crate::panels::editor_project_host_failed_reason(cx) {
            return crate::panels::render_project_host_failed_placeholder(
                "Authored Outliner",
                &reason,
                cx,
            )
            .into_any_element();
        }
        if crate::panels::editor_project_host_connecting(cx) {
            return crate::panels::render_project_host_connecting_placeholder(
                "Authored Outliner",
                cx,
            )
            .into_any_element();
        }
        self.sync_filter_input(window, cx);
        let outline = cx.try_global::<EditorAuthoredOutline>();
        let status_error = outline.and_then(|global| global.status_error.as_deref());
        let outline = outline.map(|global| &global.data);
        let inspection = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current);
        let active_level_document_id = cx
            .try_global::<EditorActiveLevel>()
            .and_then(|active| active.document_id.as_deref());
        let creatable_schemas = cx
            .try_global::<EditorCreatableAuthoredSchemas>()
            .map_or(NO_CREATABLE_SCHEMAS, |global| global.schemas.as_slice());
        let hidden = cx
            .try_global::<EditorLayerVisibility>()
            .map_or(&NO_HIDDEN_KEYS, |visibility| &visibility.hidden);
        let theme = cx.theme();
        let footer = outline_footer_summary(outline, active_level_document_id, hidden);

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.sidebar)
            .child(
                kit::search_row(theme)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_filter_input))
                    .child(
                        div().flex_1().child(
                            Input::new(&self.filter_input)
                                .small()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                    )
                    .child(
                        Icon::new(IconName::FilterList)
                            .with_size(px(15.0))
                            .text_color(theme.muted_foreground),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .child(render_creatable_schema_strip(creatable_schemas, theme)),
            )
            .when_some(status_error, |this, error| {
                this.child(
                    div()
                        .w_full()
                        .flex_none()
                        .child(render_authored_outline_error(error, theme)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(render_outline_body(
                        outline,
                        active_level_document_id,
                        &self.filter,
                        OutlineContext {
                            all_documents: outline.map_or(&[], |outline| &outline.documents),
                            inspection,
                            hidden,
                            collapsed: &self.collapsed,
                            outliner: &cx.entity().downgrade(),
                            theme,
                        },
                    )),
            )
            .child(kit::count_footer(theme).child(footer))
            .into_any_element()
    }
}

/// Footer summary string for the outliner, e.g. `14 entities · 12 visible`.
fn outline_footer_summary(
    outline: Option<&AuthoredOutlineData>,
    active_level_document_id: Option<&str>,
    hidden: &BTreeSet<String>,
) -> String {
    let Some(outline) = outline else {
        return "no authored outline".to_string();
    };
    let Some(level) = active_level_document(outline, active_level_document_id) else {
        return "0 entities · 0 visible".to_string();
    };
    let Some(root_prefab) = level_root_prefab_document(outline, level) else {
        return "0 entities · 0 visible".to_string();
    };
    let mut source_chain = BTreeSet::new();
    let (entity_count, visible_count) =
        count_prefab_entities(outline, root_prefab, hidden, &mut source_chain);
    format!("{entity_count} entities · {visible_count} visible")
}

impl Focusable for AuthoredOutliner {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter_input.read(cx).focus_handle(cx)
    }
}

impl Panel for AuthoredOutliner {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("account_tree"), "Hierarchy", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &gpui::App) -> bool {
        false
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Vec<gpui_component::button::Button>> {
        use gpui_component::button::{Button, ButtonVariants as _};
        let inspection = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current);
        let mut buttons = Vec::new();
        if let Some(parent_alias) = selected_prefab_entity_add_target(inspection) {
            buttons.push(
                Button::new("authored-add-prefab-entity")
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .tooltip("Add prefab entity")
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::actions::ApplyReflectedPrefabEdit {
                                command: PrefabEditCommand::AddEntity {
                                    alias: uuid::Uuid::now_v7().to_string(),
                                    parent_alias: Some(parent_alias.clone()),
                                },
                            }),
                            cx,
                        );
                    }),
            );
        }
        if let Some(entity_alias) = selected_prefab_entity_remove_target(inspection) {
            buttons.push(
                Button::new("authored-remove-prefab-entity")
                    .icon(IconName::Delete)
                    .ghost()
                    .small()
                    .tooltip("Remove prefab entity")
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::actions::ApplyReflectedPrefabEdit {
                                command: PrefabEditCommand::RemoveEntity {
                                    alias: entity_alias.clone(),
                                },
                            }),
                            cx,
                        );
                    }),
            );
        }
        buttons.extend([
            Button::new("authored-refresh-reflected-inspection")
                .icon(IconName::Inbox)
                .ghost()
                .small()
                .tooltip("Refresh reflected inspection")
                .on_click(|_, window, cx| {
                    window
                        .dispatch_action(Box::new(crate::actions::RefreshReflectedInspection), cx);
                }),
            Button::new("authored-refresh-outline")
                .icon(IconName::Replace)
                .ghost()
                .small()
                .tooltip("Refresh outline")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::RefreshAuthoredOutline), cx);
                }),
        ]);
        Some(buttons)
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for AuthoredOutliner {}

/// The read-only context every authored-outline renderer threads through: the
/// documents it may follow a prefab reference into, the current reflected
/// selection, the visibility and collapse sets, the panel handle its carets
/// talk back to, and the theme it paints in.
#[derive(Clone, Copy)]
struct OutlineContext<'a> {
    all_documents: &'a [AuthoredDocumentOutlineData],
    inspection: Option<&'a ReflectedEntityInspection>,
    hidden: &'a BTreeSet<String>,
    collapsed: &'a BTreeSet<String>,
    outliner: &'a gpui::WeakEntity<AuthoredOutliner>,
    theme: &'a gpui_component::theme::Theme,
}

/// Mutable state carried down one prefab walk: which objects have already been
/// emitted, and which prefab sources are on the current instance chain, so a
/// cyclic reference stops instead of recursing.
struct PrefabWalk {
    rendered: BTreeSet<String>,
    instance_chain: BTreeSet<String>,
}

fn render_outline_body(
    outline: Option<&AuthoredOutlineData>,
    active_level_document_id: Option<&str>,
    filter: &str,
    ctx: OutlineContext<'_>,
) -> gpui::Div {
    let Some(active_level_document_id) = active_level_document_id else {
        return render_empty_state("No level open — open one from the level menu", ctx.theme);
    };
    match outline {
        Some(outline)
            if active_level_has_matches(
                outline,
                active_level_document_id,
                ctx.inspection,
                filter,
            ) =>
        {
            render_authored_outline(outline, active_level_document_id, ctx)
        }
        Some(outline)
            if active_level_document(outline, Some(active_level_document_id)).is_some() =>
        {
            render_empty_state("No matching entity in the active level", ctx.theme)
        }
        Some(_) => render_empty_state("No level open — open one from the level menu", ctx.theme),
        None => render_empty_state("No authored outline loaded", ctx.theme),
    }
}

fn selected_prefab_entity_add_target(
    inspection: Option<&ReflectedEntityInspection>,
) -> Option<String> {
    Some(inspection?.selection.entity_alias.clone())
}

fn selected_prefab_entity_remove_target(
    inspection: Option<&ReflectedEntityInspection>,
) -> Option<String> {
    Some(inspection?.selection.entity_alias.clone())
}

fn render_authored_outline_error(error: &str, theme: &gpui_component::theme::Theme) -> gpui::Div {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .bg(theme.danger.opacity(0.10))
        .border_b_1()
        .border_color(theme.border)
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(px(14.0))
                .text_color(theme.danger),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.0))
                .text_color(theme.foreground)
                .child(format!("Project-host authored data error: {error}")),
        )
}

#[cfg(test)]
fn authored_outline_has_matches(data: &AuthoredOutlineData, filter: &str) -> bool {
    data.documents
        .iter()
        .any(|document| authored_document_matches(document, filter))
}

#[must_use]
pub fn active_level_document<'a>(
    outline: &'a AuthoredOutlineData,
    active_level_document_id: Option<&str>,
) -> Option<&'a AuthoredDocumentOutlineData> {
    let document_id = active_level_document_id?;
    outline.documents.iter().find(|document| {
        document.document_id == document_id
            && document.loaded
            && document.valid
            && is_scene_document(document)
    })
}

fn referenced_prefab_document<'a>(
    outline: &'a AuthoredOutlineData,
    source_path: &str,
) -> Option<&'a AuthoredDocumentOutlineData> {
    referenced_prefab_document_in(&outline.documents, source_path)
}

fn referenced_prefab_document_in<'a>(
    documents: &'a [AuthoredDocumentOutlineData],
    source_path: &str,
) -> Option<&'a AuthoredDocumentOutlineData> {
    documents.iter().find(|candidate| {
        candidate.schema_type == ENGINE_PREFAB_ROOT_SCHEMA_TYPE
            && (candidate.document_id == source_path || candidate.source_path == source_path)
    })
}

fn level_root_prefab_document<'a>(
    outline: &'a AuthoredOutlineData,
    level: &AuthoredDocumentOutlineData,
) -> Option<&'a AuthoredDocumentOutlineData> {
    level_root_prefab_document_in(&outline.documents, level)
}

fn level_root_prefab_document_in<'a>(
    documents: &'a [AuthoredDocumentOutlineData],
    level: &AuthoredDocumentOutlineData,
) -> Option<&'a AuthoredDocumentOutlineData> {
    let source_path = level
        .objects
        .iter()
        .find(|object| object.schema_type == ENGINE_SCENE_ROOT_SCHEMA_TYPE)?
        .prefab_source_path
        .as_deref()?;
    referenced_prefab_document_in(documents, source_path)
}

/// Prefab documents that contribute runtime entities to the active level.
/// The scene root is first, followed by unique nested prefab dependencies.
#[must_use]
pub fn active_level_prefab_documents<'a>(
    outline: &'a AuthoredOutlineData,
    active_level_document_id: Option<&str>,
) -> Vec<&'a AuthoredDocumentOutlineData> {
    let Some(level) = active_level_document(outline, active_level_document_id) else {
        return Vec::new();
    };
    let Some(root) = level_root_prefab_document(outline, level) else {
        return Vec::new();
    };
    let mut documents = Vec::new();
    let mut visited = BTreeSet::new();
    collect_prefab_documents(outline, root, &mut visited, &mut documents);
    documents
}

fn collect_prefab_documents<'a>(
    outline: &'a AuthoredOutlineData,
    document: &'a AuthoredDocumentOutlineData,
    visited: &mut BTreeSet<String>,
    documents: &mut Vec<&'a AuthoredDocumentOutlineData>,
) {
    if !visited.insert(document.document_id.clone()) {
        return;
    }
    documents.push(document);
    for source_path in document
        .objects
        .iter()
        .filter_map(|object| object.prefab_source_path.as_deref())
    {
        if let Some(source) = referenced_prefab_document(outline, source_path) {
            collect_prefab_documents(outline, source, visited, documents);
        }
    }
}

fn active_level_has_matches(
    outline: &AuthoredOutlineData,
    active_level_document_id: &str,
    inspection: Option<&ReflectedEntityInspection>,
    filter: &str,
) -> bool {
    let Some(level) = active_level_document(outline, Some(active_level_document_id)) else {
        return false;
    };
    authored_document_matches(level, filter) || {
        let prefab_documents =
            active_level_prefab_documents(outline, Some(active_level_document_id));
        prefab_documents
            .iter()
            .any(|document| authored_document_matches(document, filter))
            || inspection.is_some_and(|inspection| {
                prefab_documents.iter().any(|document| {
                    document.document_id == inspection.selection.source_path
                        || document.source_path == inspection.selection.source_path
                }) && reflected_selection_matches(inspection, filter)
            })
    }
}

fn count_prefab_entities(
    outline: &AuthoredOutlineData,
    document: &AuthoredDocumentOutlineData,
    hidden: &BTreeSet<String>,
    source_chain: &mut BTreeSet<String>,
) -> (usize, usize) {
    if !source_chain.insert(document.source_path.clone()) {
        return (0, 0);
    }
    let mut entities = 0;
    let mut visible = 0;
    for entity in document
        .objects
        .iter()
        .filter(|object| object.schema_type == ENGINE_PREFAB_ENTITY_SCHEMA_TYPE)
    {
        entities += 1;
        let object_key = authored_object_visibility_key(&document.document_id, &entity.object_id);
        if !hidden.contains(&document.document_id) && !hidden.contains(&object_key) {
            visible += 1;
        }
    }
    for source_path in document
        .objects
        .iter()
        .filter_map(|object| object.prefab_source_path.as_deref())
    {
        if let Some(source) = referenced_prefab_document(outline, source_path) {
            let nested = count_prefab_entities(outline, source, hidden, source_chain);
            entities += nested.0;
            visible += nested.1;
        }
    }
    source_chain.remove(&document.source_path);
    (entities, visible)
}

/// Case-insensitive substring test that allocates nothing.
///
/// ASCII case folding is a byte-for-byte permutation, so a windowed
/// `eq_ignore_ascii_case` scan accepts exactly the strings a lowercased-copy
/// `contains` would; non-ASCII bytes are untouched by both and compare equal.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    needle.is_empty()
        || haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

fn authored_document_matches(document: &AuthoredDocumentOutlineData, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }
    contains_ignore_ascii_case(&document.document_id, filter)
        || contains_ignore_ascii_case(&document.source_path, filter)
        || contains_ignore_ascii_case(&document.schema_type, filter)
        || contains_ignore_ascii_case(&document.diagnostic, filter)
        || document.objects.iter().any(|object| {
            contains_ignore_ascii_case(&object.object_id, filter)
                || contains_ignore_ascii_case(&object.schema_type, filter)
                || object
                    .display_name
                    .as_deref()
                    .is_some_and(|name| contains_ignore_ascii_case(name, filter))
                || object
                    .prefab_parent_entity_object_id
                    .as_deref()
                    .is_some_and(|parent| contains_ignore_ascii_case(parent, filter))
                || object
                    .prefab_owner_entity_object_id
                    .as_deref()
                    .is_some_and(|owner| contains_ignore_ascii_case(owner, filter))
                || object
                    .prefab_component_object_ids
                    .iter()
                    .any(|component| contains_ignore_ascii_case(component, filter))
        })
}

fn reflected_selection_matches(data: &ReflectedEntityInspection, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }
    contains_ignore_ascii_case(&data.selection.source_path, filter)
        || contains_ignore_ascii_case(&data.selection.entity_alias, filter)
        || data.components.iter().any(|component| {
            contains_ignore_ascii_case(&component.component.type_path, filter)
                || contains_ignore_ascii_case(&component.model.type_label, filter)
                || component
                    .model
                    .category
                    .as_deref()
                    .is_some_and(|category| contains_ignore_ascii_case(category, filter))
                || component
                    .model
                    .fields
                    .iter()
                    .any(|field| reflected_field_matches(field, filter))
        })
}

fn reflected_field_matches(field: &ReflectedInspectionField, filter: &str) -> bool {
    contains_ignore_ascii_case(&field.name, filter)
        || contains_ignore_ascii_case(&field.label, filter)
        || field
            .description
            .as_deref()
            .is_some_and(|description| contains_ignore_ascii_case(description, filter))
        || reflected_node_matches(&field.value, filter)
}

fn reflected_node_matches(node: &ReflectedValueNode, filter: &str) -> bool {
    contains_ignore_ascii_case(&node.type_path, filter)
        || node
            .children
            .iter()
            .any(|child| reflected_child_matches(child, filter))
}

fn reflected_child_matches(child: &ReflectedInspectionChild, filter: &str) -> bool {
    match child {
        ReflectedInspectionChild::Field(field) => reflected_field_matches(field, filter),
        ReflectedInspectionChild::TupleElement { value, .. }
        | ReflectedInspectionChild::OptionalSome(value) => reflected_node_matches(value, filter),
        ReflectedInspectionChild::ListItem(item) => reflected_node_matches(&item.value, filter),
        ReflectedInspectionChild::MapEntry(entry) => reflected_node_matches(&entry.value, filter),
        ReflectedInspectionChild::Variant(variant) => {
            contains_ignore_ascii_case(&variant.name, filter)
                || variant
                    .fields
                    .iter()
                    .any(|field| reflected_child_matches(field, filter))
        }
    }
}

fn render_authored_outline(
    data: &AuthoredOutlineData,
    active_level_document_id: &str,
    ctx: OutlineContext<'_>,
) -> gpui::Div {
    let mut documents = v_flex().w_full().py(px(3.0));
    if let Some(document) = active_level_document(data, Some(active_level_document_id)) {
        documents = documents.child(render_authored_document_outline(document, ctx));
    }

    div().w_full().child(documents)
}

fn render_creatable_schema_strip(
    entries: &[CreatableAuthoredSchemaData],
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    if entries.is_empty() {
        return div().flex_none().into_any_element();
    }

    let mut row = h_flex()
        .w_full()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(5.0))
        .bg(theme.sidebar)
        .border_b_1()
        .border_color(theme.border)
        .child(
            Icon::new(IconName::Plus)
                .with_size(px(14.0))
                .text_color(theme.muted_foreground),
        )
        .child(kit::section_label("New", theme));

    for entry in entries {
        row = row.child(render_creatable_schema_button(entry, theme));
    }

    row.overflow_x_scrollbar().into_any_element()
}

fn render_creatable_schema_button(
    entry: &CreatableAuthoredSchemaData,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let root_schema = entry.schema_type.clone();
    let label = entry.label.clone();
    let element_id = gpui::SharedString::from(format!(
        "authored-create-{}",
        schema_element_key(&entry.schema_type)
    ));

    div()
        .id(element_id)
        .flex_none()
        .px_2()
        .py(px(2.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .hover(|this| this.bg(theme.secondary_hover).border_color(theme.accent))
        .cursor_pointer()
        .text_size(px(11.0))
        .whitespace_nowrap()
        .text_color(theme.foreground)
        .child(label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::CreateAuthoredDocument {
                    root_schema: root_schema.clone(),
                }),
                cx,
            );
        })
}

fn render_authored_document_outline(
    document: &AuthoredDocumentOutlineData,
    ctx: OutlineContext<'_>,
) -> gpui::Div {
    let theme = ctx.theme;
    let root_prefab = level_root_prefab_document_in(ctx.all_documents, document);
    let element_id =
        gpui::SharedString::from(format!("authored-outline-doc-{}", document.document_id));
    let visibility_document_id = root_prefab.map_or_else(
        || document.document_id.clone(),
        |prefab| prefab.document_id.clone(),
    );
    let visible = !ctx.hidden.contains(&visibility_document_id);
    let visibility_tooltip = if visible { "Hide level" } else { "Show level" };
    let document_kind = EditorTypeKind::Level;
    let icon_color = if document.valid {
        document_kind.tint(ctx.theme)
    } else {
        ctx.theme.danger
    };
    let collapse_key = format!("document:{}", document.document_id);
    let has_children = root_prefab.is_some_and(|prefab| !prefab.objects.is_empty());
    let open = has_children && !ctx.collapsed.contains(&collapse_key);

    let header = render_document_outline_header(
        document,
        element_id,
        document_kind,
        icon_color,
        DocumentOutlineHeaderState {
            visible,
            visibility_tooltip,
            has_children,
            open,
        },
        ctx,
    );

    let mut block = v_flex().w_full().child(header);

    if !open {
        return block;
    }
    if let Some(root_prefab) = root_prefab {
        block = block.child(render_prefab_document_outline(root_prefab, ctx));
    } else {
        let root_prefab_path = document
            .objects
            .iter()
            .find(|object| object.schema_type == ENGINE_SCENE_ROOT_SCHEMA_TYPE)
            .and_then(|object| object.prefab_source_path.as_deref())
            .filter(|path| !path.trim().is_empty());
        block = block.child(render_empty_state(
            root_prefab_path.map_or_else(
                || "This level has no root prefab".to_owned(),
                |path| format!("Root prefab is not loaded: {path}"),
            ),
            theme,
        ));
    }

    block
}

/// The parts of a level header row that come from the collapse and
/// visibility sets rather than from the document itself.
#[derive(Clone, Copy)]
struct DocumentOutlineHeaderState {
    visible: bool,
    visibility_tooltip: &'static str,
    has_children: bool,
    open: bool,
}

/// The level row at the top of the outline: caret, level icon, name, status
/// badge, and the eye that toggles the whole level's visibility.
fn render_document_outline_header(
    document: &AuthoredDocumentOutlineData,
    element_id: gpui::SharedString,
    document_kind: EditorTypeKind,
    icon_color: gpui::Hsla,
    state: DocumentOutlineHeaderState,
    ctx: OutlineContext<'_>,
) -> impl IntoElement {
    let theme = ctx.theme;
    let document_id = document.document_id.clone();
    let visibility_document_id = level_root_prefab_document_in(ctx.all_documents, document)
        .map_or_else(
            || document.document_id.clone(),
            |prefab| prefab.document_id.clone(),
        );
    let caret_key = format!("document:{}", document.document_id);
    let caret_outliner = ctx.outliner.clone();
    let DocumentOutlineHeaderState {
        visible,
        visibility_tooltip,
        has_children,
        open,
    } = state;
    kit::list_row(element_id, theme, false)
        .h(px(23.0))
        .pl(px(6.0))
        .child(
            kit::row_caret(has_children.then_some(open), theme)
                .id(format!("authored-document-caret-{}", document.document_id))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    let _ = caret_outliner.update(cx, |this, cx| {
                        if !this.collapsed.insert(caret_key.clone()) {
                            this.collapsed.remove(&caret_key);
                        }
                        cx.notify();
                    });
                }),
        )
        .child(kit::row_icon(document_kind.icon(), icon_color))
        .child(kit::row_name(
            document
                .objects
                .iter()
                .find(|object| object.schema_type == ENGINE_SCENE_ROOT_SCHEMA_TYPE)
                .and_then(|object| object.display_name.clone())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| {
                    let schema_label =
                        crate::naming::schema_display_name(&document.schema_type, None);
                    crate::naming::document_display_name(
                        &document.source_path,
                        schema_label.as_ref(),
                    )
                    .into_owned()
                }),
            theme,
        ))
        .when_some(authored_document_status(document, theme), |this, status| {
            this.child(kit::status_dot_with_tooltip(
                format!("authored-document-status-{}", document.document_id),
                status.tint,
                status.tooltip,
            ))
        })
        .child(render_document_visibility_toggle(
            document,
            visible,
            visibility_tooltip,
            visibility_document_id,
            theme,
        ))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::SelectAuthoredDocument {
                    document_id: document_id.clone(),
                }),
                cx,
            );
        })
}

/// The eye on a level row: toggles the whole level's visibility through
/// [`crate::actions::SetLayerVisibility`].
fn render_document_visibility_toggle(
    document: &AuthoredDocumentOutlineData,
    visible: bool,
    visibility_tooltip: &'static str,
    visibility_document_id: String,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .id(format!(
            "authored-document-visibility-{}",
            document.document_id
        ))
        .flex_none()
        .size(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .text_color(if visible {
            theme.muted_foreground
        } else {
            theme.muted_foreground.opacity(0.55)
        })
        .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
        .cursor_pointer()
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(visibility_tooltip).build(window, cx)
        })
        .child(
            Icon::new(if visible {
                IconName::Eye
            } else {
                IconName::EyeOff
            })
            .with_size(px(14.0)),
        )
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::SetLayerVisibility {
                    document_id: visibility_document_id.clone(),
                    visible: !visible,
                }),
                cx,
            );
        })
}

/// The disclosure caret an outline row shows. `None` where the row has no
/// children; the key is the collapse-set entry the click toggles.
struct OutlineCaret {
    open: bool,
    collapse_key: String,
}

fn render_authored_object_outline(
    document_id: &str,
    object: &AuthoredObjectOutlineData,
    selected: bool,
    depth: usize,
    caret: Option<OutlineCaret>,
    kind: EditorTypeKind,
    ctx: OutlineContext<'_>,
) -> impl IntoElement {
    let theme = ctx.theme;
    let document_id = document_id.to_string();
    let object_id = object.object_id.clone();
    let element_id = gpui::SharedString::from(format!("authored-outline-object-{object_id}"));
    let visibility_key = authored_object_visibility_key(&document_id, &object_id);
    let visible = !ctx.hidden.contains(&visibility_key);
    let visibility_document_id = document_id.clone();
    let visibility_object_id = object_id.clone();
    let visibility_tooltip = if visible {
        "Hide object"
    } else {
        "Show object"
    };

    let caret = if let Some(OutlineCaret { open, collapse_key }) = caret {
        let outliner = ctx.outliner.clone();
        kit::row_caret(Some(open), theme)
            .id(format!("authored-object-caret-{collapse_key}"))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let _ = outliner.update(cx, |this, cx| {
                    if !this.collapsed.insert(collapse_key.clone()) {
                        this.collapsed.remove(&collapse_key);
                    }
                    cx.notify();
                });
            })
            .into_any_element()
    } else {
        kit::row_caret(None, theme).into_any_element()
    };

    kit::list_row(element_id, theme, selected)
        .h(px(23.0))
        .opacity(if visible { 1.0 } else { 0.48 })
        .pl(prefab_outline_indent(depth))
        .child(caret)
        .child(kit::row_icon(kind.icon(), kind.tint(theme)))
        .child(kit::row_name(outline_object_label(object), theme))
        .child(
            div()
                .id(format!("authored-outline-visibility-{visibility_key}"))
                .flex_none()
                .size(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_color(if visible {
                    theme.muted_foreground
                } else {
                    theme.muted_foreground.opacity(0.55)
                })
                .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(visibility_tooltip).build(window, cx)
                })
                .child(
                    Icon::new(if visible {
                        IconName::Eye
                    } else {
                        IconName::EyeOff
                    })
                    .with_size(px(14.0)),
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(crate::actions::SetAuthoredObjectVisibility {
                            document_id: visibility_document_id.clone(),
                            object_id: visibility_object_id.clone(),
                            visible: !visible,
                        }),
                        cx,
                    );
                }),
        )
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::SelectAuthoredObject {
                    document_id: document_id.clone(),
                    object_id: object_id.clone(),
                }),
                cx,
            );
        })
}

fn reflected_entity_is_selected(
    document: &AuthoredDocumentOutlineData,
    object: &AuthoredObjectOutlineData,
    inspection: Option<&ReflectedEntityInspection>,
) -> bool {
    object.schema_type == ENGINE_PREFAB_ENTITY_SCHEMA_TYPE
        && inspection.is_some_and(|inspection| {
            (inspection.selection.source_path == document.document_id
                || inspection.selection.source_path == document.source_path)
                && inspection.selection.entity_alias == object.object_id
        })
}

fn render_prefab_document_outline(
    document: &AuthoredDocumentOutlineData,
    ctx: OutlineContext<'_>,
) -> gpui::Div {
    let index = PrefabOutlineIndex::new(document);
    let mut walk = PrefabWalk {
        rendered: BTreeSet::new(),
        instance_chain: BTreeSet::from([document.source_path.clone()]),
    };
    let mut block = v_flex().w_full();

    for entity_id in &index.root_entity_ids {
        block = render_prefab_entity_tree(block, document, &index, entity_id, 1, ctx, &mut walk);
    }

    for object in &document.objects {
        if walk.rendered.contains(&object.object_id) || is_prefab_root_object(object) {
            continue;
        }
        if object.prefab_owner_entity_object_id.is_some() {
            walk.rendered.insert(object.object_id.clone());
            continue;
        }
        if object.schema_type == ENGINE_PREFAB_INSTANCE_SCHEMA_TYPE {
            block = render_prefab_instance_tree(block, document, object, 1, ctx, &mut walk);
        } else {
            block = block.child(render_authored_object_outline(
                &document.document_id,
                object,
                reflected_entity_is_selected(document, object, ctx.inspection),
                1,
                None,
                EditorTypeKind::Source,
                ctx,
            ));
            walk.rendered.insert(object.object_id.clone());
        }
    }

    block
}

fn render_prefab_entity_tree(
    mut block: gpui::Div,
    document: &AuthoredDocumentOutlineData,
    index: &PrefabOutlineIndex<'_>,
    entity_id: &str,
    depth: usize,
    ctx: OutlineContext<'_>,
    walk: &mut PrefabWalk,
) -> gpui::Div {
    let Some(entity) = index.objects.get(entity_id).copied() else {
        return block;
    };
    if !walk.rendered.insert(entity.object_id.clone()) {
        return block;
    }

    let has_children = index
        .instances_by_parent
        .get(entity_id)
        .is_some_and(|ids| !ids.is_empty())
        || index
            .entities_by_parent
            .get(entity_id)
            .is_some_and(|ids| !ids.is_empty());
    let kind = entity_kind(
        &entity.schema_type,
        entity
            .prefab_component_object_ids
            .iter()
            .filter_map(|component_id| {
                index
                    .objects
                    .get(component_id)
                    .map(|component| component.schema_type.as_str())
            }),
        has_children,
    );
    let collapse_key = format!("object:{}:{}", document.document_id, entity.object_id);
    let open = has_children && !ctx.collapsed.contains(&collapse_key);
    block = block.child(render_authored_object_outline(
        &document.document_id,
        entity,
        reflected_entity_is_selected(document, entity, ctx.inspection),
        depth,
        has_children.then_some(OutlineCaret { open, collapse_key }),
        kind,
        ctx,
    ));

    if !open {
        mark_prefab_descendants(index, entity_id, &mut walk.rendered);
        return block;
    }

    walk.rendered
        .extend(entity.prefab_component_object_ids.iter().cloned());

    if let Some(instance_ids) = index.instances_by_parent.get(entity_id) {
        for instance_id in instance_ids {
            if let Some(instance) = index.objects.get(instance_id).copied() {
                block =
                    render_prefab_instance_tree(block, document, instance, depth + 1, ctx, walk);
            }
        }
    }

    if let Some(child_ids) = index.entities_by_parent.get(entity_id) {
        for child_id in child_ids {
            block =
                render_prefab_entity_tree(block, document, index, child_id, depth + 1, ctx, walk);
        }
    }

    block
}

fn render_prefab_instance_tree(
    mut block: gpui::Div,
    document: &AuthoredDocumentOutlineData,
    instance: &AuthoredObjectOutlineData,
    depth: usize,
    ctx: OutlineContext<'_>,
    walk: &mut PrefabWalk,
) -> gpui::Div {
    if !walk.rendered.insert(instance.object_id.clone()) {
        return block;
    }

    let source_path = instance.prefab_source_path.as_deref();
    let source_document = source_path.and_then(|source_path| {
        ctx.all_documents.iter().find(|candidate| {
            candidate.document_id == source_path || candidate.source_path == source_path
        })
    });
    let has_children = source_document.is_some_and(|source_document| {
        !walk.instance_chain.contains(&source_document.source_path)
            && source_document.objects.iter().any(|object| {
                object.schema_type == ENGINE_PREFAB_ENTITY_SCHEMA_TYPE
                    && object.prefab_parent_entity_object_id.is_none()
            })
    });
    let collapse_key = format!("instance:{}:{}", document.document_id, instance.object_id);
    let open = has_children && !ctx.collapsed.contains(&collapse_key);
    block = block.child(render_authored_object_outline(
        &document.document_id,
        instance,
        reflected_entity_is_selected(document, instance, ctx.inspection),
        depth,
        has_children.then_some(OutlineCaret { open, collapse_key }),
        EditorTypeKind::Prefab,
        ctx,
    ));

    if !open {
        return block;
    }
    let Some(source_document) = source_document else {
        return block;
    };
    if !walk
        .instance_chain
        .insert(source_document.source_path.clone())
    {
        return block;
    }

    let source_index = PrefabOutlineIndex::new(source_document);
    // The nested document gets a fresh `rendered` set — its object ids are its
    // own — but keeps the caller's instance chain, so a cyclic prefab
    // reference still stops.
    let mut nested = PrefabWalk {
        rendered: BTreeSet::new(),
        instance_chain: std::mem::take(&mut walk.instance_chain),
    };
    for entity_id in &source_index.root_entity_ids {
        block = render_prefab_entity_tree(
            block,
            source_document,
            &source_index,
            entity_id,
            depth + 1,
            ctx,
            &mut nested,
        );
    }
    nested.instance_chain.remove(&source_document.source_path);
    walk.instance_chain = nested.instance_chain;
    block
}

fn mark_prefab_descendants(
    index: &PrefabOutlineIndex<'_>,
    entity_id: &str,
    rendered: &mut BTreeSet<String>,
) {
    if let Some(entity) = index.objects.get(entity_id).copied() {
        rendered.extend(entity.prefab_component_object_ids.iter().cloned());
    }
    if let Some(instance_ids) = index.instances_by_parent.get(entity_id) {
        rendered.extend(instance_ids.iter().cloned());
    }
    if let Some(child_ids) = index.entities_by_parent.get(entity_id) {
        for child_id in child_ids {
            rendered.insert(child_id.clone());
            mark_prefab_descendants(index, child_id, rendered);
        }
    }
}

struct PrefabOutlineIndex<'a> {
    objects: BTreeMap<String, &'a AuthoredObjectOutlineData>,
    root_entity_ids: Vec<String>,
    entities_by_parent: BTreeMap<String, Vec<String>>,
    instances_by_parent: BTreeMap<String, Vec<String>>,
}

impl<'a> PrefabOutlineIndex<'a> {
    fn new(document: &'a AuthoredDocumentOutlineData) -> Self {
        let mut objects = BTreeMap::new();
        let mut root_entity_ids = Vec::new();
        let mut entities_by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut instances_by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for object in &document.objects {
            objects.insert(object.object_id.clone(), object);
            match object.schema_type.as_str() {
                ENGINE_PREFAB_ENTITY_SCHEMA_TYPE => {
                    if let Some(parent) = &object.prefab_parent_entity_object_id {
                        entities_by_parent
                            .entry(parent.clone())
                            .or_default()
                            .push(object.object_id.clone());
                    } else {
                        root_entity_ids.push(object.object_id.clone());
                    }
                }
                ENGINE_PREFAB_INSTANCE_SCHEMA_TYPE => {
                    if let Some(parent) = &object.prefab_parent_entity_object_id {
                        instances_by_parent
                            .entry(parent.clone())
                            .or_default()
                            .push(object.object_id.clone());
                    }
                }
                _ => {}
            }
        }

        Self {
            objects,
            root_entity_ids,
            entities_by_parent,
            instances_by_parent,
        }
    }
}

fn prefab_outline_indent(depth: usize) -> gpui::Pixels {
    kit::indent(depth, 13.0, 6.0)
}

fn is_prefab_root_object(object: &AuthoredObjectOutlineData) -> bool {
    object.schema_type == ENGINE_PREFAB_ROOT_SCHEMA_TYPE
}

fn outline_object_label(object: &AuthoredObjectOutlineData) -> String {
    object
        .display_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map_or_else(
            || crate::naming::schema_display_name(&object.schema_type, None).into_owned(),
            str::to_string,
        )
}

struct AuthoredDocumentStatus {
    tint: gpui::Hsla,
    tooltip: String,
}

/// A status dot for a document row, or `None` when nominal (valid and saved).
fn authored_document_status(
    document: &AuthoredDocumentOutlineData,
    theme: &gpui_component::theme::Theme,
) -> Option<AuthoredDocumentStatus> {
    if !document.valid {
        let tooltip = if document.diagnostic.trim().is_empty() {
            "Invalid authored document".to_string()
        } else {
            format!("Invalid authored document: {}", document.diagnostic)
        };
        Some(AuthoredDocumentStatus {
            tint: theme.danger,
            tooltip,
        })
    } else if document.unsaved_changes {
        Some(AuthoredDocumentStatus {
            tint: theme.warning,
            tooltip: "Unsaved changes".to_string(),
        })
    } else {
        None
    }
}

#[cfg(test)]
fn visible_reflected_selection_fields(
    data: &ReflectedEntityInspection,
) -> impl Iterator<Item = &ReflectedInspectionField> {
    data.components
        .iter()
        .flat_map(|component| component.model.fields.iter())
        .filter(|field| !field.hidden)
}

fn render_empty_state(
    message: impl Into<String>,
    theme: &gpui_component::theme::Theme,
) -> gpui::Div {
    kit::empty_state(message, None, theme)
}

fn schema_element_key(schema_type: &str) -> String {
    schema_type
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_inspector::{
        AddComponentCapabilities, AddComponentEvaluationState, ReflectedAddComponent,
        ReflectedComponentInspection, ReflectedCurrentValue, ReflectedDefaultAvailability,
        ReflectedDefaultValue, ReflectedEditBinding, ReflectedInspectionModel,
        ReflectedPrefabSelection, ReflectedScalar, ReflectedValidationState, ReflectedValue,
        WidgetFamily, WidgetSpec,
    };
    use az_proto_project::vnext::{
        FieldConstraints, PrefabComponentSnapshot, PrefabValueTarget, ReflectedPath,
        ReflectedPathSegment, ReflectedTypeKind, ReflectedValueEncoding, ReflectedValueEnvelope,
    };

    fn inspection() -> ReflectedEntityInspection {
        let entity_alias = "018f08da-52c9-7664-a449-2df0f6c13f01";
        let type_path = "az.test.Door";
        ReflectedEntityInspection {
            selection: ReflectedPrefabSelection::new("prefabs/door.prefab.ron", entity_alias),
            registry_schema_catalog_hash: vec![7; 32],
            document_version: 1,
            type_versions: BTreeMap::from([(type_path.to_owned(), 1)]),
            revision: 4,
            components: vec![ReflectedComponentInspection {
                component: PrefabComponentSnapshot {
                    entity_alias: entity_alias.to_owned(),
                    type_path: type_path.to_owned(),
                    sparse_value: reflected_envelope(type_path),
                },
                model: ReflectedInspectionModel {
                    schema_catalog_hash: vec![7; 32],
                    entity_alias: entity_alias.to_owned(),
                    type_path: type_path.to_owned(),
                    type_label: "Door Controller".to_owned(),
                    category: Some("Gameplay".to_owned()),
                    icon: None,
                    description: None,
                    fields: vec![reflected_field(1, false)],
                    actions: Vec::new(),
                    validation: ReflectedValidationState::default(),
                    add_component: ReflectedAddComponent {
                        editor_export: true,
                        runtime_export: true,
                        default_available: false,
                        evaluation: AddComponentEvaluationState::NotProjected,
                        capabilities: AddComponentCapabilities::NotProjected,
                    },
                },
            }],
            overrides: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn reflected_envelope(type_path: &str) -> ReflectedValueEnvelope {
        ReflectedValueEnvelope {
            type_path: type_path.to_owned(),
            encoding: ReflectedValueEncoding::TypedRon,
            payload: b"()".to_vec(),
        }
    }

    fn reflected_field(field_id: u32, hidden: bool) -> ReflectedInspectionField {
        let name = format!("field_{field_id}");
        ReflectedInspectionField {
            name: format!("field_{field_id}"),
            label: format!("Field {field_id}"),
            description: None,
            read_only: false,
            hidden,
            actions: Vec::new(),
            widget: WidgetSpec {
                family: WidgetFamily::Number,
                range: None,
                rows: None,
                constraints: FieldConstraints::default(),
                variants: Vec::new(),
            },
            validation: ReflectedValidationState::default(),
            value: ReflectedValueNode {
                type_path: "f32".to_owned(),
                kind: ReflectedTypeKind::Float { bits: 32 },
                current: ReflectedCurrentValue {
                    authored: Some(ReflectedValue::Scalar(ReflectedScalar::Float(
                        field_id.to_string(),
                    ))),
                    effective: None,
                },
                default: ReflectedDefaultValue {
                    availability: ReflectedDefaultAvailability::Unavailable,
                    value: None,
                },
                binding: ReflectedEditBinding::new(PrefabValueTarget {
                    instance_alias_chain: Vec::new(),
                    entity_alias: "018f08da-52c9-7664-a449-2df0f6c13f01".to_owned(),
                    path: ReflectedPath {
                        component_type_path: "az.test.Door".to_owned(),
                        segments: vec![ReflectedPathSegment::Field(name)],
                    },
                }),
                children: Vec::new(),
            },
        }
    }

    fn outline() -> AuthoredOutlineData {
        AuthoredOutlineData {
            documents: vec![AuthoredDocumentOutlineData {
                document_id: "prefabs/door.prefab.ron".to_string(),
                source_path: "prefabs/door.prefab.ron".to_string(),
                schema_type: "az.test.Door".to_string(),
                revision: 4,
                saved_revision: Some(3),
                unsaved_changes: true,
                object_count: 1,
                journal_entry_count: 1,
                loaded: true,
                valid: true,
                diagnostic: String::new(),
                objects: vec![AuthoredObjectOutlineData {
                    object_id: "018f08da-52c9-7664-a449-2df0f6c13f01".to_string(),
                    schema_type: "az.test.Door".to_string(),
                    selected: true,
                    display_name: None,
                    prefab_parent_entity_object_id: None,
                    prefab_component_object_ids: Vec::new(),
                    prefab_owner_entity_object_id: None,
                    prefab_source_path: None,
                }],
            }],
        }
    }

    fn outline_object(
        object_id: &str,
        schema_type: &str,
        selected: bool,
    ) -> AuthoredObjectOutlineData {
        AuthoredObjectOutlineData {
            object_id: object_id.to_string(),
            schema_type: schema_type.to_string(),
            selected,
            display_name: None,
            prefab_parent_entity_object_id: None,
            prefab_component_object_ids: Vec::new(),
            prefab_owner_entity_object_id: None,
            prefab_source_path: None,
        }
    }

    #[test]
    fn reflected_outliner_filter_matches_source_entity_component_or_field() {
        let data = inspection();

        assert!(reflected_selection_matches(&data, "door.prefab"));
        assert!(reflected_selection_matches(&data, "018f08da"));
        assert!(reflected_selection_matches(&data, "az.test"));
        assert!(reflected_selection_matches(&data, "Door Controller"));
        assert!(reflected_selection_matches(&data, "Gameplay"));
        assert!(reflected_selection_matches(&data, "field_1"));
        assert!(!reflected_selection_matches(&data, "camera"));
    }

    #[test]
    fn reflected_selection_visible_fields_are_not_truncated() {
        let mut data = inspection();
        data.components[0].model.fields = (1..=24)
            .map(|field_id| reflected_field(field_id, field_id == 13))
            .collect();

        let visible_fields = visible_reflected_selection_fields(&data).collect::<Vec<_>>();

        assert_eq!(visible_fields.len(), 23);
        assert_eq!(
            visible_fields.first().map(|field| field.name.as_str()),
            Some("field_1")
        );
        assert_eq!(
            visible_fields.last().map(|field| field.name.as_str()),
            Some("field_24")
        );
        assert!(
            visible_fields
                .iter()
                .all(|field| field.name != "field_13" && !field.hidden)
        );
    }

    #[test]
    fn authored_outliner_filter_matches_document_outline_or_object() {
        let data = outline();

        assert!(authored_outline_has_matches(&data, "door.prefab"));
        assert!(authored_outline_has_matches(&data, "az.test"));
        assert!(authored_outline_has_matches(&data, "018f08da"));
        assert!(!authored_outline_has_matches(&data, "camera"));
    }

    #[test]
    fn ascii_case_insensitive_scan_handles_the_substring_edges() {
        assert!(contains_ignore_ascii_case("", ""));
        assert!(contains_ignore_ascii_case("Door.Prefab.ron", ""));
        assert!(!contains_ignore_ascii_case("Door", "Door.Prefab.ron"));
        assert!(contains_ignore_ascii_case("Door.Prefab.ron", "DOOR"));
        assert!(contains_ignore_ascii_case("Door.Prefab.ron", "prefab"));
        assert!(contains_ignore_ascii_case("Door.Prefab.ron", "RON"));
        assert!(contains_ignore_ascii_case(
            "Door.Prefab.ron",
            "door.prefab.ron"
        ));
        assert!(!contains_ignore_ascii_case("Door.Prefab.ron", "prefabs"));
        assert!(!contains_ignore_ascii_case("Door.Prefab.ron", "doo r"));
    }

    /// The retired matcher lowercased both sides and asked `contains`. ASCII
    /// case folding is a byte permutation, so the windowed scan must accept
    /// exactly the same pairs — including the non-ASCII ones it leaves alone.
    #[test]
    fn ascii_case_insensitive_scan_accepts_what_lowercased_contains_accepted() {
        for haystack in [
            "",
            "Door",
            "prefabs/DOOR.Prefab.ron",
            "Café Tür",
            "aAaA",
            "018F08DA-52c9",
        ] {
            for needle in [
                "",
                "door",
                "DOOR",
                "Café",
                "CAFÉ",
                "café",
                "tür",
                "TÜR",
                "aa",
                "AA",
                "zzz",
                "prefabs/door.prefab.ron",
                "52C9",
            ] {
                assert_eq!(
                    contains_ignore_ascii_case(haystack, needle),
                    haystack
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase()),
                    "`{needle}` in `{haystack}`"
                );
            }
        }
    }

    #[test]
    fn outliner_filters_match_regardless_of_ascii_case() {
        let data = outline();
        assert!(authored_outline_has_matches(&data, "DOOR.Prefab"));
        assert!(authored_outline_has_matches(&data, "AZ.Test"));
        assert!(authored_outline_has_matches(&data, "018F08DA"));

        let mut uppercase = outline();
        uppercase.documents[0].document_id = "Prefabs/DOOR.Prefab.ron".to_string();
        uppercase.documents[0].source_path = "Prefabs/DOOR.Prefab.ron".to_string();
        uppercase.documents[0].schema_type = "AZ.Test.Door".to_string();
        uppercase.documents[0].objects[0].schema_type = "AZ.Test.Door".to_string();
        assert!(authored_outline_has_matches(&uppercase, "door.prefab"));
        assert!(authored_outline_has_matches(&uppercase, "az.test"));
        assert!(!authored_outline_has_matches(&uppercase, "camera"));

        let selection = inspection();
        assert!(reflected_selection_matches(&selection, "DOOR.Prefab"));
        assert!(reflected_selection_matches(&selection, "door controller"));
        assert!(reflected_selection_matches(&selection, "gameplay"));
        assert!(reflected_selection_matches(&selection, "FIELD_1"));
        assert!(!reflected_selection_matches(&selection, "CAMERA"));
    }

    #[test]
    fn outliner_filters_compare_non_ascii_bytes_exactly() {
        let mut data = outline();
        data.documents[0].objects[0].display_name = Some("Café Tür".to_string());

        assert!(authored_outline_has_matches(&data, "café"));
        assert!(authored_outline_has_matches(&data, "CAFé"));
        assert!(authored_outline_has_matches(&data, "tür"));
        assert!(!authored_outline_has_matches(&data, "CAFÉ"));
        assert!(!authored_outline_has_matches(&data, "TÜR"));
    }

    #[test]
    fn outliner_filters_trim_the_needle_and_accept_an_empty_filter() {
        let data = outline();
        let selection = inspection();

        assert!(authored_outline_has_matches(&data, "  door.prefab  "));
        assert!(reflected_selection_matches(
            &selection,
            "\tDoor Controller\n"
        ));
        assert!(!authored_outline_has_matches(&data, "  camera  "));

        assert!(authored_outline_has_matches(&data, ""));
        assert!(authored_outline_has_matches(&data, "   "));
        assert!(reflected_selection_matches(&selection, ""));
        assert!(reflected_selection_matches(&selection, " \t "));
    }

    #[test]
    fn authored_outliner_footer_does_not_fallback_to_selection_without_outline() {
        assert_eq!(
            outline_footer_summary(None, None, &BTreeSet::new()),
            "no authored outline"
        );
    }

    #[test]
    fn schema_element_keys_are_stable_for_gpui_ids() {
        assert_eq!(
            schema_element_key("az.test::Door Prefab"),
            "az_test__Door_Prefab"
        );
    }

    #[test]
    fn authored_outline_error_state_preserves_project_host_failure() {
        let error = EditorAuthoredOutline::error("project-host unavailable");
        assert_eq!(
            error.status_error.as_deref(),
            Some("project-host unavailable")
        );
        assert!(error.data.documents.is_empty());

        let with_existing_data =
            EditorAuthoredOutline::new(outline()).with_error("documentList failed");
        assert_eq!(
            with_existing_data.status_error.as_deref(),
            Some("documentList failed")
        );
        assert_eq!(with_existing_data.data.documents.len(), 1);
    }

    #[test]
    fn scene_document_predicate_matches_scene_root_and_rejects_prefabs() {
        let mut data = outline();
        let mut document = data.documents.remove(0);

        document.schema_type = ENGINE_SCENE_ROOT_SCHEMA_TYPE.to_string();
        assert!(is_scene_document(&document));
        assert!(is_scene_document_schema(ENGINE_SCENE_ROOT_SCHEMA_TYPE));

        document.schema_type = ENGINE_PREFAB_ROOT_SCHEMA_TYPE.to_string();
        assert!(!is_scene_document(&document));
        assert!(!is_scene_document_schema(ENGINE_PREFAB_ROOT_SCHEMA_TYPE));

        document.schema_type = "azoth.gamedata.TableSource".to_string();
        assert!(!is_scene_document(&document));
        assert!(!is_scene_document_schema("azoth.gamedata.TableSource"));
    }

    #[test]
    fn active_level_projection_has_one_scene_root_and_only_its_prefab_closure() {
        let mut canyon_root = outline_object("scene-a", ENGINE_SCENE_ROOT_SCHEMA_TYPE, false);
        canyon_root.display_name = Some("Canyon".to_string());
        canyon_root.prefab_source_path = Some("prefabs/canyon.prefab.ron".to_string());
        let mut arena_root = outline_object("scene-b", ENGINE_SCENE_ROOT_SCHEMA_TYPE, false);
        arena_root.display_name = Some("Arena".to_string());
        arena_root.prefab_source_path = Some("prefabs/arena.prefab.ron".to_string());
        let mut nested_instance =
            outline_object("rocks-instance", ENGINE_PREFAB_INSTANCE_SCHEMA_TYPE, false);
        nested_instance.prefab_source_path = Some("prefabs/rocks.prefab.ron".to_string());

        let document =
            |document_id: &str, schema_type: &str, objects: Vec<AuthoredObjectOutlineData>| {
                AuthoredDocumentOutlineData {
                    document_id: document_id.to_string(),
                    source_path: document_id.to_string(),
                    schema_type: schema_type.to_string(),
                    revision: 1,
                    saved_revision: Some(1),
                    unsaved_changes: false,
                    object_count: u32::try_from(objects.len()).unwrap_or(u32::MAX),
                    journal_entry_count: 0,
                    loaded: true,
                    valid: true,
                    diagnostic: String::new(),
                    objects,
                }
            };
        let outline = AuthoredOutlineData {
            documents: vec![
                document(
                    "scenes/canyon.scene.ron",
                    ENGINE_SCENE_ROOT_SCHEMA_TYPE,
                    vec![canyon_root],
                ),
                document(
                    "scenes/arena.scene.ron",
                    ENGINE_SCENE_ROOT_SCHEMA_TYPE,
                    vec![arena_root],
                ),
                document(
                    "prefabs/canyon.prefab.ron",
                    ENGINE_PREFAB_ROOT_SCHEMA_TYPE,
                    vec![
                        outline_object("canyon-root", ENGINE_PREFAB_ENTITY_SCHEMA_TYPE, false),
                        outline_object("gameplay", ENGINE_PREFAB_ENTITY_SCHEMA_TYPE, false),
                        nested_instance,
                    ],
                ),
                document(
                    "prefabs/rocks.prefab.ron",
                    ENGINE_PREFAB_ROOT_SCHEMA_TYPE,
                    vec![outline_object(
                        "rock-cluster",
                        ENGINE_PREFAB_ENTITY_SCHEMA_TYPE,
                        false,
                    )],
                ),
                document(
                    "prefabs/arena.prefab.ron",
                    ENGINE_PREFAB_ROOT_SCHEMA_TYPE,
                    vec![outline_object(
                        "arena-root",
                        ENGINE_PREFAB_ENTITY_SCHEMA_TYPE,
                        false,
                    )],
                ),
            ],
        };

        let active =
            active_level_document(&outline, Some("scenes/canyon.scene.ron")).expect("active scene");
        assert_eq!(active.document_id, "scenes/canyon.scene.ron");
        assert_eq!(
            active_level_prefab_documents(&outline, Some("scenes/canyon.scene.ron"))
                .into_iter()
                .map(|document| document.document_id.as_str())
                .collect::<Vec<_>>(),
            vec!["prefabs/canyon.prefab.ron", "prefabs/rocks.prefab.ron"]
        );
        assert_eq!(
            outline_footer_summary(
                Some(&outline),
                Some("scenes/canyon.scene.ron"),
                &BTreeSet::new(),
            ),
            "3 entities · 3 visible"
        );
        assert_eq!(
            outline_footer_summary(
                Some(&outline),
                Some("scenes/arena.scene.ron"),
                &BTreeSet::new(),
            ),
            "1 entities · 1 visible"
        );
    }

    #[test]
    fn prefab_entity_toolbar_targets_follow_reflected_selection() {
        let selection = inspection();
        assert_eq!(
            selected_prefab_entity_add_target(Some(&selection)),
            Some(selection.selection.entity_alias.clone())
        );
        assert_eq!(
            selected_prefab_entity_remove_target(Some(&selection)),
            Some(selection.selection.entity_alias.clone())
        );
        assert_eq!(selected_prefab_entity_add_target(None), None);
        assert_eq!(selected_prefab_entity_remove_target(None), None);
    }

    #[test]
    fn hierarchy_selection_uses_reflected_source_and_entity_alias() {
        let mut document = outline().documents.remove(0);
        document.schema_type = ENGINE_PREFAB_ROOT_SCHEMA_TYPE.to_owned();
        document.objects[0].schema_type = ENGINE_PREFAB_ENTITY_SCHEMA_TYPE.to_owned();
        let selection = inspection();

        assert!(reflected_entity_is_selected(
            &document,
            &document.objects[0],
            Some(&selection),
        ));
        document.source_path = "prefabs/other.prefab.ron".to_owned();
        document.document_id = "prefabs/other.prefab.ron".to_owned();
        assert!(!reflected_entity_is_selected(
            &document,
            &document.objects[0],
            Some(&selection),
        ));
    }

    #[test]
    fn prefab_outline_index_groups_entities_components_and_instances() {
        let document = AuthoredDocumentOutlineData {
            document_id: "prefabs/main.prefab.ron".to_string(),
            source_path: "prefabs/main.prefab.ron".to_string(),
            schema_type: ENGINE_PREFAB_ROOT_SCHEMA_TYPE.to_string(),
            revision: 1,
            saved_revision: Some(1),
            unsaved_changes: false,
            object_count: 5,
            journal_entry_count: 0,
            loaded: true,
            valid: true,
            diagnostic: String::new(),
            objects: vec![
                outline_object("prefab-root", ENGINE_PREFAB_ROOT_SCHEMA_TYPE, false),
                AuthoredObjectOutlineData {
                    object_id: "entity-root".to_string(),
                    schema_type: ENGINE_PREFAB_ENTITY_SCHEMA_TYPE.to_string(),
                    selected: false,
                    display_name: Some("Root".to_string()),
                    prefab_parent_entity_object_id: None,
                    prefab_component_object_ids: vec!["transform".to_string()],
                    prefab_owner_entity_object_id: None,
                    prefab_source_path: None,
                },
                AuthoredObjectOutlineData {
                    object_id: "entity-child".to_string(),
                    schema_type: ENGINE_PREFAB_ENTITY_SCHEMA_TYPE.to_string(),
                    selected: false,
                    display_name: Some("Child".to_string()),
                    prefab_parent_entity_object_id: Some("entity-root".to_string()),
                    prefab_component_object_ids: Vec::new(),
                    prefab_owner_entity_object_id: None,
                    prefab_source_path: None,
                },
                AuthoredObjectOutlineData {
                    object_id: "transform".to_string(),
                    schema_type: "az.test.Transform".to_string(),
                    selected: false,
                    display_name: None,
                    prefab_parent_entity_object_id: None,
                    prefab_component_object_ids: Vec::new(),
                    prefab_owner_entity_object_id: Some("entity-root".to_string()),
                    prefab_source_path: None,
                },
                AuthoredObjectOutlineData {
                    object_id: "child-instance".to_string(),
                    schema_type: ENGINE_PREFAB_INSTANCE_SCHEMA_TYPE.to_string(),
                    selected: false,
                    display_name: Some("Child Instance".to_string()),
                    prefab_parent_entity_object_id: Some("entity-root".to_string()),
                    prefab_component_object_ids: Vec::new(),
                    prefab_owner_entity_object_id: None,
                    prefab_source_path: None,
                },
            ],
        };

        let index = PrefabOutlineIndex::new(&document);

        assert_eq!(index.root_entity_ids, vec!["entity-root".to_string()]);
        assert_eq!(
            index.entities_by_parent.get("entity-root"),
            Some(&vec!["entity-child".to_string()])
        );
        assert_eq!(
            index.instances_by_parent.get("entity-root"),
            Some(&vec!["child-instance".to_string()])
        );
    }
}
