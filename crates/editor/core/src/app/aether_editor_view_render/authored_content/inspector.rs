//! Reflected component inspection and component-authoring controls.

use std::collections::BTreeSet;
use std::path::Path;

use az_editor_inspector::{
    ReflectedComponentInspection, ReflectedEditBinding, ReflectedEntityInspection,
    ReflectedInspectionChild, ReflectedInspectionField, ReflectedOverrideOperation,
    ReflectedScalar, ReflectedValue, ReflectedValueNode, WidgetFamily,
};
use az_editor_ui::panels::{
    EditorAddableAuthoredComponents, EditorReflectedSelectionState, EditorTypeRegistry,
};
use az_proto_project::vnext::{
    PrefabEditCommand, ReflectedPathSegment, ReflectedValueEncoding, ReflectedValueEnvelope,
    TypeRegistrySnapshot,
};
use gpui::{AppContext, Context, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherItems};
use crate::app::aether_editor_model::{AetherEditorState, trace_aether_ui_state, trace_value};
use crate::app::aether_editor_view::AetherEditorView;

use super::super::presentation::{
    item_can_expand, non_empty_string_or, path_stem_label, set_item_style,
    settings_select_option_style, toggle_knob_style, toggle_track_style,
};
use super::schema_presentation::{schema_color, schema_icon};

impl AetherEditorView {
    pub(crate) fn components(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return self.components_from_inspection(inspection);
        }
        Vec::new()
    }

    pub(crate) fn add_component_schema_items(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let existing_components = selected_prefab_component_schemas(cx);
        let Some(addable) = cx.try_global::<EditorAddableAuthoredComponents>() else {
            return Vec::new();
        };
        let Some(registry) = cx.try_global::<EditorTypeRegistry>() else {
            return Vec::new();
        };
        let Some(entity_alias) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
            .map(|inspection| inspection.selection.entity_alias.clone())
        else {
            return Vec::new();
        };
        let search = self
            .state
            .add_component_search()
            .trim()
            .to_ascii_lowercase();
        let theme = cx.theme().clone();
        let mut schemas = addable
            .schemas
            .iter()
            .filter(|schema| {
                search.is_empty()
                    || schema.schema_type.to_ascii_lowercase().contains(&search)
                    || schema
                        .category
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&search)
            })
            .collect::<Vec<_>>();
        schemas.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then_with(|| left.schema_type.cmp(&right.schema_type))
        });

        schemas
            .into_iter()
            .map(|schema| {
                let category = schema
                    .category
                    .as_deref()
                    .filter(|category| !category.trim().is_empty())
                    .unwrap_or("Components");
                let disabled_reason = component_add_disabled_reason(
                    &registry.snapshot,
                    &existing_components,
                    &schema.schema_type,
                );
                let mut item = AetherItem {
                    kind: "add-component-schema".to_owned(),
                    key: schema.schema_type.clone(),
                    label: schema.label.clone(),
                    sub_label: disabled_reason
                        .as_ref()
                        .map_or_else(|| category.to_owned(), |reason| reason.clone()),
                    icon: schema
                        .icon
                        .clone()
                        .unwrap_or_else(|| schema_icon(&schema.schema_type).to_owned()),
                    icon_color: schema_color(&schema.schema_type).to_owned(),
                    disabled: disabled_reason.is_some(),
                    edit_command: Some(PrefabEditCommand::AddComponent {
                        entity_alias: entity_alias.clone(),
                        component_type_path: schema.schema_type.clone(),
                        initial_value: None,
                    }),
                    ..AetherItem::default()
                };
                set_item_style(
                    &mut item,
                    "style",
                    settings_select_option_style(disabled_reason.is_some(), &theme),
                );
                item
            })
            .collect()
    }
    pub(crate) fn open_add_component_popover(&mut self, cx: &mut Context<Self>) {
        crate::perf::begin_interaction(crate::perf::POPOVER_TO_VISIBLE);
        self.state.open_add_component_select_state();
        cx.notify();
    }

    pub(crate) fn update_add_component_search(
        &mut self,
        value: impl AsRef<str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.as_ref();
        if self.state.set_add_component_search_state(value) {
            cx.notify();
        }
    }

    pub(crate) fn add_component_search(&self) -> String {
        self.state.add_component_search().to_owned()
    }

    pub(crate) fn select_popover_is_add_component(&self) -> bool {
        self.state.add_component_select_open()
    }

    pub(crate) fn toggle_item_expanded(&mut self, item: &AetherItem, cx: &mut Context<Self>) {
        let before = self.state.trace_summary();
        let changed = self.toggle_expandable_item_state(item, true);
        trace_aether_ui_state(
            if changed {
                "item.toggle_expandable"
            } else {
                "item.toggle_expandable_ignored"
            },
            format!(
                "key={} name={} open={} before={}",
                trace_value(&item.key),
                trace_value(&item.name),
                !item.open,
                before
            ),
            &self.state,
        );
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn toggle_expandable_item_state(
        &mut self,
        item: &AetherItem,
        allow_empty_caret: bool,
    ) -> bool {
        if !item_can_expand(item, allow_empty_caret) {
            return false;
        }
        let default_open =
            item.open || item.caret == "arrow_drop_down" || item.caret == "expand_more";
        let next_open = !self.state.item_expanded(&item.key, default_open);
        self.state.set_item_expanded(&item.key, next_open)
    }

    pub(crate) fn components_from_inspection(
        &self,
        inspection: &ReflectedEntityInspection,
    ) -> Vec<AetherItem> {
        inspection
            .components
            .iter()
            .map(|component| component_section_item(&self.state, component))
            .collect()
    }
}

fn selected_prefab_component_schemas(cx: &mut Context<'_, AetherEditorView>) -> Vec<String> {
    let Some(inspection) = cx
        .try_global::<EditorReflectedSelectionState>()
        .and_then(EditorReflectedSelectionState::current)
    else {
        return Vec::new();
    };
    inspection
        .components
        .iter()
        .map(|component| component.component.type_path.clone())
        .collect()
}

fn component_add_disabled_reason(
    registry: &TypeRegistrySnapshot,
    existing: &[String],
    candidate: &str,
) -> Option<String> {
    if existing.iter().any(|type_path| type_path == candidate) {
        return Some("component is already present".to_owned());
    }
    let candidate = registry
        .types
        .iter()
        .find(|descriptor| descriptor.type_path == candidate)?;
    let existing_descriptors = existing
        .iter()
        .filter_map(|type_path| {
            registry
                .types
                .iter()
                .find(|descriptor| &descriptor.type_path == type_path)
        })
        .collect::<Vec<_>>();
    let provided = existing_descriptors
        .iter()
        .flat_map(|descriptor| &descriptor.applicability.provides)
        .collect::<BTreeSet<_>>();
    if let Some(required) = candidate
        .applicability
        .requires
        .iter()
        .find(|required| !provided.contains(required))
    {
        return Some(format!("requires capability `{required}`"));
    }
    if let Some(incompatible) = candidate
        .applicability
        .incompatible
        .iter()
        .find(|capability| provided.contains(capability))
    {
        return Some(format!("incompatible with capability `{incompatible}`"));
    }
    None
}

pub(super) fn prefab_override_item(
    index: usize,
    operation: &ReflectedOverrideOperation,
) -> AetherItem {
    let (target, action_label) = match operation {
        ReflectedOverrideOperation::Set { target, .. } => (target, "Set"),
        ReflectedOverrideOperation::Clear { target } => (target, "Clear"),
        ReflectedOverrideOperation::Insert { target, .. } => (target, "Insert"),
        ReflectedOverrideOperation::Remove { target, .. } => (target, "Remove"),
        ReflectedOverrideOperation::Move { target, .. } => (target, "Move"),
    };
    let path = target
        .path
        .segments
        .iter()
        .map(reflected_path_segment_label)
        .collect::<Vec<_>>()
        .join(".");
    AetherItem {
        key: format!("override:{index}"),
        comp: target.entity_alias.clone(),
        field: non_empty_string_or(path, &target.path.component_type_path),
        icon: "edit".to_owned(),
        icon_color: "#b78fd6".to_owned(),
        from: "source".to_owned(),
        to: action_label.to_owned(),
        edit_command: Some(operation.edit_command()),
        ..AetherItem::default()
    }
}

fn reflected_path_segment_label(segment: &ReflectedPathSegment) -> String {
    match segment {
        ReflectedPathSegment::Field(name) | ReflectedPathSegment::Variant(name) => name.clone(),
        ReflectedPathSegment::TupleIndex(index) | ReflectedPathSegment::ListIndex(index) => {
            index.to_string()
        }
    }
}

pub(crate) fn component_section_item(
    state: &AetherEditorState,
    component: &ReflectedComponentInspection,
) -> AetherItem {
    let enabled_field = component_enabled_field(&component.model.fields);
    let default_open = true;
    let open = state.item_expanded(&component.component.type_path, default_open);
    let mut item = AetherItem {
        kind: "authored-component".to_owned(),
        key: component.component.type_path.clone(),
        id: component.component.type_path.clone(),
        name: component.model.type_label.clone(),
        icon: component
            .model
            .icon
            .clone()
            .unwrap_or_else(|| schema_icon(&component.component.type_path).to_owned()),
        color: schema_color(&component.component.type_path).to_owned(),
        open,
        caret: if open {
            "expand_more".to_owned()
        } else {
            "chevron_right".to_owned()
        },
        has_val: enabled_field.is_some(),
        props: AetherItems(
            component
                .model
                .fields
                .iter()
                .filter(|field| !field.hidden)
                .filter(|field| enabled_field.is_none_or(|enabled| enabled.name != field.name))
                .map(authored_field_property_item)
                .collect(),
        ),
        items: AetherItems(component_menu_items(component)),
        ..AetherItem::default()
    };

    if let Some(field) = enabled_field {
        let on = reflected_field_bool(field).unwrap_or(true);
        item.edit_binding = Some(field.value.binding.clone());
        item.edit_value = Some(reflected_envelope(
            &field.value.type_path,
            (!on).to_string(),
        ));
        set_item_style(&mut item, "trackStyle", toggle_track_style(on));
        set_item_style(&mut item, "knobStyle", toggle_knob_style(on));
    }

    item
}

fn component_enabled_field(
    fields: &[ReflectedInspectionField],
) -> Option<&ReflectedInspectionField> {
    fields.iter().find(|field| {
        !field.hidden
            && field.name.eq_ignore_ascii_case("enabled")
            && reflected_field_bool(field).is_some()
    })
}

fn component_menu_items(component: &ReflectedComponentInspection) -> Vec<AetherItem> {
    vec![AetherItem {
        kind: "authored-remove-path".to_owned(),
        key: format!("remove:{}", component.component.type_path),
        label: "Remove Component".to_owned(),
        icon: "delete".to_owned(),
        comp: component.model.type_label.clone(),
        edit_command: Some(PrefabEditCommand::RemoveComponent {
            entity_alias: component.component.entity_alias.clone(),
            component_type_path: component.component.type_path.clone(),
        }),
        ..AetherItem::default()
    }]
}

fn authored_field_property_item(field: &ReflectedInspectionField) -> AetherItem {
    let display_value = reflected_node_label(&field.value);
    let mut item = AetherItem {
        key: field.name.clone(),
        label: non_empty_string_or(&field.label, &field.name),
        name: display_value.clone(),
        val: display_value.clone(),
        icon: authored_field_icon(field).to_owned(),
        icon_color: authored_field_color(field).to_owned(),
        edit_binding: Some(field.value.binding.clone()),
        edit_type_path: field.value.type_path.clone(),
        edit_text_quoted: matches!(
            field.widget.family,
            WidgetFamily::Text | WidgetFamily::Asset { .. } | WidgetFamily::Object { .. }
        ),
        ..AetherItem::default()
    };

    if matches!(field.widget.family, WidgetFamily::Vector { dimensions: 3 }) {
        item.is_vec3 = true;
        let components = reflected_vector_components(&field.value);
        item.x = components
            .first()
            .map_or_else(|| "0".to_owned(), |(_, value)| value.clone());
        item.y = components
            .get(1)
            .map_or_else(|| "0".to_owned(), |(_, value)| value.clone());
        item.z = components
            .get(2)
            .map_or_else(|| "0".to_owned(), |(_, value)| value.clone());
        item.x_binding = components.first().map(|(binding, _)| binding.clone());
        item.y_binding = components.get(1).map(|(binding, _)| binding.clone());
        item.z_binding = components.get(2).map(|(binding, _)| binding.clone());
        return item;
    }

    if matches!(field.widget.family, WidgetFamily::Color) {
        item.is_color = true;
        item.color = display_value.clone();
        return item;
    }

    if matches!(field.widget.family, WidgetFamily::Enum) {
        item.is_enum = true;
        item.name = display_value;
        item.items = AetherItems(
            field
                .widget
                .variants
                .iter()
                .map(|variant| AetherItem {
                    kind: "authored-enum-option".to_owned(),
                    key: format!("{}:{variant}", field.name),
                    label: variant.clone(),
                    selected: item.name == *variant,
                    edit_command: Some(PrefabEditCommand::SetVariant {
                        target: field.value.binding.target.clone(),
                        variant_name: variant.clone(),
                        value: None,
                    }),
                    ..AetherItem::default()
                })
                .collect(),
        );
        return item;
    }

    match &field.widget.family {
        WidgetFamily::Bool => {
            let value = reflected_field_bool(field).unwrap_or(false);
            item.is_bool = true;
            item.val = value.to_string();
            item.edit_value = Some(reflected_envelope(
                &field.value.type_path,
                (!value).to_string(),
            ));
            set_item_style(&mut item, "trackStyle", toggle_track_style(value));
            set_item_style(&mut item, "knobStyle", toggle_knob_style(value));
        }
        WidgetFamily::Number | WidgetFamily::Slider => {
            item.is_num = true;
        }
        WidgetFamily::Asset { .. } => {
            item.is_asset = true;
            item.name = path_stem_label(&item.val);
            item.icon = "deployed_code".to_owned();
            item.icon_color = "#7a8aa0".to_owned();
        }
        _ => {
            item.is_str = true;
        }
    }

    item
}

fn reflected_envelope(type_path: &str, payload: impl Into<String>) -> ReflectedValueEnvelope {
    ReflectedValueEnvelope {
        type_path: type_path.to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload: payload.into().into_bytes(),
    }
}

fn reflected_field_bool(field: &ReflectedInspectionField) -> Option<bool> {
    match field.value.current.effective.as_ref()? {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => Some(*value),
        _ => None,
    }
}

pub(super) fn reflected_node_label(node: &ReflectedValueNode) -> String {
    node.current
        .effective
        .as_ref()
        .map(reflected_value_label)
        .unwrap_or_else(|| "unset".to_owned())
}

fn reflected_value_label(value: &ReflectedValue) -> String {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => value.to_string(),
        ReflectedValue::Scalar(ReflectedScalar::Signed(value))
        | ReflectedValue::Scalar(ReflectedScalar::Unsigned(value))
        | ReflectedValue::Scalar(ReflectedScalar::Float(value))
        | ReflectedValue::Scalar(ReflectedScalar::String(value))
        | ReflectedValue::OpaqueRon(value) => value.clone(),
        ReflectedValue::Enum { variant, .. } => variant.clone(),
        ReflectedValue::Optional(None) => "None".to_owned(),
        ReflectedValue::Optional(Some(value)) => reflected_value_label(value),
        ReflectedValue::Struct(fields) => format!("{} fields", fields.len()),
        ReflectedValue::Tuple(values) | ReflectedValue::List(values) => values
            .iter()
            .map(reflected_value_label)
            .collect::<Vec<_>>()
            .join(", "),
        ReflectedValue::Map(values) => format!("{} entries", values.len()),
        ReflectedValue::Unit => "()".to_owned(),
        ReflectedValue::Encoded(value) => String::from_utf8_lossy(&value.payload).into_owned(),
    }
}

fn reflected_vector_components(node: &ReflectedValueNode) -> Vec<(ReflectedEditBinding, String)> {
    node.children
        .iter()
        .filter_map(|child| match child {
            ReflectedInspectionChild::Field(field) => Some((
                field.value.binding.clone(),
                reflected_node_label(&field.value),
            )),
            ReflectedInspectionChild::TupleElement { value, .. } => {
                Some((value.binding.clone(), reflected_node_label(value)))
            }
            _ => None,
        })
        .take(3)
        .collect()
}

fn authored_field_icon(field: &ReflectedInspectionField) -> &'static str {
    match &field.widget.family {
        WidgetFamily::Bool => "toggle_on",
        WidgetFamily::Number | WidgetFamily::Slider | WidgetFamily::Vector { .. } => "tag",
        WidgetFamily::Asset { .. } => "deployed_code",
        WidgetFamily::Object { .. } => "ads_click",
        WidgetFamily::Text | WidgetFamily::Multiline => "short_text",
        WidgetFamily::Enum => "list",
        WidgetFamily::Color => "palette",
        _ => "data_object",
    }
}

fn authored_field_color(field: &ReflectedInspectionField) -> &'static str {
    match &field.widget.family {
        WidgetFamily::Bool => "#57b97e",
        WidgetFamily::Number | WidgetFamily::Slider | WidgetFamily::Vector { .. } => "#d6a23b",
        WidgetFamily::Asset { .. } => "#7a8aa0",
        WidgetFamily::Object { .. } => "#4188e0",
        WidgetFamily::Text | WidgetFamily::Multiline => "#7a8aa0",
        _ => "#b78fd6",
    }
}
