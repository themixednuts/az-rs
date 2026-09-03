//! Materials mode projection data.
//!
//! Editor-core owns the materials boundary (material graph documents through
//! the visual-graph controller, authored material property documents through
//! the authored-selection controller) and publishes
//! [`EditorMaterialsProjection`] for the Materials mode panels to render. The
//! panels deliberately know nothing about graph controllers, schema catalogs,
//! or project-host; they render this projection and dispatch typed
//! `Material*` / graph actions from [`crate::actions`].

use gpui::Global;

/// Preview swatch geometry selected by the shape-switcher chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaterialPreviewShape {
    #[default]
    Sphere,
    Cube,
    Plane,
    Mesh,
}

impl MaterialPreviewShape {
    /// All switcher chips in display order.
    pub const ALL: [Self; 4] = [Self::Sphere, Self::Cube, Self::Plane, Self::Mesh];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sphere => "Sphere",
            Self::Cube => "Cube",
            Self::Plane => "Plane",
            Self::Mesh => "Mesh",
        }
    }
}

/// Which authored source kind a material document tab refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialDocumentKind {
    /// `.azmat.ron` material node-graph source.
    Graph,
    /// `.azmaterial.ron` material instance source.
    Material,
    /// `.azmaterialtype.ron` material type source.
    MaterialType,
}

impl MaterialDocumentKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Material => "material",
            Self::MaterialType => "type",
        }
    }
}

/// Root Materials mode projection global published by editor-core.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditorMaterialsProjection {
    /// Whether the graph controller has published its projection at least once.
    pub graph_ready: bool,
    /// Material document tabs (graph sources + authored property documents).
    pub documents: Vec<MaterialDocumentTabProjection>,
    pub palette: MaterialPaletteProjection,
    pub canvas: MaterialCanvasProjection,
    pub preview: MaterialPreviewProjection,
    /// Parameter table for the loaded material property document, when the
    /// loaded authored document is a material instance or material type.
    pub params: Option<MaterialParamsProjection>,
}

impl Global for EditorMaterialsProjection {}

/// One open material document tab in the workbench toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialDocumentTabProjection {
    pub document_id: String,
    /// Short display label (file stem).
    pub label: String,
    pub kind: MaterialDocumentKind,
    pub selected: bool,
    pub unsaved_changes: bool,
}

/// Node palette scoped to the material node library.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialPaletteProjection {
    pub categories: Vec<MaterialPaletteCategoryProjection>,
    /// Material node count before the filter was applied.
    pub total_count: usize,
    pub filter: String,
    /// Present when `AddGraphNode` currently targets an open material graph
    /// document; palette rows are inert without it.
    pub target: Option<MaterialGraphTargetProjection>,
}

/// One palette category (material node subcategory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialPaletteCategoryProjection {
    pub name: String,
    pub items: Vec<MaterialPaletteItemProjection>,
}

/// One addable material node type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialPaletteItemProjection {
    pub node_type: String,
    pub version: u32,
    pub label: String,
    pub description: Option<String>,
    /// e.g. "2 in · 1 out".
    pub port_summary: String,
    /// Terminal material-output node.
    pub output: bool,
}

/// Where a palette activation adds its node.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialGraphTargetProjection {
    pub document_id: String,
    /// Document-space position for the next added node.
    pub next_x: f32,
    pub next_y: f32,
}

/// Center canvas hosting state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterialCanvasProjection {
    /// The graph controller's current document when it is a material graph;
    /// the workbench hosts the graph canvas only in that case.
    pub material_graph_document_id: Option<String>,
    pub unsaved_changes: bool,
    /// Why the canvas is empty when no material graph is hosted.
    pub empty_reason: String,
}

/// Preview swatch state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialPreviewProjection {
    pub shape: MaterialPreviewShape,
    /// Property-derived preview inputs; `None` renders the explicit empty state.
    pub material: Option<MaterialPreviewMaterialProjection>,
}

/// Property-derived preview parameterization (rendering brief B3 v1: base
/// color drives tint, roughness drives highlight size, metallic drives
/// contrast). These values are document data, not design chrome.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialPreviewMaterialProjection {
    pub label: String,
    pub base_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    /// Preview inputs resolved from authored property values.
    pub bound: Vec<String>,
    /// Preview inputs that fell back to neutral defaults.
    pub defaulted: Vec<String>,
}

/// Parameter table for the loaded material property document.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParamsProjection {
    pub document_id: String,
    pub label: String,
    /// "Material" or "Material Type".
    pub schema_label: String,
    pub rows: Vec<MaterialParamRowProjection>,
}

/// One parameter row (label + typed read-only control).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParamRowProjection {
    pub key: String,
    pub label: String,
    pub control: MaterialParamControlProjection,
}

/// Classified parameter control, mirroring the design's slider / color /
/// texture row kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum MaterialParamControlProjection {
    Slider { fraction: f32, display: String },
    Color { rgba: [f32; 4], display: String },
    Texture { path: String },
    Toggle { value: bool },
    Value { display: String },
}
