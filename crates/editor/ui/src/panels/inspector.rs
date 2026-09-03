//! Reflected Prefab inspector.
//!
//! GPUI owns presentation and keyed control state. Project-host reflection owns
//! field shape, validation, paths, and edit bindings.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use az_editor_inspector::{
    ReflectedComponentInspection, ReflectedEditBinding, ReflectedEntityInspection,
    ReflectedInspectionChild, ReflectedInspectionField, ReflectedMapEntryBinding, ReflectedScalar,
    ReflectedValidationState, ReflectedValue, ReflectedValueNode, WidgetFamily,
};
use az_proto_project::vnext::{
    PrefabEditCommand, PrefabValueTarget, ReflectedPath, ReflectedPathSegment,
    ReflectedValueEncoding, ReflectedValueEnvelope,
};
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Global, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, px,
};
use gpui_component::dock::{Panel, PanelControl, PanelEvent, PanelInfo, PanelState};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, IndexPath, Sizable, StyledExt, Theme,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState, SliderValue},
    tooltip::Tooltip,
    v_flex,
};

use super::asset_browser::{AssetBrowserEntryStatus, EditorAssetBrowserStatus};
use super::outliner::{CreatableAuthoredSchemaData, EditorAuthoredOutline};
use crate::panels::kit;

const TEXTAREA_DEFAULT_ROWS: usize = 3;
const TEXTAREA_MAX_ROWS: usize = 12;

/// Component schemas still supplied by the deferred Add Component flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorAddableAuthoredComponents {
    pub schemas: Vec<CreatableAuthoredSchemaData>,
}

impl EditorAddableAuthoredComponents {
    #[must_use]
    pub const fn new(schemas: Vec<CreatableAuthoredSchemaData>) -> Self {
        Self { schemas }
    }
}

impl Global for EditorAddableAuthoredComponents {}

/// Editor-owned current vNext selection and its immutable projection.
#[derive(Debug, Clone, Default)]
pub struct EditorReflectedSelectionState {
    current: Option<ReflectedEntityInspection>,
}

impl EditorReflectedSelectionState {
    #[must_use]
    pub const fn new() -> Self {
        Self { current: None }
    }

    #[must_use]
    pub const fn current(&self) -> Option<&ReflectedEntityInspection> {
        self.current.as_ref()
    }

    pub fn set_current(&mut self, inspection: ReflectedEntityInspection) {
        self.current = Some(inspection);
    }

    pub fn clear(&mut self) {
        self.current = None;
    }
}

impl Global for EditorReflectedSelectionState {}

/// Reflected Prefab inspector panel.
pub struct AuthoredInspector {
    focus: FocusHandle,
    active_tab: InspectorTab,
    collapsed_component_types: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum InspectorTab {
    #[default]
    Details,
    Prefab,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct InspectorPanelState {
    active_tab: InspectorTab,
    collapsed_component_types: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReflectedControlScope {
    source_path: String,
    entity_alias: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReflectedControlKind {
    AssetBrowse,
    ObjectBrowse,
    Slider,
    ColorPicker,
    DropdownSelect,
    MapKeyInput,
    TextareaInput,
    TextInput,
}

impl ReflectedControlKind {
    const fn key(self) -> &'static str {
        match self {
            Self::AssetBrowse => "asset-browse",
            Self::ObjectBrowse => "object-browse",
            Self::Slider => "slider",
            Self::ColorPicker => "color-picker",
            Self::DropdownSelect => "dropdown-select",
            Self::MapKeyInput => "map-key-input",
            Self::TextareaInput => "textarea-input",
            Self::TextInput => "text-input",
        }
    }
}

impl ReflectedControlScope {
    fn state_key(&self, target: &ReflectedEditTarget, kind: ReflectedControlKind) -> SharedString {
        let mut key = format!("reflected-control:{}:", kind.key());
        push_control_key_part(&mut key, &self.source_path);
        push_control_key_part(&mut key, &self.entity_alias);
        push_reflected_path_key(&mut key, &target.binding().target.path);
        if let ReflectedEditTarget::MapEntry(binding) = target {
            key.push_str("map-key:");
            push_control_key_part(&mut key, &String::from_utf8_lossy(&binding.key.payload));
        }
        key.into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReflectedEditTarget {
    Direct(ReflectedEditBinding),
    MapEntry(ReflectedMapEntryBinding),
}

impl ReflectedEditTarget {
    const fn binding(&self) -> &ReflectedEditBinding {
        match self {
            Self::Direct(binding) => binding,
            Self::MapEntry(binding) => &binding.map,
        }
    }

    fn set_value(&self, value: ReflectedValueEnvelope) -> PrefabEditCommand {
        match self {
            Self::Direct(binding) => binding.set_value(value),
            Self::MapEntry(binding) => binding.set_value(value),
        }
    }
}

struct ReflectedTextInputState {
    input: Entity<InputState>,
    _subscription: Subscription,
}

struct ReflectedSliderState {
    slider: Entity<SliderState>,
    last_value: f32,
    _subscription: Subscription,
}

struct ReflectedColorPickerState {
    picker: Entity<ColorPickerState>,
    binding: ReflectedEditBinding,
    type_path: String,
    value: ReflectedValue,
    last_synced: [f32; 4],
    _subscription: Subscription,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReflectedDropdownChoice {
    label: String,
    value: String,
}

impl SelectItem for ReflectedDropdownChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

struct ReflectedDropdownSelectState {
    select: Entity<SelectState<SearchableVec<ReflectedDropdownChoice>>>,
    choices: Vec<ReflectedDropdownChoice>,
    selected_value: Option<String>,
    _subscription: Subscription,
}

#[derive(Clone, Debug)]
struct ReflectedVectorComponentData {
    label: String,
    node: ReflectedValueNode,
}

#[derive(Clone, Debug)]
struct ReflectedColorPickerData {
    rgba: [f32; 4],
    binding: ReflectedEditBinding,
    type_path: String,
    value: ReflectedValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextValueEncoding {
    ReflectedString,
    RawRon,
}

impl AuthoredInspector {
    pub const NAME: &'static str = "inspector";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            active_tab: InspectorTab::Details,
            collapsed_component_types: BTreeSet::new(),
        }
    }

    pub fn init_from_panel_state(state: &PanelState, cx: &mut Context<'_, Self>) -> Self {
        let persisted = match &state.info {
            PanelInfo::Panel(value) => {
                serde_json::from_value::<InspectorPanelState>(value.clone()).unwrap_or_default()
            }
            _ => InspectorPanelState::default(),
        };
        Self {
            focus: cx.focus_handle(),
            active_tab: persisted.active_tab,
            collapsed_component_types: persisted.collapsed_component_types,
        }
    }
}

impl Render for AuthoredInspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(reason) = crate::panels::editor_project_host_failed_reason(cx) {
            return crate::panels::render_project_host_failed_placeholder("Inspector", &reason, cx)
                .into_any_element();
        }
        if crate::panels::editor_project_host_connecting(cx) {
            return crate::panels::render_project_host_connecting_placeholder("Inspector", cx)
                .into_any_element();
        }

        let inspection = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
            .cloned();
        let creatable_components = cx
            .try_global::<EditorAddableAuthoredComponents>()
            .map(|global| global.schemas.clone())
            .unwrap_or_default();
        let theme = cx.theme().clone();
        let active_tab = self.active_tab;
        let collapsed_component_types = self.collapsed_component_types.clone();

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(render_inspector_tab_strip(active_tab, cx))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(inspection.map_or_else(
                        || render_inspector_fallback(&theme),
                        |inspection| {
                            render_authored_inspection_content(
                                &inspection,
                                creatable_components,
                                active_tab,
                                &collapsed_component_types,
                                &theme,
                                window,
                                cx,
                            )
                        },
                    )),
            )
            .into_any_element()
    }
}

fn render_authored_inspection_content(
    inspection: &ReflectedEntityInspection,
    creatable_components: Vec<CreatableAuthoredSchemaData>,
    active_tab: InspectorTab,
    collapsed_component_types: &BTreeSet<String>,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    if active_tab == InspectorTab::Prefab {
        return render_prefab_view(inspection, theme).into_any_element();
    }

    let display_name = crate::naming::display_name(&inspection.selection.entity_alias).into_owned();
    let enabled = entity_field(inspection, &["enabled"]);
    let tag = entity_field(inspection, &["tag"]);
    let layer = entity_field(inspection, &["layer"]);
    let static_field = entity_field(inspection, &["static", "is_static"]);
    let scope = ReflectedControlScope {
        source_path: inspection.selection.source_path.clone(),
        entity_alias: inspection.selection.entity_alias.clone(),
    };
    let existing_components = inspection
        .components
        .iter()
        .map(|component| component.component.type_path.clone())
        .collect::<Vec<_>>();

    v_flex()
        .w_full()
        .child(
            v_flex()
                .w_full()
                .gap_1p5()
                .px(px(11.0))
                .py(px(9.0))
                .bg(theme.sidebar)
                .border_b_1()
                .border_color(theme.table_row_border)
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(enabled.as_ref().map_or_else(
                            || render_disabled_entity_enabled_toggle(theme),
                            |field| render_entity_enabled_toggle(field, theme),
                        ))
                        .child(kit::material_symbol_icon(
                            "deployed_code",
                            17.0,
                            theme.accent,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(13.0))
                                .font_semibold()
                                .text_color(theme.tab_active_foreground)
                                .child(display_name),
                        )
                        .child(kit::meta_text(format!("r{}", inspection.revision), theme)),
                ),
        )
        .child(render_entity_metadata_row(
            tag.as_ref(),
            layer.as_ref(),
            static_field.as_ref(),
            &scope,
            theme,
            window,
            cx,
        ))
        .children(inspection.components.iter().map(|component| {
            let collapsed = collapsed_component_types.contains(&component.component.type_path);
            render_inspector_component(component, collapsed, &scope, theme, window, cx)
        }))
        .child(render_add_component_menu(
            creatable_components,
            &existing_components,
            theme,
        ))
        .into_any_element()
}

fn entity_field(
    inspection: &ReflectedEntityInspection,
    names: &[&str],
) -> Option<ReflectedInspectionField> {
    inspection
        .components
        .iter()
        .flat_map(|component| &component.model.fields)
        .find(|field| {
            names
                .iter()
                .any(|name| field.name.eq_ignore_ascii_case(name))
        })
        .cloned()
}

fn render_disabled_entity_enabled_toggle(theme: &Theme) -> gpui::AnyElement {
    div()
        .size(px(15.0))
        .rounded(px(3.0))
        .bg(theme.input_background())
        .border_1()
        .border_color(theme.border.opacity(0.55))
        .opacity(0.45)
        .into_any_element()
}

fn render_entity_enabled_toggle(
    field: &ReflectedInspectionField,
    theme: &Theme,
) -> gpui::AnyElement {
    let checked = reflected_bool(&field.value).unwrap_or(true);
    let command = field_editable(field)
        .then(|| reflected_envelope(&field.value.type_path, (!checked).to_string()))
        .map(|value| field.value.binding.set_value(value));
    div()
        .id("inspector-entity-enabled")
        .size(px(15.0))
        .rounded(px(3.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(if checked {
            theme.accent
        } else {
            theme.input_background()
        })
        .border_1()
        .border_color(if checked { theme.accent } else { theme.border })
        .when(checked, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .with_size(px(12.0))
                    .text_color(theme.accent_foreground),
            )
        })
        .when_some(command, |this, command| {
            this.cursor_pointer().on_click(move |_, window, cx| {
                cx.stop_propagation();
                dispatch_reflected_command(window, cx, command.clone());
            })
        })
        .into_any_element()
}

fn render_entity_metadata_row(
    tag: Option<&ReflectedInspectionField>,
    layer: Option<&ReflectedInspectionField>,
    static_field: Option<&ReflectedInspectionField>,
    scope: &ReflectedControlScope,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .gap_1p5()
        .px(px(11.0))
        .pb(px(9.0))
        .bg(theme.sidebar)
        .border_b_1()
        .border_color(theme.table_row_border)
        .child(render_entity_metadata_select(
            "Tag", tag, scope, theme, window, cx,
        ))
        .child(render_entity_metadata_select(
            "Layer", layer, scope, theme, window, cx,
        ))
        .child(
            v_flex()
                .flex_none()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme.muted_foreground)
                        .child("Static"),
                )
                .child(static_field.map_or_else(
                    || render_disabled_entity_enabled_toggle(theme),
                    |field| render_entity_static_control(field, theme),
                )),
        )
        .into_any_element()
}

fn render_entity_metadata_select(
    label: &'static str,
    field: Option<&ReflectedInspectionField>,
    scope: &ReflectedControlScope,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let value = field.map_or_else(|| "Not available".to_owned(), field_display_value);
    let disabled = field.is_none_or(|field| !field_editable(field));
    let control = field.map_or_else(
        || kit::inspector_metadata_select(value.clone(), true, theme).into_any_element(),
        |field| {
            render_reflected_dropdown_select(field, scope, window, cx).unwrap_or_else(|| {
                kit::inspector_metadata_select(value.clone(), disabled, theme).into_any_element()
            })
        },
    );
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(3.0))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(control)
        .into_any_element()
}

fn render_entity_static_control(
    field: &ReflectedInspectionField,
    theme: &Theme,
) -> gpui::AnyElement {
    let checked = reflected_bool(&field.value).unwrap_or(false);
    let command = field_editable(field)
        .then(|| reflected_envelope(&field.value.type_path, (!checked).to_string()))
        .map(|value| field.value.binding.set_value(value));
    kit::inspector_control(theme)
        .id("inspector-static")
        .w(px(31.0))
        .justify_center()
        .opacity(if command.is_some() { 1.0 } else { 0.45 })
        .child(kit::material_symbol_icon(
            if checked {
                "check_box"
            } else {
                "check_box_outline_blank"
            },
            15.0,
            if checked {
                theme.success
            } else {
                theme.muted_foreground
            },
        ))
        .when_some(command, |this, command| {
            this.cursor_pointer().on_click(move |_, window, cx| {
                dispatch_reflected_command(window, cx, command.clone());
            })
        })
        .into_any_element()
}

/// The enabled/disabled switch on a component header. Disabled when the
/// component publishes no `enabled` field to write through.
fn render_component_enabled_toggle(
    type_path: &str,
    enabled: bool,
    toggle_command: Option<PrefabEditCommand>,
    theme: &Theme,
    // The returned element owns its content; spelling that out frees the
    // caller to move `type_path` into the header afterwards.
) -> impl IntoElement + use<> {
    kit::inspector_toggle(
        SharedString::from(format!(
            "component-enabled-{}",
            sanitize_element_key(type_path)
        )),
        enabled,
        toggle_command.is_none(),
        theme,
    )
    .when_some(toggle_command, |this, command| {
        this.on_click(move |_, window, cx| {
            cx.stop_propagation();
            dispatch_reflected_command(window, cx, command.clone());
        })
    })
}

/// The overflow menu on a component header: Reset (not yet published by the
/// project host) and Remove Component.
fn render_component_actions_menu(
    type_path: &str,
    remove_command: PrefabEditCommand,
) -> impl IntoElement + use<> {
    Button::new(SharedString::from(format!(
        "component-menu-{}",
        sanitize_element_key(type_path)
    )))
    .icon(IconName::Ellipsis)
    .ghost()
    .small()
    .tooltip("Component actions")
    .dropdown_menu(move |mut popup, _window, _cx| {
        popup = popup.item(PopupMenuItem::new("Reset").disabled(true));
        popup
            .separator()
            .item(PopupMenuItem::new("Remove Component").on_click({
                let command = remove_command.clone();
                move |_, window, cx| dispatch_reflected_command(window, cx, command.clone())
            }))
    })
}

/// The chrome a component header carries that is built by its caller: the
/// hover tint, the enabled toggle, and the overflow menu.
struct ComponentHeaderChrome {
    header_hover: Hsla,
    toggle: gpui::AnyElement,
    menu: gpui::AnyElement,
}

/// One component card's header row: disclosure caret, type icon, type label,
/// validation indicator, enabled toggle and overflow menu. Clicking the row
/// collapses the card.
fn render_component_header(
    component: &ReflectedComponentInspection,
    collapse_type_path: String,
    component_icon: &str,
    collapsed: bool,
    chrome: ComponentHeaderChrome,
    theme: &Theme,
    cx: &Context<'_, AuthoredInspector>,
) -> impl IntoElement {
    let type_path = component.component.type_path.clone();
    let validation = component.model.validation.clone();
    let ComponentHeaderChrome {
        header_hover,
        toggle,
        menu,
    } = chrome;
    h_flex()
        .id(SharedString::from(format!(
            "component-card-{}",
            sanitize_element_key(&type_path)
        )))
        .h(px(30.0))
        .px(px(9.0))
        .gap_1p5()
        .items_center()
        .bg(theme.sidebar_accent)
        .hover(move |this| this.bg(header_hover))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this
                .collapsed_component_types
                .insert(collapse_type_path.clone())
            {
                this.collapsed_component_types.remove(&collapse_type_path);
            }
            cx.emit(PanelEvent::LayoutChanged);
            cx.notify();
        }))
        .child(
            Icon::new(if collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            })
            .with_size(px(16.0))
            .text_color(theme.muted_foreground),
        )
        .child(kit::material_symbol_icon(
            component_icon,
            15.0,
            theme.accent,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_ellipsis()
                .font_semibold()
                .text_size(px(11.5))
                .text_color(theme.foreground)
                .child(component.model.type_label.clone()),
        )
        .when(!validation.is_valid(), |this| {
            this.child(render_validation_indicator(&validation, theme))
        })
        .child(toggle)
        .child(menu)
}
fn render_inspector_component(
    component: &ReflectedComponentInspection,
    collapsed: bool,
    scope: &ReflectedControlScope,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let type_path = component.component.type_path.clone();
    let enabled_field = component
        .model
        .fields
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case("enabled"));
    let enabled = enabled_field
        .and_then(|field| reflected_bool(&field.value))
        .unwrap_or(true);
    let toggle_command = enabled_field
        .filter(|field| field_editable(field))
        .map(|field| {
            field.value.binding.set_value(reflected_envelope(
                &field.value.type_path,
                (!enabled).to_string(),
            ))
        });
    let fields = component
        .model
        .fields
        .iter()
        .filter(|field| !field.name.eq_ignore_ascii_case("enabled"))
        .cloned()
        .collect::<Vec<_>>();
    let component_binding = component_root_binding(component);
    let remove_command = PrefabEditCommand::RemoveComponent {
        entity_alias: component.component.entity_alias.clone(),
        component_type_path: type_path.clone(),
    };
    let component_icon = component.model.icon.as_deref().unwrap_or("deployed_code");
    let header_hover = theme.secondary_hover;
    let toggle = render_component_enabled_toggle(&type_path, enabled, toggle_command, theme);
    let menu = render_component_actions_menu(&type_path, remove_command);
    let actions = component.model.actions.clone();

    v_flex()
        .w_full()
        .border_b_1()
        .border_color(theme.table_row_border)
        .child(render_component_header(
            component,
            type_path,
            component_icon,
            collapsed,
            ComponentHeaderChrome {
                header_hover,
                toggle: toggle.into_any_element(),
                menu: menu.into_any_element(),
            },
            theme,
            cx,
        ))
        .when(!collapsed, |this| {
            this.child(
                div()
                    .w_full()
                    .bg(theme.sidebar)
                    .child(render_authored_fields_list(
                        fields, scope, theme, window, cx,
                    ))
                    .child(render_reflected_actions(actions, &component_binding, theme)),
            )
        })
        .into_any_element()
}

fn component_root_binding(component: &ReflectedComponentInspection) -> ReflectedEditBinding {
    ReflectedEditBinding::new(PrefabValueTarget {
        instance_alias_chain: Vec::new(),
        entity_alias: component.component.entity_alias.clone(),
        path: ReflectedPath {
            component_type_path: component.component.type_path.clone(),
            segments: Vec::new(),
        },
    })
}

fn render_authored_fields_list(
    fields: Vec<ReflectedInspectionField>,
    scope: &ReflectedControlScope,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    if fields.is_empty() {
        return div()
            .p_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child("No reflected fields")
            .into_any_element();
    }

    v_flex()
        .w_full()
        .px(px(9.0))
        .py(px(7.0))
        .children(fields.into_iter().map(|field| {
            if field.hidden {
                return div().into_any_element();
            }
            render_reflected_field(&field, scope, window, cx)
        }))
        .into_any_element()
}

fn render_reflected_field(
    field: &ReflectedInspectionField,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let grouped = matches!(
        field.widget.family,
        WidgetFamily::Vector { .. } | WidgetFamily::Quaternion | WidgetFamily::Color
    );
    let children = field.value.children.clone();
    let read_only = field.read_only;
    let description = field.description.clone();
    let actions = field.actions.clone();
    let binding = field.value.binding.clone();
    let label_id = SharedString::from(format!(
        "reflected-field-label-{}",
        reflected_path_element_key(&field.value.binding.target.path)
    ));

    v_flex()
        .w_full()
        .gap(px(3.0))
        .child(
            h_flex()
                .w_full()
                .min_h(px(24.0))
                .items_center()
                .gap_2()
                .child(
                    h_flex()
                        .id(label_id)
                        .w(px(104.0))
                        .flex_none()
                        .min_w_0()
                        .pr_2()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(px(11.0))
                        .text_color(theme.description_list_label_foreground)
                        .child(field.label.clone())
                        .when(read_only, |this| {
                            this.child(
                                Icon::new(IconName::EyeOff)
                                    .with_size(px(11.0))
                                    .text_color(theme.muted_foreground),
                            )
                        })
                        .when(!field.validation.is_valid(), |this| {
                            this.child(render_validation_indicator(&field.validation, &theme))
                        })
                        .when_some(description, |this, description| {
                            this.tooltip(move |window, cx| {
                                Tooltip::new(description.clone()).build(window, cx)
                            })
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(render_authored_field_control(field, scope, window, cx)),
                ),
        )
        .when(!children.is_empty() && !grouped, |this| {
            this.child(div().ml(px(104.0)).child(render_authored_value_children(
                children,
                field_editable(field),
                scope,
                window,
                cx,
            )))
        })
        .when(!actions.is_empty(), |this| {
            this.child(
                div()
                    .ml(px(104.0))
                    .child(render_reflected_actions(actions, &binding, &theme)),
            )
        })
        .into_any_element()
}

/// Multiline text control: a sized textarea over the authored string, falling
/// back to a read-only label when the value is not a string.
fn render_authored_multiline_control(
    field: &ReflectedInspectionField,
    target: &ReflectedEditTarget,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let Some(value) = reflected_string(&field.value) else {
        return render_reflected_value_label(&field.value, &theme);
    };
    render_authored_textarea_input(
        target.clone(),
        &field.value.type_path,
        &value,
        field
            .widget
            .rows
            .map_or(TEXTAREA_DEFAULT_ROWS, |rows| rows as usize),
        scope,
        window,
        cx,
    )
}

/// Single-line text control: the enum dropdown when the field publishes one,
/// otherwise a text input over the authored string.
fn render_authored_text_control(
    field: &ReflectedInspectionField,
    target: &ReflectedEditTarget,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    if let Some(dropdown) = render_reflected_dropdown_select(field, scope, window, cx) {
        return dropdown;
    }
    let theme = cx.theme().clone();
    let Some(value) = reflected_string(&field.value) else {
        return render_reflected_value_label(&field.value, &theme);
    };
    render_authored_text_input(
        target.clone(),
        &field.value.type_path,
        &value,
        TextValueEncoding::ReflectedString,
        scope,
        window,
        cx,
    )
}

fn render_authored_field_control(
    field: &ReflectedInspectionField,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let target = ReflectedEditTarget::Direct(field.value.binding.clone());
    let editable = field_editable(field);
    let control = match &field.widget.family {
        WidgetFamily::Slider if editable => render_authored_slider(field, scope, window, cx)
            .unwrap_or_else(|| render_reflected_number(field, scope, window, cx)),
        WidgetFamily::Number if editable => render_reflected_number(field, scope, window, cx),
        WidgetFamily::Bool if editable => render_reflected_bool(field, &theme),
        WidgetFamily::Color if editable => reflected_color_picker_data(&field.value).map_or_else(
            || render_reflected_value_label(&field.value, &theme),
            |color| render_authored_color_picker(&field.value, &color, scope, window, cx),
        ),
        WidgetFamily::Enum if editable => {
            render_reflected_dropdown_select(field, scope, window, cx)
                .unwrap_or_else(|| render_reflected_value_label(&field.value, &theme))
        }
        WidgetFamily::Asset { asset_type } if editable => {
            render_authored_asset_path_control(&field.value, asset_type, scope, window, cx)
        }
        WidgetFamily::Object { object_type } if editable => {
            render_authored_object_ref_control(&field.value, object_type, scope, window, cx)
        }
        WidgetFamily::Vector { dimensions } if editable => {
            let components = reflected_vector_components(&field.value, usize::from(*dimensions));
            render_authored_vector_components(components, field_suffix(field), scope, window, cx)
        }
        WidgetFamily::Quaternion if editable => {
            let components = reflected_vector_components(&field.value, 4);
            render_authored_vector_components(components, field_suffix(field), scope, window, cx)
        }
        WidgetFamily::Multiline if editable => {
            render_authored_multiline_control(field, &target, scope, window, cx)
        }
        WidgetFamily::Text if editable => {
            render_authored_text_control(field, &target, scope, window, cx)
        }
        WidgetFamily::List if editable => render_reflected_list_control(&field.value, &theme),
        WidgetFamily::Map if editable => {
            render_reflected_map_control(&field.value, scope, window, cx)
        }
        WidgetFamily::Optional if editable => {
            render_reflected_optional_control(&field.value, &theme)
        }
        WidgetFamily::Struct
        | WidgetFamily::List
        | WidgetFamily::Map
        | WidgetFamily::Optional
        | WidgetFamily::Opaque
        | WidgetFamily::Number
        | WidgetFamily::Slider
        | WidgetFamily::Bool
        | WidgetFamily::Color
        | WidgetFamily::Enum
        | WidgetFamily::Asset { .. }
        | WidgetFamily::Object { .. }
        | WidgetFamily::Vector { .. }
        | WidgetFamily::Quaternion
        | WidgetFamily::Multiline
        | WidgetFamily::Text => render_reflected_value_label(&field.value, &theme),
    };

    if editable && field.value.current.authored.is_some() {
        let command = field.value.binding.remove_override();
        h_flex()
            .items_center()
            .gap_1()
            .child(control)
            .child(reflected_command_button(
                command,
                "reset",
                "Reset authored override",
                &theme,
            ))
            .into_any_element()
    } else {
        control
    }
}

fn render_reflected_bool(field: &ReflectedInspectionField, theme: &Theme) -> gpui::AnyElement {
    let checked = reflected_bool(&field.value).unwrap_or(false);
    let command = field.value.binding.set_value(reflected_envelope(
        &field.value.type_path,
        (!checked).to_string(),
    ));
    kit::inspector_toggle(
        SharedString::from(format!(
            "reflected-toggle-{}",
            reflected_path_element_key(&field.value.binding.target.path)
        )),
        checked,
        false,
        theme,
    )
    .on_click(move |_, window, cx| {
        dispatch_reflected_command(window, cx, command.clone());
    })
    .into_any_element()
}

fn render_reflected_number(
    field: &ReflectedInspectionField,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let value = reflected_number(&field.value).unwrap_or_default();
    let input = render_authored_text_input(
        ReflectedEditTarget::Direct(field.value.binding.clone()),
        &field.value.type_path,
        &value,
        TextValueEncoding::RawRon,
        scope,
        window,
        cx,
    );
    kit::inspector_control(&theme)
        .px_1p5()
        .child(input)
        .when_some(field_suffix(field).map(str::to_owned), |this, suffix| {
            this.child(
                div()
                    .flex_none()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(9.5))
                    .text_color(theme.muted_foreground)
                    .child(suffix),
            )
        })
        .into_any_element()
}

#[derive(Clone, Copy)]
enum SliderEditKind {
    Float,
    Signed,
    Unsigned,
}

/// The numeric bounds a reflected slider is built against.
#[derive(Clone, Copy)]
struct SliderBounds {
    min: f32,
    max: f32,
    step: f32,
}

/// Look up (or first build) the keyed slider state for one reflected field.
///
/// The state owns the gpui slider entity and the sentinel copy of the value
/// last pushed into it, so a projection change can be told from a repaint.
fn reflected_slider_state(
    field: &ReflectedInspectionField,
    scope: &ReflectedControlScope,
    bounds: SliderBounds,
    value: f32,
    kind: SliderEditKind,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> Entity<ReflectedSliderState> {
    let SliderBounds { min, max, step } = bounds;
    let target = ReflectedEditTarget::Direct(field.value.binding.clone());
    let key = scope.state_key(&target, ReflectedControlKind::Slider);
    let type_path = field.value.type_path.clone();
    window.use_keyed_state(key, cx, move |window, cx| {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(min)
                .max(max)
                .step(step)
                .default_value(value)
        });
        let subscription = cx.subscribe_in(
            &slider,
            window,
            move |_: &mut ReflectedSliderState,
                  _: &Entity<SliderState>,
                  event: &SliderEvent,
                  window: &mut Window,
                  cx| {
                let SliderEvent::Release(SliderValue::Single(value)) = event else {
                    return;
                };
                let raw = match kind {
                    SliderEditKind::Float => f64::from(*value).to_string(),
                    // Rust has no checked f32 -> i64 conversion; `as` saturates
                    // at i64's bounds, so a signed field's slider clamps rather
                    // than wrapping, and truncation toward zero is the intent.
                    #[allow(clippy::cast_possible_truncation)]
                    SliderEditKind::Signed => (*value as i64).to_string(),
                    SliderEditKind::Unsigned => value.max(0.0).round().to_string(),
                };
                dispatch_reflected_command(
                    window,
                    cx,
                    target.set_value(reflected_envelope(&type_path, raw)),
                );
            },
        );
        ReflectedSliderState {
            slider,
            last_value: value,
            _subscription: subscription,
        }
    })
}

fn render_authored_slider(
    field: &ReflectedInspectionField,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> Option<gpui::AnyElement> {
    let range = field.widget.range.as_ref()?;
    let min = range.minimum.as_deref()?.parse::<f32>().ok()?;
    let max = range.maximum.as_deref()?.parse::<f32>().ok()?;
    if max <= min {
        return None;
    }
    let (value, kind) = match field.value.current.effective.as_ref()? {
        ReflectedValue::Scalar(ReflectedScalar::Float(value)) => {
            (value.parse::<f32>().ok()?, SliderEditKind::Float)
        }
        ReflectedValue::Scalar(ReflectedScalar::Signed(value)) => {
            (value.parse::<f32>().ok()?, SliderEditKind::Signed)
        }
        ReflectedValue::Scalar(ReflectedScalar::Unsigned(value)) => {
            (value.parse::<f32>().ok()?, SliderEditKind::Unsigned)
        }
        _ => return None,
    };
    let step = range
        .step
        .as_deref()
        .and_then(|step| step.parse::<f32>().ok())
        .unwrap_or_else(|| ((max - min) / 100.0).max(f32::EPSILON));
    let state = reflected_slider_state(
        field,
        scope,
        SliderBounds { min, max, step },
        value,
        kind,
        window,
        cx,
    );
    state.update(cx, |state, cx| {
        // `last_value` is the sentinel copy of the value last pushed into the
        // slider, so this is a "did the projection change" identity check, not
        // a numeric comparison: an epsilon would swallow small real edits.
        #[allow(clippy::float_cmp)]
        let changed = state.last_value != value;
        if changed {
            state.last_value = value;
            state
                .slider
                .update(cx, |slider, cx| slider.set_value(value, window, cx));
        }
    });
    let slider = state.read(cx).slider.clone();
    let theme = cx.theme().clone();
    let display = range
        .suffix
        .as_deref()
        .map_or_else(|| value.to_string(), |suffix| format!("{value} {suffix}"));
    Some(
        h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Slider::new(&slider).horizontal()),
            )
            .child(
                div()
                    .w(px(48.0))
                    .flex_none()
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(10.5))
                    .text_color(theme.foreground)
                    .child(display),
            )
            .into_any_element(),
    )
}

fn render_authored_vector_components(
    components: Vec<ReflectedVectorComponentData>,
    suffix: Option<&str>,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .children(
            components
                .into_iter()
                .map(|component| render_authored_vector_component(&component, scope, window, cx)),
        )
        .when_some(suffix.map(str::to_owned), |this, suffix| {
            this.child(
                div()
                    .flex_none()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(suffix),
            )
        })
        .into_any_element()
}

fn render_authored_vector_component(
    component: &ReflectedVectorComponentData,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let axis = match component.label.to_ascii_lowercase().as_str() {
        "x" | "r" | "0" => kit::InspectorAxis::X,
        "y" | "g" | "1" => kit::InspectorAxis::Y,
        "z" | "b" | "2" => kit::InspectorAxis::Z,
        _ => kit::InspectorAxis::W,
    };
    let value = reflected_number(&component.node).unwrap_or_default();
    let control = render_authored_text_input(
        ReflectedEditTarget::Direct(component.node.binding.clone()),
        &component.node.type_path,
        &value,
        TextValueEncoding::RawRon,
        scope,
        window,
        cx,
    );
    kit::inspector_axis_control(axis, control, cx.theme()).into_any_element()
}

fn render_authored_color_picker(
    node: &ReflectedValueNode,
    color: &ReflectedColorPickerData,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let target = ReflectedEditTarget::Direct(node.binding.clone());
    let key = scope.state_key(&target, ReflectedControlKind::ColorPicker);
    let state = window.use_keyed_state(key, cx, {
        let color = color.clone();
        move |window, cx| {
            let picker = cx.new(|cx| {
                ColorPickerState::new(window, cx).default_value(hsla_from_rgba(color.rgba))
            });
            let subscription = cx.subscribe_in(
                &picker,
                window,
                move |this: &mut ReflectedColorPickerState,
                      _: &Entity<ColorPickerState>,
                      event: &ColorPickerEvent,
                      window: &mut Window,
                      cx| {
                    let ColorPickerEvent::Change(Some(color)) = event else {
                        return;
                    };
                    let value = reflected_color_value(&this.value, rgba_from_hsla(*color));
                    dispatch_reflected_command(
                        window,
                        cx,
                        this.binding
                            .set_value(reflected_value_envelope(&value, &this.type_path)),
                    );
                },
            );
            ReflectedColorPickerState {
                picker,
                binding: color.binding,
                type_path: color.type_path,
                value: color.value,
                last_synced: color.rgba,
                _subscription: subscription,
            }
        }
    });
    let next = color.clone();
    state.update(cx, move |state, cx| {
        state.binding = next.binding;
        state.type_path = next.type_path;
        state.value = next.value;
        // `last_synced` is the sentinel copy of the channels last pushed into
        // the picker, so this is a "did the projection change" identity check,
        // not a numeric comparison on the color.
        #[allow(clippy::float_cmp)]
        let changed = state.last_synced != next.rgba;
        if changed {
            state.last_synced = next.rgba;
            state.picker.update(cx, |picker, cx| {
                picker.set_value(hsla_from_rgba(next.rgba), window, cx);
            });
        }
    });
    let picker = state.read(cx).picker.clone();
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(div().flex_none().child(ColorPicker::new(&picker).small()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .font_family("monospace")
                .text_size(px(10.0))
                .text_color(cx.theme().muted_foreground)
                .child(reflected_color_label(color.rgba)),
        )
        .into_any_element()
}

/// The choices a dropdown offers for one field, its currently selected value,
/// and whether those choices are enum variants (which write through as a
/// variant) rather than allowed strings.
///
/// `None` for a field that publishes no closed set to choose from.
fn reflected_dropdown_choices(
    field: &ReflectedInspectionField,
) -> Option<(Vec<ReflectedDropdownChoice>, Option<String>, bool)> {
    let resolved = match &field.widget.family {
        WidgetFamily::Enum if !field.widget.variants.is_empty() => (
            field
                .widget
                .variants
                .iter()
                .map(|value| ReflectedDropdownChoice {
                    label: humanize(value),
                    value: value.clone(),
                })
                .collect::<Vec<_>>(),
            reflected_variant(&field.value),
            true,
        ),
        WidgetFamily::Text if !field.widget.constraints.allowed_strings.is_empty() => (
            field
                .widget
                .constraints
                .allowed_strings
                .iter()
                .map(|value| ReflectedDropdownChoice {
                    label: value.clone(),
                    value: value.clone(),
                })
                .collect::<Vec<_>>(),
            reflected_string(&field.value),
            false,
        ),
        _ => return None,
    };
    Some(resolved)
}

fn render_reflected_dropdown_select(
    field: &ReflectedInspectionField,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> Option<gpui::AnyElement> {
    if !field_editable(field) {
        return None;
    }
    let (choices, selected, enum_values) = reflected_dropdown_choices(field)?;
    let target = ReflectedEditTarget::Direct(field.value.binding.clone());
    let key = scope.state_key(&target, ReflectedControlKind::DropdownSelect);
    let type_path = field.value.type_path.clone();
    let binding = field.value.binding.clone();
    let state = window.use_keyed_state(key, cx, {
        let choices = choices.clone();
        let selected = selected.clone();
        move |window, cx| {
            let selected_index = selected
                .as_ref()
                .and_then(|selected| choices.iter().position(|choice| &choice.value == selected))
                .map(IndexPath::new);
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(choices.clone()),
                    selected_index,
                    window,
                    cx,
                )
                .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |_: &mut ReflectedDropdownSelectState,
                      _: &Entity<SelectState<SearchableVec<ReflectedDropdownChoice>>>,
                      event: &SelectEvent<SearchableVec<ReflectedDropdownChoice>>,
                      window: &mut Window,
                      cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    let command = if enum_values {
                        binding.set_variant(value.clone(), None)
                    } else {
                        binding.set_value(reflected_string_envelope(&type_path, value))
                    };
                    dispatch_reflected_command(window, cx, command);
                },
            );
            ReflectedDropdownSelectState {
                select,
                choices,
                selected_value: selected,
                _subscription: subscription,
            }
        }
    });
    let next_choices = choices;
    state.update(cx, |state, cx| {
        let choices_changed = state.choices != next_choices;
        if choices_changed {
            state.choices.clone_from(&next_choices);
            state.select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(next_choices.clone()), window, cx);
            });
        }
        if choices_changed || state.selected_value != selected {
            state.selected_value.clone_from(&selected);
            state.select.update(cx, |select, cx| {
                if let Some(value) = selected.as_ref() {
                    select.set_selected_value(value, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
        }
    });
    let select = state.read(cx).select.clone();
    Some(
        div()
            .w_full()
            .child(
                Select::new(&select)
                    .small()
                    .placeholder("Select variant")
                    .search_placeholder("Search variants"),
            )
            .into_any_element(),
    )
}

fn render_authored_asset_path_control(
    node: &ReflectedValueNode,
    asset_type: &str,
    scope: &ReflectedControlScope,
    _window: &mut Window,
    cx: &Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let value = reflected_string(node).unwrap_or_default();
    let display = if value.is_empty() {
        "None".to_owned()
    } else {
        crate::naming::display_name(&value).into_owned()
    };
    let options = cx
        .try_global::<EditorAssetBrowserStatus>()
        .map(|status| {
            status
                .entries
                .iter()
                .filter(|entry| entry.status != AssetBrowserEntryStatus::Deleted)
                .filter(|entry| {
                    entry.schema_type.as_deref().is_none_or(|schema_type| {
                        schema_type.eq_ignore_ascii_case(asset_type)
                            || schema_type.ends_with(asset_type)
                    })
                })
                .map(|entry| entry.source_path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let target = ReflectedEditTarget::Direct(node.binding.clone());
    let selected_value = value.clone();
    let browse = Button::new(scope.state_key(&target, ReflectedControlKind::AssetBrowse))
        .icon(IconName::Ellipsis)
        .ghost()
        .small()
        .rounded_full()
        .tooltip("Browse cooked products")
        .dropdown_menu({
            let target = target.clone();
            let type_path = node.type_path.clone();
            move |mut popup, _window, _cx| {
                if !selected_value.is_empty() {
                    let command = target.set_value(reflected_string_envelope(&type_path, ""));
                    popup =
                        popup.item(PopupMenuItem::new("None").on_click(move |_, window, cx| {
                            dispatch_reflected_command(window, cx, command.clone());
                        }));
                    popup = popup.separator();
                }
                for option in &options {
                    let command =
                        target.set_value(reflected_string_envelope(&type_path, option.as_str()));
                    popup = popup.item(
                        PopupMenuItem::new(crate::naming::display_name(option).into_owned())
                            .checked(option == &selected_value)
                            .on_click(move |_, window, cx| {
                                dispatch_reflected_command(window, cx, command.clone());
                            }),
                    );
                }
                popup.min_w(px(230.0)).max_h(px(360.0))
            }
        });
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .child(
            kit::inspector_control(&theme)
                .flex_1()
                .min_w_0()
                .px_1p5()
                .gap_1p5()
                .child(kit::material_symbol_icon(
                    asset_icon(asset_type),
                    15.0,
                    theme.accent,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(px(10.5))
                        .text_color(if value.is_empty() {
                            theme.muted_foreground
                        } else {
                            theme.foreground
                        })
                        .child(display),
                ),
        )
        .child(browse)
        .into_any_element()
}

fn render_authored_object_ref_control(
    node: &ReflectedValueNode,
    object_type: &str,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let value = reflected_string(node).unwrap_or_default();
    let target = ReflectedEditTarget::Direct(node.binding.clone());
    let input = render_authored_text_input(
        target.clone(),
        &node.type_path,
        &value,
        TextValueEncoding::ReflectedString,
        scope,
        window,
        cx,
    );
    let options = cx
        .try_global::<EditorAuthoredOutline>()
        .map(|outline| {
            outline
                .data
                .documents
                .iter()
                .flat_map(|document| &document.objects)
                .filter(|object| {
                    object_type.is_empty()
                        || object.schema_type == object_type
                        || object.schema_type.ends_with(object_type)
                })
                .map(|object| {
                    (
                        object
                            .display_name
                            .clone()
                            .unwrap_or_else(|| object.object_id.clone()),
                        object.object_id.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let browse = Button::new(scope.state_key(&target, ReflectedControlKind::ObjectBrowse))
        .icon(IconName::Ellipsis)
        .ghost()
        .small()
        .rounded_full()
        .tooltip("Select object")
        .dropdown_menu({
            let type_path = node.type_path.clone();
            move |mut popup, _window, _cx| {
                for (label, object_id) in &options {
                    let command =
                        target.set_value(reflected_string_envelope(&type_path, object_id));
                    popup = popup.item(
                        PopupMenuItem::new(label.clone())
                            .checked(object_id == &value)
                            .on_click(move |_, window, cx| {
                                dispatch_reflected_command(window, cx, command.clone());
                            }),
                    );
                }
                popup.min_w(px(230.0)).max_h(px(360.0))
            }
        });
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .child(input)
        .child(browse)
        .into_any_element()
}

fn render_reflected_list_control(node: &ReflectedValueNode, theme: &Theme) -> gpui::AnyElement {
    let count = node
        .children
        .iter()
        .filter(|child| matches!(child, ReflectedInspectionChild::ListItem(_)))
        .count();
    let command = list_insert_template(node).map(|value| {
        node.binding.list_insert(
            u32::try_from(count).unwrap_or(u32::MAX),
            reflected_value_envelope(&value, &value_type_path(node)),
        )
    });
    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("{count} items")),
        )
        .when_some(command, |this, command| {
            this.child(reflected_command_button(
                command,
                "+",
                "Insert item from reflected template",
                theme,
            ))
        })
        .into_any_element()
}

fn render_reflected_map_control(
    node: &ReflectedValueNode,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let entries = node
        .children
        .iter()
        .filter_map(|child| match child {
            ReflectedInspectionChild::MapEntry(entry) => Some(entry),
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(template) = entries.first() else {
        return div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("0 entries — reflected value template unavailable")
            .into_any_element();
    };
    let key_type = template.binding.key.type_path.clone();
    let key_encoding = match &template.key {
        ReflectedValue::Scalar(ReflectedScalar::String(_)) => TextValueEncoding::ReflectedString,
        ReflectedValue::Scalar(
            ReflectedScalar::Bool(_)
            | ReflectedScalar::Signed(_)
            | ReflectedScalar::Unsigned(_)
            | ReflectedScalar::Float(_),
        ) => TextValueEncoding::RawRon,
        _ => {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "{} entries — complex map keys are read-only",
                    entries.len()
                ))
                .into_any_element();
        }
    };
    let value = template.value_envelope.clone();
    let binding = node.binding.clone();
    let target = ReflectedEditTarget::Direct(binding.clone());
    let key = scope.state_key(&target, ReflectedControlKind::MapKeyInput);
    let state = window.use_keyed_state(key, cx, move |window, cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("key"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |_: &mut ReflectedTextInputState,
                  input: &Entity<InputState>,
                  event: &InputEvent,
                  window: &mut Window,
                  cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let key = input.read(cx).value().trim().to_owned();
                    if !key.is_empty() {
                        dispatch_reflected_command(
                            window,
                            cx,
                            binding.map_insert(
                                reflected_text_envelope(&key_type, &key, key_encoding),
                                value.clone(),
                            ),
                        );
                        input.update(cx, |input, cx| {
                            input.set_value(String::new(), window, cx);
                        });
                    }
                }
            },
        );
        ReflectedTextInputState {
            input,
            _subscription: subscription,
        }
    });
    let input = state.read(cx).input.clone();
    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} entries", entries.len())),
        )
        .child(
            div().w_24().child(
                Input::new(&input)
                    .small()
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
            ),
        )
        .into_any_element()
}

fn render_reflected_optional_control(node: &ReflectedValueNode, theme: &Theme) -> gpui::AnyElement {
    let current = node.current.effective.as_ref();
    let is_some = matches!(current, Some(ReflectedValue::Optional(Some(_))));
    let command = if is_some {
        Some(
            node.binding
                .set_value(reflected_envelope(&node.type_path, "None")),
        )
    } else {
        optional_default(node).map(|value| {
            node.binding.set_value(reflected_envelope(
                &node.type_path,
                format!("Some({})", reflected_value_ron(value)),
            ))
        })
    };
    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(if is_some { "Some" } else { "None" }),
        )
        .when_some(command, |this, command| {
            this.child(reflected_command_button(
                command,
                if is_some { "clear" } else { "initialize" },
                "Toggle optional value",
                theme,
            ))
        })
        .into_any_element()
}

fn render_authored_value_children(
    children: Vec<ReflectedInspectionChild>,
    editable: bool,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let list_count = u32::try_from(
        children
            .iter()
            .filter(|child| matches!(child, ReflectedInspectionChild::ListItem(_)))
            .count(),
    )
    .unwrap_or(u32::MAX);
    v_flex()
        .ml_2()
        .gap_1()
        .children(children.into_iter().map(|child| {
            render_authored_value_child(child, editable, list_count, scope, window, cx)
        }))
        .into_any_element()
}

/// One child row under a reflected value: a nested field, a tuple element, a
/// list item with its move/remove commands, a map entry, an enum variant's
/// own children, or an `Option`'s `Some` payload.
fn render_authored_value_child(
    child: ReflectedInspectionChild,
    editable: bool,
    list_count: u32,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    match child {
        ReflectedInspectionChild::Field(field) => render_reflected_field(&field, scope, window, cx),
        ReflectedInspectionChild::TupleElement { index, value } => {
            let value = *value;
            let binding = value.binding.clone();
            render_authored_value_node(
                ReflectedValueRow {
                    label: index.to_string(),
                    target: ReflectedEditTarget::Direct(binding),
                    editable,
                    commands: None,
                },
                &value,
                scope,
                window,
                cx,
            )
        }
        ReflectedInspectionChild::ListItem(item) => {
            let parent = parent_binding(&item.value.binding);
            let value = *item.value;
            let binding = value.binding.clone();
            let mut commands = Vec::new();
            if editable && let Some(parent) = parent {
                if let Some(up) = item.index.checked_sub(1) {
                    commands.push((parent.list_move(item.index, up), "^"));
                }
                if item.index + 1 < list_count {
                    commands.push((parent.list_move(item.index, item.index + 1), "v"));
                }
                commands.push((parent.list_remove(item.index), "remove"));
            }
            render_authored_value_node(
                ReflectedValueRow {
                    label: format!("[{}]", item.index),
                    target: ReflectedEditTarget::Direct(binding),
                    editable,
                    commands: Some(commands),
                },
                &value,
                scope,
                window,
                cx,
            )
        }
        ReflectedInspectionChild::MapEntry(entry) => {
            let entry_editable = editable && entry.value.children.is_empty();
            let command = editable.then(|| entry.binding.remove());
            let target = ReflectedEditTarget::MapEntry(entry.binding.clone());
            render_authored_value_node(
                ReflectedValueRow {
                    label: reflected_value_display(&entry.key),
                    target,
                    editable: entry_editable,
                    commands: command.map(|command| vec![(command, "remove")]),
                },
                &entry.value,
                scope,
                window,
                cx,
            )
        }
        ReflectedInspectionChild::Variant(variant) => v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().foreground)
                    .child(humanize(&variant.name)),
            )
            .child(render_authored_value_children(
                variant.fields,
                editable,
                scope,
                window,
                cx,
            ))
            .into_any_element(),
        ReflectedInspectionChild::OptionalSome(value) => {
            let value = *value;
            let binding = value.binding.clone();
            render_authored_value_node(
                ReflectedValueRow {
                    label: "Some".to_owned(),
                    target: ReflectedEditTarget::Direct(binding),
                    editable,
                    commands: None,
                },
                &value,
                scope,
                window,
                cx,
            )
        }
    }
}
/// One reflected value row's own shape: the label it shows, the edit target its
/// control writes through, whether that control is live, and the list/map row
/// commands rendered after it.
struct ReflectedValueRow {
    label: String,
    target: ReflectedEditTarget,
    editable: bool,
    commands: Option<Vec<(PrefabEditCommand, &'static str)>>,
}

fn render_authored_value_node(
    row: ReflectedValueRow,
    node: &ReflectedValueNode,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let ReflectedValueRow {
        label,
        target,
        editable,
        commands: row_commands,
    } = row;
    let theme = cx.theme().clone();
    let children = node.children.clone();
    let family = WidgetFamily::from(&node.kind);
    let direct_edit = editable && children.is_empty();
    let control = if direct_edit {
        render_reflected_node_control(node, target, &family, scope, window, cx)
    } else {
        render_reflected_value_label(node, &theme)
    };
    let mut row = h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(86.0))
                .flex_none()
                .min_w_0()
                .text_xs()
                .text_color(theme.foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control));
    if let Some(commands) = row_commands {
        for (command, label) in commands {
            row = row.child(reflected_command_button(command, label, label, &theme));
        }
    }
    v_flex()
        .w_full()
        .gap_1()
        .child(row)
        .when(!children.is_empty(), |this| {
            this.child(render_authored_value_children(
                children, editable, scope, window, cx,
            ))
        })
        .into_any_element()
}

fn render_reflected_node_control(
    node: &ReflectedValueNode,
    target: ReflectedEditTarget,
    family: &WidgetFamily,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    match family {
        WidgetFamily::Bool => {
            let checked = reflected_bool(node).unwrap_or(false);
            let command =
                target.set_value(reflected_envelope(&node.type_path, (!checked).to_string()));
            reflected_command_button(
                command,
                if checked { "on" } else { "off" },
                "Toggle value",
                &theme,
            )
            .into_any_element()
        }
        WidgetFamily::Number => render_authored_text_input(
            target,
            &node.type_path,
            &reflected_number(node).unwrap_or_default(),
            TextValueEncoding::RawRon,
            scope,
            window,
            cx,
        ),
        WidgetFamily::Text | WidgetFamily::Asset { .. } | WidgetFamily::Object { .. } => {
            render_authored_text_input(
                target,
                &node.type_path,
                &reflected_string(node).unwrap_or_default(),
                TextValueEncoding::ReflectedString,
                scope,
                window,
                cx,
            )
        }
        _ => render_reflected_value_label(node, &theme),
    }
}

fn render_authored_textarea_input(
    target: ReflectedEditTarget,
    type_path: &str,
    value: &str,
    min_rows: usize,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let key = scope.state_key(&target, ReflectedControlKind::TextareaInput);
    let type_path = type_path.to_owned();
    let state = window.use_keyed_state(key, cx, {
        let value = value.to_owned();
        move |window, cx| {
            let max_rows = min_rows.max(TEXTAREA_MAX_ROWS);
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .auto_grow(min_rows, max_rows)
                    .default_value(value.clone())
            });
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |_: &mut ReflectedTextInputState,
                      input: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx| {
                    if matches!(event, InputEvent::Blur) {
                        let value = input.read(cx).value().to_string();
                        dispatch_reflected_command(
                            window,
                            cx,
                            target.set_value(reflected_string_envelope(&type_path, &value)),
                        );
                    }
                },
            );
            ReflectedTextInputState {
                input,
                _subscription: subscription,
            }
        }
    });
    let input = state.read(cx).input.clone();
    sync_text_input(&input, value, window, cx);
    div()
        .w_full()
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .bordered(false)
                .focus_bordered(false),
        )
        .into_any_element()
}

fn render_authored_text_input(
    target: ReflectedEditTarget,
    type_path: &str,
    value: &str,
    encoding: TextValueEncoding,
    scope: &ReflectedControlScope,
    window: &mut Window,
    cx: &mut Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let key = scope.state_key(&target, ReflectedControlKind::TextInput);
    let type_path = type_path.to_owned();
    let state = window.use_keyed_state(key, cx, {
        let value = value.to_owned();
        move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |_: &mut ReflectedTextInputState,
                      input: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        let value = input.read(cx).value().to_string();
                        dispatch_reflected_command(
                            window,
                            cx,
                            target.set_value(reflected_text_envelope(&type_path, &value, encoding)),
                        );
                    }
                },
            );
            ReflectedTextInputState {
                input,
                _subscription: subscription,
            }
        }
    });
    let input = state.read(cx).input.clone();
    sync_text_input(&input, value, window, cx);
    div()
        .flex_1()
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .bordered(false)
                .focus_bordered(false),
        )
        .into_any_element()
}

fn sync_text_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value().as_ref() != value {
        input.update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx);
        });
    }
}

fn render_reflected_actions(
    actions: Vec<String>,
    binding: &ReflectedEditBinding,
    theme: &Theme,
) -> gpui::AnyElement {
    if actions.is_empty() {
        return div().into_any_element();
    }
    h_flex()
        .w_full()
        .items_center()
        .flex_wrap()
        .gap_1()
        .px_3()
        .py(px(4.0))
        .children(actions.into_iter().map(|action_id| {
            let id = SharedString::from(format!(
                "reflected-action-{}-{}",
                reflected_path_element_key(&binding.target.path),
                sanitize_element_key(&action_id)
            ));
            let hover = theme.secondary_hover;
            let label = humanize(&action_id);
            let binding = binding.clone();
            kit::field_button(theme)
                .id(id)
                .hover(move |this| this.bg(hover))
                .cursor_pointer()
                .child(label)
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(crate::actions::InvokeReflectedInspectorAction {
                            binding: binding.clone(),
                            action_id: action_id.clone(),
                        }),
                        cx,
                    );
                })
        }))
        .into_any_element()
}

fn render_validation_indicator(
    validation: &ReflectedValidationState,
    theme: &Theme,
) -> gpui::AnyElement {
    let message = validation
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("\n");
    div()
        .id(SharedString::from(format!(
            "reflected-validation-{}",
            sanitize_element_key(&message)
        )))
        .tooltip(move |window, cx| Tooltip::new(message.clone()).build(window, cx))
        .child(kit::material_symbol_icon("error", 13.0, theme.danger))
        .into_any_element()
}

fn render_prefab_view(inspection: &ReflectedEntityInspection, theme: &Theme) -> gpui::AnyElement {
    let mut contents = vec![
        ("Source", inspection.selection.source_path.clone()),
        ("Entity", inspection.selection.entity_alias.clone()),
        ("Document version", inspection.document_version.to_string()),
        (
            "Schema catalog hash",
            inspection
                .registry_schema_catalog_hash
                .iter()
                .fold(String::new(), |mut hex, byte| {
                    let _ = write!(hex, "{byte:02x}");
                    hex
                }),
        ),
        ("Revision", inspection.revision.to_string()),
        ("Overrides", inspection.overrides.len().to_string()),
    ];
    contents.extend(
        inspection
            .components
            .iter()
            .map(|component| ("Component", component.model.type_label.clone())),
    );
    v_flex()
        .w_full()
        .child(kit::panel_toolbar(theme).child(kit::section_label("Prefab", theme)))
        .children(
            contents
                .into_iter()
                .enumerate()
                .map(|(index, (kind, name))| {
                    h_flex()
                        .id(SharedString::from(format!("prefab-content-{index}")))
                        .h(px(27.0))
                        .items_center()
                        .gap_2()
                        .px_3()
                        .child(kit::material_symbol_icon(
                            if kind == "Component" {
                                "deployed_code"
                            } else {
                                "widgets"
                            },
                            15.0,
                            theme.accent,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(11.5))
                                .text_color(theme.foreground)
                                .child(name),
                        )
                        .child(kit::meta_text(kind, theme))
                }),
        )
        .into_any_element()
}

fn render_add_component_menu(
    schemas: Vec<CreatableAuthoredSchemaData>,
    existing_schemas: &[String],
    theme: &Theme,
) -> gpui::AnyElement {
    if schemas.is_empty() {
        return div().into_any_element();
    }
    let existing_schemas = existing_schemas.to_vec();
    let icon_color = theme.accent;
    div()
        .w_full()
        .p_3()
        .flex()
        .justify_center()
        .child(
            Button::new("inspector-add-component")
                .small()
                .icon(IconName::Plus)
                .label("Add Component")
                .bg(theme.secondary)
                .border_1()
                .border_color(theme.border)
                .dropdown_menu(move |mut popup, _window, _cx| {
                    for schema in schemas.iter().cloned() {
                        let disabled_reason =
                            component_add_disabled_reason(&schema, &schemas, &existing_schemas);
                        let icon = schema
                            .icon
                            .clone()
                            .unwrap_or_else(|| "extension".to_owned());
                        let schema_type = schema.schema_type;
                        let item = if let Some(reason) = disabled_reason {
                            PopupMenuItem::new(format!("{} — {reason}", schema.label))
                                .icon(kit::material_symbol_icon(&icon, 15.0, icon_color))
                                .disabled(true)
                        } else {
                            PopupMenuItem::new(schema.label)
                                .icon(kit::material_symbol_icon(&icon, 15.0, icon_color))
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::actions::AddAuthoredComponent {
                                            schema_type: schema_type.clone(),
                                        }),
                                        cx,
                                    );
                                })
                        };
                        popup = popup.item(item);
                    }
                    popup.min_w(px(220.0)).max_h(px(360.0))
                }),
        )
        .into_any_element()
}

fn component_add_disabled_reason(
    candidate: &CreatableAuthoredSchemaData,
    catalog: &[CreatableAuthoredSchemaData],
    existing_schemas: &[String],
) -> Option<String> {
    let mut components = existing_schemas
        .iter()
        .filter_map(|schema| {
            catalog
                .iter()
                .find(|candidate| &candidate.schema_type == schema)
        })
        .collect::<Vec<_>>();
    components.push(candidate);
    for component in &components {
        let Some(metadata) = &component.component_capabilities else {
            continue;
        };
        for required in &metadata.requires {
            if !components.iter().any(|provider| {
                provider
                    .component_capabilities
                    .as_ref()
                    .is_some_and(|metadata| metadata.provides.contains(required))
            }) {
                return Some(format!("requires {required}"));
            }
        }
    }
    for (index, component) in components.iter().enumerate() {
        let Some(metadata) = &component.component_capabilities else {
            continue;
        };
        for incompatible in &metadata.incompatible {
            if components
                .iter()
                .enumerate()
                .any(|(candidate_index, provider)| {
                    candidate_index != index
                        && provider
                            .component_capabilities
                            .as_ref()
                            .is_some_and(|metadata| metadata.provides.contains(incompatible))
                })
            {
                return Some(format!("incompatible with {incompatible}"));
            }
        }
    }
    None
}

fn render_inspector_fallback(theme: &Theme) -> gpui::AnyElement {
    kit::empty_state(
        "No reflected entity selected",
        Some("Select an entity in Hierarchy or the viewport to edit its components".to_owned()),
        theme,
    )
    .into_any_element()
}

fn dispatch_reflected_command(window: &mut Window, cx: &mut App, command: PrefabEditCommand) {
    window.dispatch_action(
        Box::new(crate::actions::ApplyReflectedPrefabEdit { command }),
        cx,
    );
}

fn reflected_command_button(
    command: PrefabEditCommand,
    label: impl Into<String>,
    tooltip: impl Into<SharedString>,
    theme: &Theme,
) -> impl IntoElement {
    let label = label.into();
    let id = SharedString::from(format!(
        "reflected-command-{}-{}",
        sanitize_element_key(&format!("{command:?}")),
        sanitize_element_key(&label)
    ));
    let hover = theme.secondary_hover;
    let tooltip: SharedString = tooltip.into();
    kit::field_button(theme)
        .id(id)
        .hover(move |this| this.bg(hover))
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(label)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            dispatch_reflected_command(window, cx, command.clone());
        })
}

const fn field_editable(field: &ReflectedInspectionField) -> bool {
    !field.read_only && !field.hidden
}

const fn reflected_value(node: &ReflectedValueNode) -> Option<&ReflectedValue> {
    node.current.effective.as_ref()
}

fn reflected_bool(node: &ReflectedValueNode) -> Option<bool> {
    match reflected_value(node)? {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn reflected_number(node: &ReflectedValueNode) -> Option<String> {
    match reflected_value(node)? {
        ReflectedValue::Scalar(
            ReflectedScalar::Signed(value)
            | ReflectedScalar::Unsigned(value)
            | ReflectedScalar::Float(value),
        ) => Some(value.clone()),
        _ => None,
    }
}

fn reflected_string(node: &ReflectedValueNode) -> Option<String> {
    match reflected_value(node)? {
        ReflectedValue::Scalar(ReflectedScalar::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn reflected_variant(node: &ReflectedValueNode) -> Option<String> {
    match reflected_value(node)? {
        ReflectedValue::Enum { variant, .. } => Some(variant.clone()),
        _ => None,
    }
}

fn field_display_value(field: &ReflectedInspectionField) -> String {
    let display = field
        .value
        .current
        .effective
        .as_ref()
        .map_or_else(|| "Unset".to_owned(), reflected_value_display);
    match field_suffix(field) {
        Some(suffix) => format!("{display} {suffix}"),
        None => display,
    }
}

fn field_suffix(field: &ReflectedInspectionField) -> Option<&str> {
    field.widget.range.as_ref()?.suffix.as_deref()
}

fn render_reflected_value_label(node: &ReflectedValueNode, theme: &Theme) -> gpui::AnyElement {
    div()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(
            node.current
                .effective
                .as_ref()
                .map_or_else(|| "Unset".to_owned(), reflected_value_display),
        )
        .into_any_element()
}

fn reflected_value_display(value: &ReflectedValue) -> String {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => value.to_string(),
        // A scalar and an opaque RON blob both already hold the exact text the
        // producer sent, so the summary shows it verbatim.
        ReflectedValue::Scalar(
            ReflectedScalar::Signed(value)
            | ReflectedScalar::Unsigned(value)
            | ReflectedScalar::Float(value)
            | ReflectedScalar::String(value),
        )
        | ReflectedValue::OpaqueRon(value) => value.clone(),
        ReflectedValue::Struct(fields) => format!("{} fields", fields.len()),
        ReflectedValue::Tuple(values) => format!("{} values", values.len()),
        ReflectedValue::List(values) => format!("{} items", values.len()),
        ReflectedValue::Map(values) => format!("{} entries", values.len()),
        ReflectedValue::Enum { variant, .. } => humanize(variant),
        ReflectedValue::Optional(Some(_)) => "Some".to_owned(),
        ReflectedValue::Optional(None) => "None".to_owned(),
        ReflectedValue::Unit => "()".to_owned(),
        ReflectedValue::Encoded(value) => format!("{} bytes", value.payload.len()),
    }
}

fn reflected_value_ron(value: &ReflectedValue) -> String {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => value.to_string(),
        // A numeric scalar and an opaque RON blob are both already RON text;
        // re-encoding either would double-quote or double-escape it.
        ReflectedValue::Scalar(
            ReflectedScalar::Signed(value)
            | ReflectedScalar::Unsigned(value)
            | ReflectedScalar::Float(value),
        )
        | ReflectedValue::OpaqueRon(value) => value.clone(),
        ReflectedValue::Scalar(ReflectedScalar::String(value)) => format!("{value:?}"),
        ReflectedValue::Struct(fields) => format!(
            "({})",
            fields
                .iter()
                .map(|(name, value)| format!("{name}:{}", reflected_value_ron(value)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Tuple(values) => format!(
            "({}{})",
            values
                .iter()
                .map(reflected_value_ron)
                .collect::<Vec<_>>()
                .join(","),
            if values.len() == 1 { "," } else { "" }
        ),
        ReflectedValue::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(reflected_value_ron)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Map(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|entry| format!(
                    "{}:{}",
                    reflected_value_ron(&entry.key),
                    reflected_value_ron(&entry.value)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Enum { variant, fields } if fields.is_empty() => variant.clone(),
        // A variant's fields carry the names they were declared under: indices
        // for a tuple-shaped variant, which is spelled positionally, and real
        // field names for a struct-shaped one, which the producer only accepts
        // spelled with them.
        ReflectedValue::Enum { variant, fields } => format!(
            "{variant}({})",
            fields
                .iter()
                .map(|(name, value)| {
                    let value = reflected_value_ron(value);
                    if name.parse::<usize>().is_ok() {
                        value
                    } else {
                        format!("{name}:{value}")
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Optional(Some(value)) => {
            format!("Some({})", reflected_value_ron(value))
        }
        ReflectedValue::Optional(None) => "None".to_owned(),
        ReflectedValue::Unit => "()".to_owned(),
        ReflectedValue::Encoded(value) => String::from_utf8_lossy(&value.payload).into_owned(),
    }
}

fn reflected_value_envelope(value: &ReflectedValue, type_path: &str) -> ReflectedValueEnvelope {
    match value {
        ReflectedValue::Encoded(envelope) if envelope.type_path == type_path => envelope.clone(),
        _ => reflected_envelope(type_path, reflected_value_ron(value)),
    }
}

fn reflected_envelope(type_path: &str, raw: impl Into<String>) -> ReflectedValueEnvelope {
    ReflectedValueEnvelope {
        type_path: type_path.to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload: raw.into().into_bytes(),
    }
}

fn reflected_string_envelope(type_path: &str, value: impl AsRef<str>) -> ReflectedValueEnvelope {
    reflected_envelope(type_path, format!("{:?}", value.as_ref()))
}

fn reflected_text_envelope(
    type_path: &str,
    value: &str,
    encoding: TextValueEncoding,
) -> ReflectedValueEnvelope {
    match encoding {
        TextValueEncoding::ReflectedString => reflected_string_envelope(type_path, value),
        TextValueEncoding::RawRon => reflected_envelope(type_path, value.trim()),
    }
}

fn reflected_vector_components(
    node: &ReflectedValueNode,
    dimensions: usize,
) -> Vec<ReflectedVectorComponentData> {
    node.children
        .iter()
        .filter_map(|child| match child {
            ReflectedInspectionChild::Field(field) => Some(ReflectedVectorComponentData {
                label: field.name.clone(),
                node: field.value.clone(),
            }),
            ReflectedInspectionChild::TupleElement { index, value } => {
                Some(ReflectedVectorComponentData {
                    label: reflected_component_label(*index).to_owned(),
                    node: (**value).clone(),
                })
            }
            _ => None,
        })
        .take(dimensions)
        .collect()
}

fn reflected_color_picker_data(node: &ReflectedValueNode) -> Option<ReflectedColorPickerData> {
    let components = reflected_vector_components(node, 4);
    if components.len() < 3 {
        return None;
    }
    let mut rgba = [0.0_f32, 0.0, 0.0, 1.0];
    for (index, component) in components.into_iter().enumerate() {
        rgba[index] = reflected_number(&component.node)?.parse().ok()?;
    }
    Some(ReflectedColorPickerData {
        rgba,
        binding: node.binding.clone(),
        type_path: node.type_path.clone(),
        value: node.current.effective.clone()?,
    })
}

fn reflected_color_value(template: &ReflectedValue, rgba: [f32; 4]) -> ReflectedValue {
    let mut value = template.clone();
    let values = match &mut value {
        ReflectedValue::Struct(fields) => fields
            .iter_mut()
            .map(|(_, value)| value)
            .collect::<Vec<_>>(),
        ReflectedValue::Tuple(values) => values.iter_mut().collect::<Vec<_>>(),
        _ => return value,
    };
    for (value, channel) in values.into_iter().zip(rgba) {
        *value = ReflectedValue::Scalar(ReflectedScalar::Float(f64::from(channel).to_string()));
    }
    value
}

fn hsla_from_rgba(rgba: [f32; 4]) -> Hsla {
    Hsla::from(Rgba {
        r: rgba[0],
        g: rgba[1],
        b: rgba[2],
        a: rgba[3],
    })
}

fn rgba_from_hsla(color: Hsla) -> [f32; 4] {
    let rgba = Rgba::from(color);
    [rgba.r, rgba.g, rgba.b, rgba.a]
}

/// One `0.0..=1.0` color channel as its 8-bit component.
///
/// The clamp bounds the product to `0.0..=255.0` before the narrowing, so
/// neither truncation nor a lost sign is reachable; Rust offers no checked
/// `f32` -> `u8` conversion to express that with instead.
fn color_component(channel: f32) -> u8 {
    // Bounded by the clamp on the same line — see the doc comment above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (channel.clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

fn reflected_color_label(rgba: [f32; 4]) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color_component(rgba[0]),
        color_component(rgba[1]),
        color_component(rgba[2]),
        color_component(rgba[3]),
    )
}

fn list_insert_template(node: &ReflectedValueNode) -> Option<ReflectedValue> {
    node.children
        .iter()
        .find_map(|child| match child {
            ReflectedInspectionChild::ListItem(item) => item.value.current.effective.clone(),
            _ => None,
        })
        .or_else(|| match node.default.value.as_ref() {
            Some(ReflectedValue::List(values)) => values.first().cloned(),
            _ => None,
        })
}

fn optional_default(node: &ReflectedValueNode) -> Option<&ReflectedValue> {
    match node.default.value.as_ref()? {
        ReflectedValue::Optional(Some(value)) => Some(value),
        _ => None,
    }
}

fn value_type_path(node: &ReflectedValueNode) -> String {
    node.children
        .iter()
        .find_map(|child| match child {
            ReflectedInspectionChild::ListItem(item) => Some(item.value.type_path.clone()),
            _ => None,
        })
        .or_else(|| generic_arguments(&node.type_path).into_iter().next())
        .unwrap_or_else(|| "()".to_owned())
}

fn generic_arguments(type_path: &str) -> Vec<String> {
    let Some(start) = type_path.find('<') else {
        return Vec::new();
    };
    let Some(end) = type_path.rfind('>') else {
        return Vec::new();
    };
    let mut depth = 0_u32;
    let mut item_start = start + 1;
    let mut values = Vec::new();
    for (offset, character) in type_path[start + 1..end].char_indices() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let item_end = start + 1 + offset;
                values.push(type_path[item_start..item_end].trim().to_owned());
                item_start = item_end + 1;
            }
            _ => {}
        }
    }
    values.push(type_path[item_start..end].trim().to_owned());
    values
}

fn parent_binding(binding: &ReflectedEditBinding) -> Option<ReflectedEditBinding> {
    let mut target = binding.target.clone();
    target.path.segments.pop()?;
    Some(ReflectedEditBinding::new(target))
}

const fn reflected_component_label(index: u32) -> &'static str {
    match index {
        0 => "X",
        1 => "Y",
        2 => "Z",
        _ => "W",
    }
}

fn asset_icon(asset_type: &str) -> &'static str {
    match asset_type.to_ascii_lowercase().as_str() {
        "mesh" | "model" => "deployed_code",
        "material" => "palette",
        "texture" => "image",
        "prefab" => "widgets",
        "audio" => "graphic_eq",
        _ => "attachment",
    }
}

fn humanize(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_lowercase = false;
    for character in value.chars() {
        if character == '_' || character == '-' {
            output.push(' ');
            previous_lowercase = false;
        } else {
            if character.is_uppercase() && previous_lowercase {
                output.push(' ');
            }
            output.push(character);
            previous_lowercase = character.is_lowercase();
        }
    }
    let mut chars = output.chars();
    let Some(first) = chars.next() else {
        return output;
    };
    first.to_uppercase().collect::<String>() + chars.as_str()
}

fn push_control_key_part(key: &mut String, part: &str) {
    let _ = write!(key, "{}:{part};", part.len());
}

fn push_reflected_path_key(key: &mut String, path: &ReflectedPath) {
    push_control_key_part(key, &path.component_type_path);
    for segment in &path.segments {
        match segment {
            ReflectedPathSegment::Field(name) => {
                key.push('f');
                push_control_key_part(key, name);
            }
            ReflectedPathSegment::Variant(name) => {
                key.push('v');
                push_control_key_part(key, name);
            }
            ReflectedPathSegment::TupleIndex(index) => {
                let _ = write!(key, "t{index};");
            }
            ReflectedPathSegment::ListIndex(index) => {
                let _ = write!(key, "i{index};");
            }
        }
    }
}

fn reflected_path_element_key(path: &ReflectedPath) -> String {
    let mut key = sanitize_element_key(&path.component_type_path);
    for segment in &path.segments {
        key.push('-');
        match segment {
            ReflectedPathSegment::Field(name) => {
                key.push('f');
                key.push_str(&sanitize_element_key(name));
            }
            ReflectedPathSegment::Variant(name) => {
                key.push('v');
                key.push_str(&sanitize_element_key(name));
            }
            ReflectedPathSegment::TupleIndex(index) => {
                let _ = write!(key, "t{index}");
            }
            ReflectedPathSegment::ListIndex(index) => {
                let _ = write!(key, "i{index}");
            }
        }
    }
    key
}

fn sanitize_element_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

impl Focusable for AuthoredInspector {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for AuthoredInspector {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        h_flex()
            .h_full()
            .items_center()
            .text_size(px(11.5))
            .text_color(cx.theme().muted_foreground)
            .child("Inspector")
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![
            Button::new("inspector-lock-selection")
                .icon(kit::material_symbol_icon(
                    "lock_open",
                    16.0,
                    cx.theme().muted_foreground,
                ))
                .ghost()
                .small()
                .disabled(true)
                .tooltip("Selection locking is not available for reflected selections"),
        ])
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        Some(PanelControl::Both)
    }

    fn dropdown_menu(
        &mut self,
        menu: gpui_component::menu::PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) -> gpui_component::menu::PopupMenu {
        menu.item(
            PopupMenuItem::new("Refresh Inspection").on_click(|_, window, cx| {
                window.dispatch_action(Box::new(crate::actions::RefreshReflectedInspection), cx);
            }),
        )
    }

    fn dump(&self, _cx: &App) -> PanelState {
        let value = serde_json::to_value(InspectorPanelState {
            active_tab: self.active_tab,
            collapsed_component_types: self.collapsed_component_types.clone(),
        })
        .unwrap_or(serde_json::Value::Null);
        PanelState {
            panel_name: Self::NAME.to_owned(),
            children: Vec::new(),
            info: PanelInfo::Panel(value),
        }
    }
}

fn render_inspector_tab_strip(
    active_tab: InspectorTab,
    cx: &Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    h_flex()
        .h(px(31.0))
        .w_full()
        .flex_none()
        .items_center()
        .gap_1()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border)
        .child(inspector_tab_title(
            "inspector-tab-details",
            "tune",
            "Details",
            active_tab == InspectorTab::Details,
            InspectorTab::Details,
            cx,
        ))
        .child(inspector_tab_title(
            "inspector-tab-prefab",
            "widgets",
            "Prefab",
            active_tab == InspectorTab::Prefab,
            InspectorTab::Prefab,
            cx,
        ))
        .into_any_element()
}

fn inspector_tab_title(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    active: bool,
    tab: InspectorTab,
    cx: &Context<'_, AuthoredInspector>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    div()
        .h_full()
        .min_w(px(72.0))
        .when(active, |this| this.border_b_2().border_color(theme.accent))
        .child(
            Button::new(id)
                .ghost()
                .small()
                .h_full()
                .w_full()
                .icon(kit::material_symbol_icon(
                    icon,
                    15.0,
                    if active {
                        theme.tab_active_foreground
                    } else {
                        theme.tab_foreground
                    },
                ))
                .label(label)
                .text_color(if active {
                    theme.tab_active_foreground
                } else {
                    theme.tab_foreground
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active_tab = tab;
                    cx.notify();
                    cx.emit(PanelEvent::LayoutChanged);
                })),
        )
        .into_any_element()
}

impl gpui::EventEmitter<PanelEvent> for AuthoredInspector {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflected_control_key_uses_named_path_segments() {
        let scope = ReflectedControlScope {
            source_path: "levels/main.prefab.ron".to_owned(),
            entity_alias: "root".to_owned(),
        };
        let target = ReflectedEditTarget::Direct(ReflectedEditBinding::new(PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "root".to_owned(),
            path: ReflectedPath {
                component_type_path: "azoth.transform.Transform".to_owned(),
                segments: vec![
                    ReflectedPathSegment::Field("translation".to_owned()),
                    ReflectedPathSegment::TupleIndex(2),
                ],
            },
        }));

        let key = scope.state_key(&target, ReflectedControlKind::TextInput);

        assert!(key.contains("azoth.transform.Transform"));
        assert!(key.contains("translation"));
        assert!(key.contains("t2"));
    }

    #[test]
    fn reflected_string_edits_emit_typed_ron() {
        let envelope = reflected_string_envelope("alloc::string::String", "hello");

        assert_eq!(envelope.encoding, ReflectedValueEncoding::TypedRon);
        // TypedRon payloads are valid RON documents: a String renders as a
        // quoted Rust string literal.
        assert_eq!(envelope.payload, b"\"hello\"");
    }
}
