//! Editor-owned Materials mode controller.
//!
//! Materials mode is a projection over two existing controller boundaries:
//! material node-graph documents (`.azmat.ron`) live in the visual-graph
//! controller ([`crate::graph_ui`]) and material property documents
//! (`.azmaterial.ron` / `.azmaterialtype.ron`) live in the authored-selection
//! controller ([`crate::authored_selection`]). This module joins their
//! published globals with materials-local UI state and publishes the read-only
//! [`az_editor_ui::panels::EditorMaterialsProjection`] global that the
//! Materials mode panels render. Selection/preview state lives here and is
//! mutated by the typed `Material*` actions dispatched from the panels.

use az_editor_inspector::{
    ReflectedComponentInspection, ReflectedEntityInspection, ReflectedInspectionChild,
    ReflectedInspectionField, ReflectedScalar, ReflectedValue, ReflectedValueNode, WidgetFamily,
    WidgetSpec,
};
use az_editor_ui::panels::{
    EditorAuthoredOutline, EditorGraphDocumentProjection, EditorMaterialsProjection,
    EditorReflectedSelectionState, MaterialCanvasProjection, MaterialDocumentKind,
    MaterialDocumentTabProjection, MaterialGraphTargetProjection,
    MaterialPaletteCategoryProjection, MaterialPaletteItemProjection, MaterialPaletteProjection,
    MaterialParamControlProjection, MaterialParamRowProjection, MaterialParamsProjection,
    MaterialPreviewMaterialProjection, MaterialPreviewProjection, MaterialPreviewShape,
};
use gpui::{App, Global};
use std::collections::BTreeMap;
use tracing::{error, info};

use crate::mode_projection::{
    ModeProjectionInputs, ModeProjectionRegistration, ModeProjectionRegistrationError,
    ModeProjectionSpec, publish_mode_projection_and_refresh,
};
use crate::source_navigation::{MaterialSourceKind, material_source_kind};

/// Material graph type id served by az-material-nodes.
const MATERIAL_GRAPH_TYPE: &str = az_material::MATERIAL_GRAPH_ASSET_TYPE_HINT;
/// Node-palette tag every material node carries (az-material-nodes).
const MATERIAL_NODE_TAG: &str = "material";
/// Root palette category of the material node library.
const MATERIAL_NODE_ROOT_CATEGORY: &str = "Material";

const MAX_PARAM_ROWS: usize = 64;
const MAX_PARAM_DEPTH: usize = 6;

/// Neutral preview defaults used when the authored document does not bind a
/// preview input (kept honest via `bound`/`defaulted` in the projection).
const DEFAULT_PREVIEW_BASE_COLOR: [f32; 4] = [0.62, 0.64, 0.68, 1.0];
const DEFAULT_PREVIEW_ROUGHNESS: f32 = 0.5;
const DEFAULT_PREVIEW_METALLIC: f32 = 0.0;

/// Editor-core-owned Materials mode UI state. Inputs to the projection build;
/// never rendered directly.
#[derive(Debug, Clone, Default)]
struct MaterialsUiState {
    selected_document_id: String,
    preview_shape: MaterialPreviewShape,
    palette_filter: String,
}

impl Global for MaterialsUiState {}

struct MaterialsMode;

impl ModeProjectionSpec for MaterialsMode {
    type State = MaterialsUiState;
    type Projection = EditorMaterialsProjection;

    const NAME: &'static str = "materials";

    fn register_inputs(inputs: &mut ModeProjectionInputs) {
        inputs.depends_on::<EditorGraphDocumentProjection>();
        inputs.depends_on::<EditorAuthoredOutline>();
        inputs.depends_on::<EditorReflectedSelectionState>();
    }

    fn install_actions(cx: &mut App) {
        install_materials_actions(cx);
    }

    fn project(state: &Self::State, cx: &App) -> Self::Projection {
        let graph = cx.try_global::<EditorGraphDocumentProjection>();
        let outline = cx
            .try_global::<EditorAuthoredOutline>()
            .map(|outline| &outline.data);
        let inspection = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current);
        build_materials_projection(graph, outline, inspection, state)
    }
}

pub fn mode_projection_registration()
-> Result<ModeProjectionRegistration, ModeProjectionRegistrationError> {
    ModeProjectionRegistration::for_spec::<MaterialsMode>()
}

fn install_materials_actions(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::SelectMaterialDocument, cx| {
            select_material_document(cx, action.document_id.clone());
        },
    );

    cx.on_action(
        |action: &az_editor_ui::actions::SetMaterialPreviewShape, cx| {
            cx.default_global::<MaterialsUiState>().preview_shape = action.shape;
            publish_mode_projection_and_refresh::<MaterialsMode>(cx);
        },
    );

    cx.on_action(
        |action: &az_editor_ui::actions::SetMaterialPaletteFilter, cx| {
            cx.default_global::<MaterialsUiState>()
                .palette_filter
                .clone_from(&action.filter);
            publish_mode_projection_and_refresh::<MaterialsMode>(cx);
        },
    );

    info!("installed materials action handlers");
}

/// Select a material document: graph sources route into the visual-graph
/// controller, property documents route into the authored-selection
/// controller.
pub fn select_material_document(cx: &mut App, document_id: String) {
    match material_document_route(cx, &document_id) {
        MaterialDocumentRoute::Graph(routed) => open_material_graph_document(cx, &routed),
        MaterialDocumentRoute::Authored(routed) => {
            if let Err(err) =
                crate::authored_selection::select_reflected_entity(cx, routed.clone(), "root")
            {
                error!(
                    error = %err,
                    document = %routed,
                    "failed to load material property document"
                );
            }
        }
    }
    cx.default_global::<MaterialsUiState>().selected_document_id = document_id;
    publish_mode_projection_and_refresh::<MaterialsMode>(cx);
}

#[derive(Debug, PartialEq, Eq)]
enum MaterialDocumentRoute {
    Graph(String),
    Authored(String),
}

fn material_document_route(cx: &App, document_id: &str) -> MaterialDocumentRoute {
    match material_source_kind(document_id) {
        Some(MaterialSourceKind::Graph) => {
            MaterialDocumentRoute::Graph(resolve_graph_document_id(cx, document_id))
        }
        Some(MaterialSourceKind::Material | MaterialSourceKind::MaterialType) | None => {
            MaterialDocumentRoute::Authored(document_id.to_owned())
        }
    }
}

/// Track a material selection whose authored-document load is already in
/// flight (asset-browser `SelectAuthoredDocument` routing): records the
/// selection and additionally opens `.azmat.ron` sources as graph documents.
pub fn sync_material_selection_from_authored(cx: &mut App, document_id: String) {
    if material_source_kind(&document_id) == Some(MaterialSourceKind::Graph) {
        open_material_graph_document(cx, &document_id);
    }
    cx.default_global::<MaterialsUiState>().selected_document_id = document_id;
    publish_mode_projection_and_refresh::<MaterialsMode>(cx);
}

fn open_material_graph_document(cx: &mut App, document_id: &str) {
    if let Err(err) = crate::graph_ui::open_graph_document(cx, document_id.to_owned()) {
        error!(
            error = %err,
            document = %document_id,
            "failed to open material graph document"
        );
    }
}

/// Map an asset-browser source path onto the graph controller's document id
/// when they differ (falls back to the given id).
fn resolve_graph_document_id(cx: &App, document_id: &str) -> String {
    cx.try_global::<EditorGraphDocumentProjection>()
        .and_then(|graph| {
            graph
                .graph_documents
                .documents
                .iter()
                .find(|entry| entry.document_id == document_id || entry.source_path == document_id)
                .map(|entry| entry.document_id.clone())
        })
        .unwrap_or_else(|| document_id.to_owned())
}

// ---------------------------------------------------------------------------
// Projection build (pure)
// ---------------------------------------------------------------------------

fn build_materials_projection(
    graph: Option<&EditorGraphDocumentProjection>,
    outline: Option<&az_editor_ui::panels::AuthoredOutlineData>,
    inspection: Option<&ReflectedEntityInspection>,
    state: &MaterialsUiState,
) -> EditorMaterialsProjection {
    let material_inspection = inspection.and_then(material_component);
    let selected = effective_selected_document(
        state,
        graph,
        inspection.filter(|_| material_inspection.is_some()),
    );
    let documents = material_document_tabs(graph, outline, selected.as_deref());
    let palette = material_palette(graph, &state.palette_filter);
    let canvas = material_canvas(graph);
    let params = material_params(
        inspection.filter(|_| material_inspection.is_some()),
        material_inspection,
    );
    let preview = MaterialPreviewProjection {
        shape: state.preview_shape,
        material: params.as_ref().map(material_preview_from_params),
    };

    EditorMaterialsProjection {
        graph_ready: graph.is_some(),
        documents,
        palette,
        canvas,
        preview,
        params,
    }
}

fn effective_selected_document(
    state: &MaterialsUiState,
    graph: Option<&EditorGraphDocumentProjection>,
    material_inspection: Option<&ReflectedEntityInspection>,
) -> Option<String> {
    if !state.selected_document_id.trim().is_empty() {
        return Some(state.selected_document_id.clone());
    }
    if let Some(inspection) = material_inspection {
        return Some(inspection.selection.source_path.clone());
    }
    graph
        .and_then(|graph| graph.document.as_ref())
        .filter(|document| document.graph_type == MATERIAL_GRAPH_TYPE)
        .map(|document| document.document_id.clone())
}

fn material_schema_label(schema_type: &str) -> Option<&'static str> {
    if schema_type == az_material::MATERIAL_SCHEMA_NAME
        || schema_type.ends_with("::MaterialSource")
        || schema_type.ends_with(".MaterialSource")
    {
        Some("Material")
    } else if schema_type == az_material::MATERIAL_TYPE_SCHEMA_NAME
        || schema_type.ends_with("::MaterialTypeSource")
        || schema_type.ends_with(".MaterialTypeSource")
    {
        Some("Material Type")
    } else {
        None
    }
}

fn material_component(
    inspection: &ReflectedEntityInspection,
) -> Option<&ReflectedComponentInspection> {
    inspection
        .components
        .iter()
        .find(|component| material_schema_label(&component.model.type_path).is_some())
}

// --- tabs ---

fn material_document_tabs(
    graph: Option<&EditorGraphDocumentProjection>,
    outline: Option<&az_editor_ui::panels::AuthoredOutlineData>,
    selected: Option<&str>,
) -> Vec<MaterialDocumentTabProjection> {
    let selected_matches = |document_id: &str, source_path: &str| {
        selected.is_some_and(|selected| selected == document_id || selected == source_path)
    };

    let mut tabs = Vec::new();
    if let Some(graph) = graph {
        for entry in &graph.graph_documents.documents {
            if entry.graph_type != MATERIAL_GRAPH_TYPE {
                continue;
            }
            tabs.push(MaterialDocumentTabProjection {
                document_id: entry.document_id.clone(),
                label: source_stem(non_empty_str(&entry.source_path, &entry.document_id)),
                kind: MaterialDocumentKind::Graph,
                selected: selected_matches(&entry.document_id, &entry.source_path),
                unsaved_changes: entry.unsaved_changes,
            });
        }
    }
    if let Some(outline) = outline {
        for document in &outline.documents {
            let kind = if document.schema_type == az_material::MATERIAL_SCHEMA_NAME {
                MaterialDocumentKind::Material
            } else if document.schema_type == az_material::MATERIAL_TYPE_SCHEMA_NAME {
                MaterialDocumentKind::MaterialType
            } else {
                continue;
            };
            tabs.push(MaterialDocumentTabProjection {
                document_id: document.document_id.clone(),
                label: source_stem(non_empty_str(&document.source_path, &document.document_id)),
                kind,
                selected: selected_matches(&document.document_id, &document.source_path),
                unsaved_changes: document.unsaved_changes,
            });
        }
    }
    tabs
}

// --- palette ---

fn material_palette(
    graph: Option<&EditorGraphDocumentProjection>,
    filter: &str,
) -> MaterialPaletteProjection {
    let Some(graph) = graph else {
        return MaterialPaletteProjection {
            filter: filter.to_owned(),
            ..MaterialPaletteProjection::default()
        };
    };

    let material_nodes = graph
        .node_palette
        .nodes
        .iter()
        .filter(|node| is_material_palette_node(node))
        .collect::<Vec<_>>();
    let total_count = material_nodes.len();

    let filter_lower = filter.trim().to_ascii_lowercase();
    let mut by_category = BTreeMap::<String, Vec<MaterialPaletteItemProjection>>::new();
    for node in material_nodes {
        if !filter_lower.is_empty()
            && !node.label.to_ascii_lowercase().contains(&filter_lower)
            && !node.category.to_ascii_lowercase().contains(&filter_lower)
        {
            continue;
        }
        let category = material_subcategory(&node.category);
        by_category
            .entry(category.clone())
            .or_default()
            .push(MaterialPaletteItemProjection {
                node_type: node.node_type.clone(),
                version: node.version,
                label: node.label.clone(),
                description: node.description.clone(),
                port_summary: format!("{} in · {} out", node.input_count, node.output_count),
                output: category == "Output",
            });
    }

    let mut categories = by_category
        .into_iter()
        .map(|(name, mut items)| {
            items.sort_by(|left, right| left.label.cmp(&right.label));
            MaterialPaletteCategoryProjection { name, items }
        })
        .collect::<Vec<_>>();
    // Keep the terminal Output category last (design order: inputs first,
    // output sink at the bottom).
    categories.sort_by(|left, right| {
        (left.name == "Output", &left.name).cmp(&(right.name == "Output", &right.name))
    });

    MaterialPaletteProjection {
        categories,
        total_count,
        filter: filter.to_owned(),
        target: material_graph_target(graph),
    }
}

fn is_material_palette_node(node: &az_editor_ui::panels::GraphNodePaletteItemData) -> bool {
    node.tags.iter().any(|tag| tag == MATERIAL_NODE_TAG)
        || node.category == MATERIAL_NODE_ROOT_CATEGORY
        || node
            .category
            .starts_with(&format!("{MATERIAL_NODE_ROOT_CATEGORY}/"))
}

fn material_subcategory(category: &str) -> String {
    category
        .strip_prefix(&format!("{MATERIAL_NODE_ROOT_CATEGORY}/"))
        .map_or_else(
            || {
                if category == MATERIAL_NODE_ROOT_CATEGORY || category.is_empty() {
                    "General".to_owned()
                } else {
                    category.to_owned()
                }
            },
            ToOwned::to_owned,
        )
}

/// `AddGraphNode` targets the graph controller's current document; expose the
/// target only when that document is a material graph. Placement mirrors the
/// graph panel's `next_node_position` grid.
fn material_graph_target(
    graph: &EditorGraphDocumentProjection,
) -> Option<MaterialGraphTargetProjection> {
    let document = graph
        .document
        .as_ref()
        .filter(|document| document.graph_type == MATERIAL_GRAPH_TYPE)?;
    // invariant: node counts in an authored graph are small; f32 precision
    // loss is irrelevant for layout placement
    #[allow(clippy::cast_precision_loss)]
    let index = document.nodes.len() as f32;
    Some(MaterialGraphTargetProjection {
        document_id: document.document_id.clone(),
        next_x: (index % 4.0).mul_add(240.0, 80.0),
        next_y: (index / 4.0).floor().mul_add(160.0, 80.0),
    })
}

// --- canvas ---

fn material_canvas(graph: Option<&EditorGraphDocumentProjection>) -> MaterialCanvasProjection {
    let Some(graph) = graph else {
        return MaterialCanvasProjection {
            material_graph_document_id: None,
            unsaved_changes: false,
            empty_reason: "Graph controller not connected yet.".to_owned(),
        };
    };
    match graph.document.as_ref() {
        Some(document) if document.graph_type == MATERIAL_GRAPH_TYPE => MaterialCanvasProjection {
            material_graph_document_id: Some(document.document_id.clone()),
            unsaved_changes: document.unsaved_changes,
            empty_reason: String::new(),
        },
        Some(document) => MaterialCanvasProjection {
            material_graph_document_id: None,
            unsaved_changes: false,
            empty_reason: format!(
                "Current graph document `{}` is a {} graph, not a material graph. Open a \
                 .azmat.ron source to edit material shading.",
                document.document_id, document.graph_type
            ),
        },
        None => MaterialCanvasProjection {
            material_graph_document_id: None,
            unsaved_changes: false,
            empty_reason: "No material graph open. Activate a .azmat.ron source in the Asset \
                           Browser to edit the material node graph."
                .to_owned(),
        },
    }
}

// --- parameters ---

fn material_params(
    inspection: Option<&ReflectedEntityInspection>,
    component: Option<&ReflectedComponentInspection>,
) -> Option<MaterialParamsProjection> {
    let inspection = inspection?;
    let component = component?;
    let schema_label = material_schema_label(&component.model.type_path)?;
    Some(MaterialParamsProjection {
        document_id: inspection.selection.source_path.clone(),
        label: source_stem(non_empty_str(
            &inspection.selection.source_path,
            &component.model.type_label,
        )),
        schema_label: schema_label.to_owned(),
        rows: param_rows_from_fields(&component.model.fields),
    })
}

#[derive(Clone)]
struct ParamNode<'a> {
    name: String,
    label: String,
    value: &'a ReflectedValueNode,
    widget: Option<&'a WidgetSpec>,
    hidden: bool,
}

impl<'a> ParamNode<'a> {
    fn from_field(field: &'a ReflectedInspectionField) -> Self {
        Self {
            name: field.name.clone(),
            label: non_empty_str(&field.label, &field.name).to_owned(),
            value: &field.value,
            widget: Some(&field.widget),
            hidden: field.hidden,
        }
    }
}

fn param_rows_from_fields(fields: &[ReflectedInspectionField]) -> Vec<MaterialParamRowProjection> {
    let mut rows = Vec::new();
    for field in fields {
        let node = ParamNode::from_field(field);
        collect_param_rows(&node, node.label.clone(), &field.name, 0, &mut rows);
    }
    rows
}

fn collect_param_rows(
    node: &ParamNode<'_>,
    label: String,
    key: &str,
    depth: usize,
    rows: &mut Vec<MaterialParamRowProjection>,
) {
    if node.hidden || rows.len() >= MAX_PARAM_ROWS || depth > MAX_PARAM_DEPTH {
        return;
    }

    if let Some(control) = classify_param_control(node) {
        rows.push(MaterialParamRowProjection {
            key: key.to_owned(),
            label,
            control,
        });
        return;
    }

    // Material property binding: `{ property: "base_color", value: <enum> }`
    // renders as one row labeled by the authored property id.
    if let Some((property, child)) = property_binding_parts(node) {
        collect_param_rows(
            &child,
            property.clone(),
            &format!("{key}.{property}"),
            depth + 1,
            rows,
        );
        return;
    }

    // Material type property definition: label by its display name, classify
    // its default value.
    if let Some((display_name, child)) = property_definition_parts(node) {
        collect_param_rows(
            &child,
            display_name.clone(),
            &format!("{key}.{display_name}"),
            depth + 1,
            rows,
        );
        return;
    }

    let visible_children = param_children(node)
        .into_iter()
        .filter(|child| !child.hidden)
        .collect::<Vec<_>>();

    // Single-child wrappers (enum payloads, newtype structs) keep the parent
    // label instead of stacking breadcrumbs.
    if let [only] = visible_children.as_slice() {
        collect_param_rows(only, label, key, depth + 1, rows);
        return;
    }

    for child in visible_children {
        let child_label = if child.label.trim().is_empty() {
            label.clone()
        } else {
            format!("{label} · {}", child.label)
        };
        let child_key = format!("{key}.{}", non_empty_str(&child.name, "child"));
        collect_param_rows(&child, child_label, &child_key, depth + 1, rows);
    }
}

fn param_children<'a>(node: &ParamNode<'a>) -> Vec<ParamNode<'a>> {
    let mut children = Vec::new();
    for child in &node.value.children {
        push_param_child(child, &mut children);
    }
    children
}

fn push_param_child<'a>(child: &'a ReflectedInspectionChild, children: &mut Vec<ParamNode<'a>>) {
    match child {
        ReflectedInspectionChild::Field(field) => children.push(ParamNode::from_field(field)),
        ReflectedInspectionChild::TupleElement { index, value }
        | ReflectedInspectionChild::ListItem(az_editor_inspector::ReflectedListItem {
            index,
            value,
        }) => children.push(ParamNode {
            name: index.to_string(),
            label: index.to_string(),
            value,
            widget: None,
            hidden: false,
        }),
        ReflectedInspectionChild::MapEntry(entry) => children.push(ParamNode {
            name: "value".to_owned(),
            label: reflected_display(&entry.key).unwrap_or_else(|| "value".to_owned()),
            value: &entry.value,
            widget: None,
            hidden: false,
        }),
        ReflectedInspectionChild::Variant(variant) => {
            for field in &variant.fields {
                push_param_child(field, children);
            }
        }
        ReflectedInspectionChild::OptionalSome(value) => children.push(ParamNode {
            name: "value".to_owned(),
            label: String::new(),
            value,
            widget: None,
            hidden: false,
        }),
    }
}

const fn node_value(node: &ReflectedValueNode) -> Option<&ReflectedValue> {
    node.current.effective.as_ref()
}

fn classify_param_control(node: &ParamNode<'_>) -> Option<MaterialParamControlProjection> {
    let children = param_children(node);
    if node
        .widget
        .is_some_and(|widget| widget.family == WidgetFamily::Color)
        && let Some(rgba) = rgba_from_children(&children)
    {
        return Some(MaterialParamControlProjection::Color {
            rgba,
            display: rgba_hex_display(rgba),
        });
    }

    match node_value(node.value) {
        Some(ReflectedValue::Scalar(ReflectedScalar::Float(value))) => {
            let value = value.parse::<f64>().ok()?;
            let (fraction, display) = slider_projection(value, node.widget);
            Some(MaterialParamControlProjection::Slider { fraction, display })
        }
        Some(ReflectedValue::Scalar(ReflectedScalar::Bool(value))) => {
            Some(MaterialParamControlProjection::Toggle { value: *value })
        }
        Some(ReflectedValue::Scalar(
            ReflectedScalar::Signed(value) | ReflectedScalar::Unsigned(value),
        )) => Some(MaterialParamControlProjection::Value {
            display: value.clone(),
        }),
        Some(ReflectedValue::Scalar(ReflectedScalar::String(text)))
            if node
                .widget
                .is_some_and(|widget| matches!(widget.family, WidgetFamily::Asset { .. })) =>
        {
            Some(MaterialParamControlProjection::Texture { path: text.clone() })
        }
        Some(ReflectedValue::Scalar(ReflectedScalar::String(reference)))
            if node
                .widget
                .is_some_and(|widget| matches!(widget.family, WidgetFamily::Object { .. })) =>
        {
            Some(MaterialParamControlProjection::Value {
                display: reference.clone(),
            })
        }
        Some(ReflectedValue::Scalar(ReflectedScalar::String(text))) => {
            Some(parse_hex_color(text).map_or_else(
                || MaterialParamControlProjection::Value {
                    display: text.clone(),
                },
                |rgba| MaterialParamControlProjection::Color {
                    rgba,
                    display: text.clone(),
                },
            ))
        }
        Some(value @ (ReflectedValue::Enum { .. } | ReflectedValue::OpaqueRon(_))) => {
            reflected_display(value)
                .map(|display| MaterialParamControlProjection::Value { display })
        }
        _ => None,
    }
}

fn slider_projection(value: f64, widget: Option<&WidgetSpec>) -> (f32, String) {
    let fraction = widget
        .and_then(|widget| widget.range.as_ref())
        .and_then(|range| {
            let min = range.minimum.as_deref()?.parse::<f64>().ok()?;
            let max = range.maximum.as_deref()?.parse::<f64>().ok()?;
            (max > min).then_some((value - min) / (max - min))
        })
        .unwrap_or(value);
    // invariant: UI slider fractions and property scalars are small values;
    // f64->f32 narrowing is the intended precision
    #[allow(clippy::cast_possible_truncation)]
    {
        ((fraction as f32).clamp(0.0, 1.0), format!("{value:.3}"))
    }
}

fn property_binding_parts<'a>(node: &ParamNode<'a>) -> Option<(String, ParamNode<'a>)> {
    let children = param_children(node);
    let property = children.iter().find(|child| {
        child.name.eq_ignore_ascii_case("property") || child.label.eq_ignore_ascii_case("property")
    })?;
    let value = children
        .iter()
        .find(|child| child.name.eq_ignore_ascii_case("value"))?
        .clone();
    let ReflectedValue::Scalar(ReflectedScalar::String(property_name)) =
        node_value(property.value)?
    else {
        return None;
    };
    Some((non_empty_str(property_name, "property").to_owned(), value))
}

fn property_definition_parts<'a>(node: &ParamNode<'a>) -> Option<(String, ParamNode<'a>)> {
    let children = param_children(node);
    let display_name = children.iter().find(|child| {
        child.name.eq_ignore_ascii_case("display_name")
            || child.label.eq_ignore_ascii_case("display name")
    })?;
    let default_value = children
        .iter()
        .find(|child| child.name.eq_ignore_ascii_case("default_value"))?
        .clone();
    let ReflectedValue::Scalar(ReflectedScalar::String(name)) = node_value(display_name.value)?
    else {
        return None;
    };
    Some((non_empty_str(name, "property").to_owned(), default_value))
}

fn rgba_from_children(children: &[ParamNode<'_>]) -> Option<[f32; 4]> {
    let component = |label: &str| {
        children
            .iter()
            .find(|child| {
                child.name.eq_ignore_ascii_case(label) || child.label.eq_ignore_ascii_case(label)
            })
            .and_then(|child| match node_value(child.value)? {
                ReflectedValue::Scalar(ReflectedScalar::Float(value)) => value.parse().ok(),
                _ => None,
            })
    };
    Some([
        component("r")?,
        component("g")?,
        component("b")?,
        component("a").unwrap_or(1.0),
    ])
}

fn reflected_display(value: &ReflectedValue) -> Option<String> {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => Some(value.to_string()),
        ReflectedValue::Scalar(
            ReflectedScalar::Signed(value)
            | ReflectedScalar::Unsigned(value)
            | ReflectedScalar::Float(value)
            | ReflectedScalar::String(value),
        )
        | ReflectedValue::OpaqueRon(value) => Some(value.clone()),
        ReflectedValue::Enum { variant, .. } => Some(variant.clone()),
        ReflectedValue::Optional(Some(value)) => reflected_display(value),
        ReflectedValue::Optional(None) => Some("None".to_owned()),
        _ => None,
    }
}

fn rgba_hex_display(rgba: [f32; 4]) -> String {
    let channel = |value: f32| {
        // invariant: clamped 0..=1 channel scaled to u8
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        }
    };
    if (rgba[3] - 1.0).abs() < f32::EPSILON {
        format!(
            "#{:02X}{:02X}{:02X}",
            channel(rgba[0]),
            channel(rgba[1]),
            channel(rgba[2])
        )
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            channel(rgba[0]),
            channel(rgba[1]),
            channel(rgba[2]),
            channel(rgba[3])
        )
    }
}

fn parse_hex_color(text: &str) -> Option<[f32; 4]> {
    let text = text.trim().strip_prefix('#')?;
    let parse = |slice: &str| u8::from_str_radix(slice, 16).ok();
    let channel = |value: u8| f32::from(value) / 255.0;
    match text.len() {
        6 => Some([
            channel(parse(&text[0..2])?),
            channel(parse(&text[2..4])?),
            channel(parse(&text[4..6])?),
            1.0,
        ]),
        8 => Some([
            channel(parse(&text[0..2])?),
            channel(parse(&text[2..4])?),
            channel(parse(&text[4..6])?),
            channel(parse(&text[6..8])?),
        ]),
        _ => None,
    }
}

// --- preview ---

/// Rendering brief B3 v1: derive the preview parameterization from the
/// classified parameter rows — base color → tint, roughness → highlight,
/// metallic → contrast. Missing inputs fall back to neutral defaults and are
/// reported in `defaulted`.
fn material_preview_from_params(
    params: &MaterialParamsProjection,
) -> MaterialPreviewMaterialProjection {
    let mut bound = Vec::new();
    let mut defaulted = Vec::new();

    let row_matches = |row: &MaterialParamRowProjection, needles: &[&str]| {
        let label = row.label.to_ascii_lowercase();
        let key = row.key.to_ascii_lowercase();
        needles
            .iter()
            .all(|needle| label.contains(needle) || key.contains(needle))
    };

    let base_color = params
        .rows
        .iter()
        .find(|row| {
            matches!(row.control, MaterialParamControlProjection::Color { .. })
                && (row_matches(row, &["base", "color"]) || row_matches(row, &["albedo"]))
        })
        .or_else(|| {
            params
                .rows
                .iter()
                .find(|row| matches!(row.control, MaterialParamControlProjection::Color { .. }))
        })
        .and_then(|row| match &row.control {
            MaterialParamControlProjection::Color { rgba, .. } => Some(*rgba),
            _ => None,
        });
    match base_color {
        Some(_) => bound.push("base color".to_owned()),
        None => defaulted.push("base color".to_owned()),
    }

    let slider_value = |needle: &str| {
        params
            .rows
            .iter()
            .find(|row| {
                matches!(row.control, MaterialParamControlProjection::Slider { .. })
                    && row_matches(row, &[needle])
            })
            .and_then(|row| match &row.control {
                MaterialParamControlProjection::Slider { fraction, .. } => Some(*fraction),
                _ => None,
            })
    };
    let roughness = slider_value("rough");
    match roughness {
        Some(_) => bound.push("roughness".to_owned()),
        None => defaulted.push("roughness".to_owned()),
    }
    let metallic = slider_value("metal");
    match metallic {
        Some(_) => bound.push("metallic".to_owned()),
        None => defaulted.push("metallic".to_owned()),
    }

    MaterialPreviewMaterialProjection {
        label: params.label.clone(),
        base_color: base_color.unwrap_or(DEFAULT_PREVIEW_BASE_COLOR),
        roughness: roughness.unwrap_or(DEFAULT_PREVIEW_ROUGHNESS),
        metallic: metallic.unwrap_or(DEFAULT_PREVIEW_METALLIC),
        bound,
        defaulted,
    }
}

// --- helpers ---

fn non_empty_str<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if preferred.trim().is_empty() {
        fallback
    } else {
        preferred
    }
}

/// "`materials/graphs/metal_brushed.azmat.ron`" → "`metal_brushed`".
fn source_stem(path: &str) -> String {
    az_editor_ui::naming::display_name(path).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_core::reflect::ReflectedValueEnvelope;
    use az_editor_inspector::{
        AddComponentCapabilities, AddComponentEvaluationState, ReflectedAddComponent,
        ReflectedCurrentValue, ReflectedDefaultAvailability, ReflectedDefaultValue,
        ReflectedEditBinding, ReflectedInspectionModel, ReflectedPrefabSelection,
        ReflectedValidationState,
    };
    use az_editor_ui::panels::{
        GraphDocumentListItemProjectionData, GraphDocumentListProjectionData,
        GraphDocumentProjectionData, GraphNodePaletteItemData, GraphNodePaletteProjectionData,
    };
    use az_proto_project::vnext::{
        FieldConstraints, NumericRange, PrefabComponentSnapshot, PrefabValueTarget, ReflectedPath,
        ReflectedTypeKind,
    };

    fn palette_node(
        node_type: &str,
        label: &str,
        category: &str,
        tags: &[&str],
    ) -> GraphNodePaletteItemData {
        GraphNodePaletteItemData {
            node_type: node_type.to_owned(),
            version: 1,
            label: label.to_owned(),
            category: category.to_owned(),
            description: None,
            input_count: 2,
            output_count: 1,
            default_input_count: 0,
            runtime_bound: true,
            runtime_binding: None,
            source_link_count: 0,
            tags: tags.iter().map(ToString::to_string).collect(),
        }
    }

    fn material_graph_document(
        document_id: &str,
        node_count: usize,
    ) -> GraphDocumentProjectionData {
        GraphDocumentProjectionData {
            document_id: document_id.to_owned(),
            graph_type: MATERIAL_GRAPH_TYPE.to_owned(),
            graph_type_info: None,
            revision: 3,
            saved_revision: Some(3),
            unsaved_changes: false,
            catalog_version: 1,
            nodes: (0..node_count)
                .map(|index| az_editor_ui::panels::GraphNodeProjectionData {
                    node_id: format!("node-{index}"),
                    node_type: "azoth.material.multiply".to_owned(),
                    label: "Multiply".to_owned(),
                    x: 0.0,
                    y: 0.0,
                    width: 160.0,
                    height: 80.0,
                    selected: false,
                    source_links: Vec::new(),
                    ports: Vec::new(),
                })
                .collect(),
            connections: Vec::new(),
            comments: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn graph_projection_fixture() -> EditorGraphDocumentProjection {
        EditorGraphDocumentProjection::document(material_graph_document(
            "materials/graphs/metal.azmat.ron",
            5,
        ))
        .with_graph_documents(GraphDocumentListProjectionData::new(vec![
            GraphDocumentListItemProjectionData {
                document_id: "materials/graphs/metal.azmat.ron".to_owned(),
                graph_type: MATERIAL_GRAPH_TYPE.to_owned(),
                source_path: "materials/graphs/metal.azmat.ron".to_owned(),
                revision: 3,
                saved_revision: Some(2),
                unsaved_changes: true,
                loaded: true,
                current: true,
            },
            GraphDocumentListItemProjectionData {
                document_id: "scripts/ai.azgraph.ron".to_owned(),
                graph_type: "azoth.script.graph".to_owned(),
                source_path: "scripts/ai.azgraph.ron".to_owned(),
                revision: 1,
                saved_revision: Some(1),
                unsaved_changes: false,
                loaded: false,
                current: false,
            },
        ]))
        .with_node_palette(GraphNodePaletteProjectionData::new(vec![
            palette_node(
                "azoth.material.multiply",
                "Multiply",
                "Material/Math",
                &["material"],
            ),
            palette_node(
                "azoth.material.pbr-output",
                "PBR Material Output",
                "Material/Output",
                &["material"],
            ),
            palette_node("azoth.material.uv", "UV", "Material/Input", &["material"]),
            palette_node("azoth.script.print", "Print", "Script/Debug", &["script"]),
        ]))
    }

    #[gpui::test]
    fn document_route_preserves_graph_identity_and_authored_ownership(cx: &gpui::TestAppContext) {
        cx.update(|app| {
            app.set_global(graph_projection_fixture());
            assert_eq!(
                material_document_route(app, "materials/graphs/metal.azmat.ron"),
                MaterialDocumentRoute::Graph("materials/graphs/metal.azmat.ron".to_owned())
            );
            assert_eq!(
                material_document_route(app, "materials/metal.azmaterial.ron"),
                MaterialDocumentRoute::Authored("materials/metal.azmaterial.ron".to_owned())
            );
        });
    }

    fn float_field(name: &str, label: &str, value: f64) -> ReflectedInspectionField {
        reflected_field(
            name,
            label,
            ReflectedValueNode {
                type_path: "f64".to_owned(),
                kind: ReflectedTypeKind::Float { bits: 64 },
                current: ReflectedCurrentValue {
                    authored: Some(ReflectedValue::Scalar(ReflectedScalar::Float(
                        value.to_string(),
                    ))),
                    effective: Some(ReflectedValue::Scalar(ReflectedScalar::Float(
                        value.to_string(),
                    ))),
                },
                default: no_default(),
                binding: binding(name),
                children: Vec::new(),
            },
            WidgetFamily::Number,
        )
    }

    fn scalar_field(
        name: &str,
        label: &str,
        value: ReflectedScalar,
        family: WidgetFamily,
    ) -> ReflectedInspectionField {
        let kind = match &value {
            ReflectedScalar::Bool(_) => ReflectedTypeKind::Bool,
            ReflectedScalar::Signed(_) => ReflectedTypeKind::SignedInteger { bits: 64 },
            ReflectedScalar::Unsigned(_) => ReflectedTypeKind::UnsignedInteger { bits: 64 },
            ReflectedScalar::Float(_) => ReflectedTypeKind::Float { bits: 64 },
            ReflectedScalar::String(_) => ReflectedTypeKind::String,
        };
        let value = ReflectedValue::Scalar(value);
        reflected_field(
            name,
            label,
            ReflectedValueNode {
                type_path: match kind {
                    ReflectedTypeKind::Bool => "bool",
                    ReflectedTypeKind::Float { .. } => "f64",
                    ReflectedTypeKind::String => "alloc::string::String",
                    ReflectedTypeKind::SignedInteger { .. } => "i64",
                    ReflectedTypeKind::UnsignedInteger { .. } => "u64",
                    _ => "value",
                }
                .to_owned(),
                kind,
                current: ReflectedCurrentValue {
                    authored: Some(value.clone()),
                    effective: Some(value),
                },
                default: no_default(),
                binding: binding(name),
                children: Vec::new(),
            },
            family,
        )
    }

    fn struct_field(
        name: &str,
        label: &str,
        family: WidgetFamily,
        children: Vec<ReflectedInspectionField>,
    ) -> ReflectedInspectionField {
        reflected_field(
            name,
            label,
            ReflectedValueNode {
                type_path: format!("test::{name}"),
                kind: ReflectedTypeKind::Struct,
                current: ReflectedCurrentValue {
                    authored: Some(ReflectedValue::Struct(Vec::new())),
                    effective: Some(ReflectedValue::Struct(Vec::new())),
                },
                default: no_default(),
                binding: binding(name),
                children: children
                    .into_iter()
                    .map(Box::new)
                    .map(ReflectedInspectionChild::Field)
                    .collect(),
            },
            family,
        )
    }

    fn reflected_field(
        name: &str,
        label: &str,
        value: ReflectedValueNode,
        family: WidgetFamily,
    ) -> ReflectedInspectionField {
        ReflectedInspectionField {
            name: name.to_owned(),
            label: label.to_owned(),
            description: None,
            read_only: false,
            hidden: false,
            actions: Vec::new(),
            widget: WidgetSpec {
                family,
                range: None,
                rows: None,
                constraints: FieldConstraints::default(),
                variants: Vec::new(),
            },
            validation: ReflectedValidationState::default(),
            value,
        }
    }

    fn binding(field: &str) -> ReflectedEditBinding {
        ReflectedEditBinding::new(PrefabValueTarget {
            instance_alias_chain: Vec::new(),
            entity_alias: "root".to_owned(),
            path: ReflectedPath {
                component_type_path: az_material::MATERIAL_SCHEMA_NAME.to_owned(),
                segments: Vec::new(),
            },
        })
        .field(field)
    }

    fn no_default() -> ReflectedDefaultValue {
        ReflectedDefaultValue {
            availability: ReflectedDefaultAvailability::Unavailable,
            value: None,
        }
    }

    fn color_binding(property: &str, rgba: [f64; 4]) -> ReflectedInspectionField {
        let rgba = struct_field(
            "rgba",
            "rgba",
            WidgetFamily::Color,
            ["r", "g", "b", "a"]
                .into_iter()
                .zip(rgba)
                .map(|(name, value)| float_field(name, name, value))
                .collect(),
        );
        struct_field(
            "0",
            "0",
            WidgetFamily::Struct,
            vec![
                scalar_field(
                    "property",
                    "Property",
                    ReflectedScalar::String(property.to_owned()),
                    WidgetFamily::Text,
                ),
                struct_field("value", "Value", WidgetFamily::Struct, vec![rgba]),
            ],
        )
    }

    fn float_binding(property: &str, value: f64) -> ReflectedInspectionField {
        struct_field(
            "1",
            "1",
            WidgetFamily::Struct,
            vec![
                scalar_field(
                    "property",
                    "Property",
                    ReflectedScalar::String(property.to_owned()),
                    WidgetFamily::Text,
                ),
                float_field("value", "Value", value),
            ],
        )
    }

    fn material_inspection_fixture() -> ReflectedEntityInspection {
        let fields = vec![
            scalar_field(
                "name",
                "Name",
                ReflectedScalar::String("Metal Brushed".to_owned()),
                WidgetFamily::Text,
            ),
            scalar_field(
                "material_type",
                "Material Type",
                ReflectedScalar::String("materials/types/pbr.azmaterialtype.ron".to_owned()),
                WidgetFamily::Asset {
                    asset_type: "material-type".to_owned(),
                },
            ),
            struct_field(
                "property_values",
                "Property Values",
                WidgetFamily::List,
                vec![
                    color_binding("base_color", [0.8, 0.2, 0.1, 1.0]),
                    float_binding("roughness", 0.25),
                    float_binding("metallic", 0.9),
                ],
            ),
        ];
        let component = PrefabComponentSnapshot {
            entity_alias: "root".to_owned(),
            type_path: az_material::MATERIAL_SCHEMA_NAME.to_owned(),
            sparse_value: ReflectedValueEnvelope::typed_ron(
                az_material::MATERIAL_SCHEMA_NAME,
                "()",
            ),
        };
        ReflectedEntityInspection {
            selection: ReflectedPrefabSelection::new("materials/metal.azmaterial.ron", "root"),
            registry_schema_catalog_hash: vec![1; 32],
            document_version: 1,
            type_versions: BTreeMap::new(),
            revision: 1,
            components: vec![ReflectedComponentInspection {
                component,
                model: ReflectedInspectionModel {
                    schema_catalog_hash: vec![1; 32],
                    entity_alias: "root".to_owned(),
                    type_path: az_material::MATERIAL_SCHEMA_NAME.to_owned(),
                    type_label: "Material".to_owned(),
                    category: None,
                    icon: None,
                    description: None,
                    fields,
                    actions: Vec::new(),
                    validation: ReflectedValidationState::default(),
                    add_component: ReflectedAddComponent {
                        editor_export: true,
                        runtime_export: true,
                        default_available: true,
                        evaluation: AddComponentEvaluationState::NotProjected,
                        capabilities: AddComponentCapabilities::NotProjected,
                    },
                },
            }],
            overrides: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn palette_groups_material_nodes_by_subcategory_and_excludes_other_libraries() {
        let graph = graph_projection_fixture();
        let projection =
            build_materials_projection(Some(&graph), None, None, &MaterialsUiState::default());

        assert!(projection.graph_ready);
        assert_eq!(projection.palette.total_count, 3);
        let names = projection
            .palette
            .categories
            .iter()
            .map(|category| category.name.as_str())
            .collect::<Vec<_>>();
        // Alphabetical with the terminal Output category forced last.
        assert_eq!(names, ["Input", "Math", "Output"]);
        assert!(
            projection
                .palette
                .categories
                .iter()
                .flat_map(|category| &category.items)
                .all(|item| item.node_type.starts_with("azoth.material.")),
            "script nodes must not leak into the material palette"
        );
        let output = projection
            .palette
            .categories
            .last()
            .and_then(|category| category.items.first())
            .expect("output item");
        assert!(output.output);
        assert_eq!(output.port_summary, "2 in · 1 out");
    }

    #[test]
    fn palette_target_follows_current_material_graph_and_grid_placement() {
        let graph = graph_projection_fixture();
        let projection =
            build_materials_projection(Some(&graph), None, None, &MaterialsUiState::default());

        let target = projection.palette.target.expect("palette target");
        assert_eq!(target.document_id, "materials/graphs/metal.azmat.ron");
        // 5 nodes: index 5 → column 1, row 1 of the 4-wide grid.
        assert!((target.next_x - 320.0).abs() < f32::EPSILON);
        assert!((target.next_y - 240.0).abs() < f32::EPSILON);
        assert_eq!(
            projection.canvas.material_graph_document_id.as_deref(),
            Some("materials/graphs/metal.azmat.ron")
        );
    }

    #[test]
    fn palette_filter_narrows_items_and_reports_filter() {
        let graph = graph_projection_fixture();
        let state = MaterialsUiState {
            palette_filter: "mult".to_owned(),
            ..MaterialsUiState::default()
        };
        let projection = build_materials_projection(Some(&graph), None, None, &state);

        assert_eq!(projection.palette.filter, "mult");
        assert_eq!(projection.palette.total_count, 3);
        assert_eq!(projection.palette.categories.len(), 1);
        assert_eq!(projection.palette.categories[0].items[0].label, "Multiply");
    }

    #[test]
    fn tabs_join_material_graph_documents_and_skip_other_graph_types() {
        let graph = graph_projection_fixture();
        let state = MaterialsUiState {
            selected_document_id: "materials/graphs/metal.azmat.ron".to_owned(),
            ..MaterialsUiState::default()
        };
        let projection = build_materials_projection(Some(&graph), None, None, &state);

        assert_eq!(projection.documents.len(), 1);
        let tab = &projection.documents[0];
        assert_eq!(tab.label, "metal");
        assert_eq!(tab.kind, MaterialDocumentKind::Graph);
        assert!(tab.selected);
        assert!(tab.unsaved_changes);
    }

    #[test]
    fn preview_parameterizes_from_material_property_bindings() {
        let inspection = material_inspection_fixture();
        let projection =
            build_materials_projection(None, None, Some(&inspection), &MaterialsUiState::default());

        let params = projection.params.expect("material params");
        assert_eq!(params.schema_label, "Material");
        assert_eq!(params.label, "metal");
        let labels = params
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "Name",
                "Material Type",
                "base_color",
                "roughness",
                "metallic"
            ]
        );
        assert!(matches!(
            params.rows[1].control,
            MaterialParamControlProjection::Texture { .. }
        ));
        assert!(matches!(
            params.rows[2].control,
            MaterialParamControlProjection::Color { .. }
        ));

        let material = projection.preview.material.expect("preview material");
        assert!((material.base_color[0] - 0.8).abs() < 1e-6);
        assert!((material.base_color[1] - 0.2).abs() < 1e-6);
        assert!((material.roughness - 0.25).abs() < 1e-6);
        assert!((material.metallic - 0.9).abs() < 1e-6);
        assert_eq!(material.bound, ["base color", "roughness", "metallic"]);
        assert!(material.defaulted.is_empty());
    }

    #[test]
    fn preview_defaults_missing_inputs_and_reports_them() {
        let mut inspection = material_inspection_fixture();
        // Keep only the roughness binding.
        inspection.components[0].model.fields[2].value.children =
            vec![ReflectedInspectionChild::Field(Box::new(float_binding(
                "roughness",
                0.7,
            )))];
        let projection =
            build_materials_projection(None, None, Some(&inspection), &MaterialsUiState::default());

        let material = projection.preview.material.expect("preview material");
        // `DEFAULT_PREVIEW_BASE_COLOR` is copied verbatim when no binding
        // supplies one, so bit equality is exactly the property under test.
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(material.base_color, DEFAULT_PREVIEW_BASE_COLOR);
        }
        assert!((material.roughness - 0.7).abs() < 1e-6);
        assert!((material.metallic - DEFAULT_PREVIEW_METALLIC).abs() < f32::EPSILON);
        assert_eq!(material.bound, ["roughness"]);
        assert_eq!(material.defaulted, ["base color", "metallic"]);
    }

    #[test]
    fn params_are_absent_for_non_material_documents() {
        let mut inspection = material_inspection_fixture();
        inspection.components[0].model.type_path = "azoth.prefab.Prefab".to_owned();
        inspection.components[0].component.type_path = "azoth.prefab.Prefab".to_owned();
        let projection =
            build_materials_projection(None, None, Some(&inspection), &MaterialsUiState::default());

        assert!(projection.params.is_none());
        assert!(projection.preview.material.is_none());
    }

    #[test]
    fn slider_fraction_normalizes_against_widget_range() {
        let mut field = float_field("intensity", "Intensity", 5.0);
        field.widget.family = WidgetFamily::Slider;
        field.widget.range = Some(NumericRange {
            minimum: Some("0".to_owned()),
            maximum: Some("10".to_owned()),
            step: None,
            suffix: None,
        });
        let rows = param_rows_from_fields(&[field]);

        assert_eq!(rows.len(), 1);
        let MaterialParamControlProjection::Slider { fraction, display } = &rows[0].control else {
            panic!("expected slider control");
        };
        assert!((fraction - 0.5).abs() < f32::EPSILON);
        assert_eq!(display, "5.000");
    }

    #[test]
    fn canvas_reports_honest_empty_reasons() {
        let projection = build_materials_projection(None, None, None, &MaterialsUiState::default());
        assert!(!projection.graph_ready);
        assert!(
            projection
                .canvas
                .empty_reason
                .contains("Graph controller not connected")
        );

        let graph = EditorGraphDocumentProjection::empty();
        let projection =
            build_materials_projection(Some(&graph), None, None, &MaterialsUiState::default());
        assert!(projection.canvas.material_graph_document_id.is_none());
        assert!(projection.canvas.empty_reason.contains(".azmat.ron"));
        assert!(projection.palette.target.is_none());
    }
}
