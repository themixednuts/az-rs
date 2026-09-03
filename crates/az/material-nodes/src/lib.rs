//! Azoth built-in material graph node library.
//!
//! Defines a real set of material `NodeTypeDescriptor`s and a `Material`
//! `GraphTypeDescriptor`, registered through the engine's inventory system so
//! that project-host serves them into the editor's visual-graph editor. Authors
//! create a `Material` graph and wire these nodes; the graph lowers to a shader
//! via the `shader_pipeline` compiler backend.
//!
//! The node descriptors, ports, graph type, palette policy, and catalog wiring
//! are fully real and served. The graph→shader code generation behind the
//! `shader_pipeline` backend is the one explicit TODO (see
//! [`MATERIAL_PIPELINE_KIND`]); every node carries an `External { kind: "shader-node" }`
//! runtime binding the compiler interprets.

use az_core::AssetPathBuf;
use az_node_graph::{
    GraphCompilerBackendDescriptor, GraphNodeCatalogRequirement, GraphPalettePolicy,
    GraphSourceWorkflow, GraphTypeDescriptor, GraphTypeRegistration, NodeCapability,
    NodePortDescriptor, NodePortId, NodeRuntimeBinding, NodeTypeDescriptor, NodeTypeRegistration,
    RuntimeGraphExecutionStrategy, RuntimeGraphProductDescriptor, VisualGraphType, VisualNode,
};
use glam::{Vec2, Vec3, Vec4};

/// Node-catalog id the Material graph draws its palette from.
pub const MATERIAL_NODE_CATALOG: &str = "azoth.material.nodes";
/// Capability every material node carries (used by the Material palette policy).
pub const MATERIAL_NODE_CAPABILITY: &str = "azoth.material.node";
/// Graph type id for material graphs.
pub const MATERIAL_GRAPH_TYPE: &str = az_material::MATERIAL_GRAPH_ASSET_TYPE_HINT;
/// Shared pipeline-kind string linking the compiler backend to the runtime
/// strategy. The graph→shader codegen behind this kind is the remaining TODO.
pub const MATERIAL_PIPELINE_KIND: &str = "azoth.material.shader-pipeline";

/// Ids of every node this library registers (also the inventory anchor count).
pub const MATERIAL_NODE_IDS: &[&str] = &[
    "azoth.material.constant-float",
    "azoth.material.constant-color",
    "azoth.material.uv",
    "azoth.material.vertex-normal",
    "azoth.material.texture-sample",
    "azoth.material.multiply",
    "azoth.material.add",
    "azoth.material.lerp",
    "azoth.material.saturate",
    "azoth.material.pbr-output",
];

/// Common descriptor scaffold for a material node: category under `Material/…`,
/// the material capability, a shader-node runtime binding, and the `material` tag.
fn material_node(id: &str, name: &str, subcategory: &str, description: &str) -> NodeTypeDescriptor {
    NodeTypeDescriptor::new(id, 1, name)
        .with_category_path(["Material".to_string(), subcategory.to_string()])
        .with_description(description)
        .with_capability(NodeCapability::new(MATERIAL_NODE_CAPABILITY))
        .with_runtime_binding(NodeRuntimeBinding::External {
            kind: "shader-node".to_string(),
            locator: id.to_string(),
        })
        .with_tag("material")
}

// ---------------------------------------------------------------------------
// Constants & inputs
// ---------------------------------------------------------------------------

struct ConstantFloatNode;
impl VisualNode for ConstantFloatNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.constant-float",
            "Constant Float",
            "Constants",
            "A constant scalar value.",
        )
        .with_port(NodePortDescriptor::data_output::<f32>(
            NodePortId::new(1),
            "value",
        ))
    }
}

struct ConstantColorNode;
impl VisualNode for ConstantColorNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.constant-color",
            "Constant Color",
            "Constants",
            "A constant RGBA color.",
        )
        .with_port(NodePortDescriptor::data_output::<Vec4>(
            NodePortId::new(1),
            "color",
        ))
    }
}

struct UvNode;
impl VisualNode for UvNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.uv",
            "UV",
            "Input",
            "Interpolated texture coordinates of the fragment.",
        )
        .with_port(NodePortDescriptor::data_output::<Vec2>(
            NodePortId::new(1),
            "uv",
        ))
    }
}

struct VertexNormalNode;
impl VisualNode for VertexNormalNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.vertex-normal",
            "Vertex Normal",
            "Input",
            "Interpolated world-space surface normal.",
        )
        .with_port(NodePortDescriptor::data_output::<Vec3>(
            NodePortId::new(1),
            "normal",
        ))
    }
}

struct TextureSampleNode;
impl VisualNode for TextureSampleNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.texture-sample",
            "Texture Sample",
            "Texture",
            "Samples a texture asset at the given UV coordinates.",
        )
        .with_port(NodePortDescriptor::data_input::<AssetPathBuf>(
            NodePortId::new(1),
            "texture",
        ))
        .with_port(NodePortDescriptor::data_input::<Vec2>(
            NodePortId::new(2),
            "uv",
        ))
        .with_port(NodePortDescriptor::data_output::<Vec4>(
            NodePortId::new(3),
            "color",
        ))
    }
}

// ---------------------------------------------------------------------------
// Math
// ---------------------------------------------------------------------------

struct MultiplyNode;
impl VisualNode for MultiplyNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.multiply",
            "Multiply",
            "Math",
            "Component-wise multiply of two values.",
        )
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(1),
            "a",
        ))
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(2),
            "b",
        ))
        .with_port(NodePortDescriptor::data_output::<Vec4>(
            NodePortId::new(3),
            "result",
        ))
    }
}

struct AddNode;
impl VisualNode for AddNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.add",
            "Add",
            "Math",
            "Component-wise add of two values.",
        )
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(1),
            "a",
        ))
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(2),
            "b",
        ))
        .with_port(NodePortDescriptor::data_output::<Vec4>(
            NodePortId::new(3),
            "result",
        ))
    }
}

struct LerpNode;
impl VisualNode for LerpNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.lerp",
            "Lerp",
            "Math",
            "Linearly interpolates between a and b by t.",
        )
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(1),
            "a",
        ))
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(2),
            "b",
        ))
        .with_port(NodePortDescriptor::data_input::<f32>(
            NodePortId::new(3),
            "t",
        ))
        .with_port(NodePortDescriptor::data_output::<Vec4>(
            NodePortId::new(4),
            "result",
        ))
    }
}

struct SaturateNode;
impl VisualNode for SaturateNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.saturate",
            "Saturate",
            "Math",
            "Clamps each component to the [0, 1] range.",
        )
        .with_port(NodePortDescriptor::data_input::<Vec4>(
            NodePortId::new(1),
            "value",
        ))
        .with_port(NodePortDescriptor::data_output::<Vec4>(
            NodePortId::new(2),
            "result",
        ))
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

struct PbrMaterialOutputNode;
impl VisualNode for PbrMaterialOutputNode {
    fn node_type_descriptor() -> NodeTypeDescriptor {
        material_node(
            "azoth.material.pbr-output",
            "PBR Material Output",
            "Output",
            "Terminal node: the surface inputs the renderer shades with.",
        )
        // The `output` marker identifies the graph's terminal material node.
        .with_capability(NodeCapability::new(MATERIAL_NODE_CAPABILITY).with_marker("output"))
        .with_port(NodePortDescriptor::data_input::<Vec3>(
            NodePortId::new(1),
            "base_color",
        ))
        .with_port(NodePortDescriptor::data_input::<f32>(
            NodePortId::new(2),
            "metallic",
        ))
        .with_port(NodePortDescriptor::data_input::<f32>(
            NodePortId::new(3),
            "roughness",
        ))
        .with_port(NodePortDescriptor::data_input::<Vec3>(
            NodePortId::new(4),
            "normal",
        ))
        .with_port(NodePortDescriptor::data_input::<Vec3>(
            NodePortId::new(5),
            "emissive",
        ))
    }
}

// ---------------------------------------------------------------------------
// Material graph type
// ---------------------------------------------------------------------------

struct MaterialGraph;
impl VisualGraphType for MaterialGraph {
    fn graph_type_descriptor() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            MATERIAL_GRAPH_TYPE,
            1,
            "Material",
            GraphSourceWorkflow::file(
                az_material::MATERIAL_GRAPH_SOURCE_SCHEMA_NAME,
                az_material::MATERIAL_GRAPH_EXTENSION,
            ),
            // TODO: the graph -> shader code generation behind this backend is
            // not implemented yet; the node library, descriptors, graph type,
            // palette, and catalog wiring are fully real and served.
            GraphCompilerBackendDescriptor::shader_pipeline(
                "azoth.material.compiler",
                MATERIAL_PIPELINE_KIND,
            ),
            RuntimeGraphProductDescriptor::new(
                "azoth.material.shader",
                "azoth.material.shader",
                RuntimeGraphExecutionStrategy::shader_pipeline(MATERIAL_PIPELINE_KIND),
            ),
        )
        .with_category_path(["Rendering".to_string()])
        .with_node_catalog(GraphNodeCatalogRequirement::new(MATERIAL_NODE_CATALOG))
        .with_palette_policy(
            GraphPalettePolicy::default()
                .with_root_category("Material")
                .with_required_node_capability(MATERIAL_NODE_CAPABILITY),
        )
        .with_tag("material")
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The built-in material node library, in palette order.
///
/// A host's contribution glue registers these through its node-type registrar;
/// the order here is the order they compose in.
#[must_use]
pub fn node_types() -> [NodeTypeRegistration; MATERIAL_NODE_IDS.len()] {
    [
        NodeTypeRegistration::of::<ConstantFloatNode>(),
        NodeTypeRegistration::of::<ConstantColorNode>(),
        NodeTypeRegistration::of::<UvNode>(),
        NodeTypeRegistration::of::<VertexNormalNode>(),
        NodeTypeRegistration::of::<TextureSampleNode>(),
        NodeTypeRegistration::of::<MultiplyNode>(),
        NodeTypeRegistration::of::<AddNode>(),
        NodeTypeRegistration::of::<LerpNode>(),
        NodeTypeRegistration::of::<SaturateNode>(),
        NodeTypeRegistration::of::<PbrMaterialOutputNode>(),
    ]
}

/// The Material graph type this crate's node library belongs to.
#[must_use]
pub fn graph_types() -> [GraphTypeRegistration; 1] {
    [GraphTypeRegistration::of::<MaterialGraph>()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::{
        ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
        GemId, GemTargetRole, ProductActivation, declare_caps,
    };
    use az_node_graph::{
        NodeTypeCatalog, NodeTypeId, graph_types as composed_graph_types,
        node_types as composed_node_types,
    };

    declare_caps!(MaterialCaps:);

    /// The material node library contributed the way a host's glue contributes
    /// it.
    struct Material;

    impl Contribution for Material {
        type Caps = MaterialCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.material-nodes"),
                contribution: ContributionId::new("nodes"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, MaterialCaps>) {
            ctx.registrar::<NodeTypeRegistration>()
                .register_many(node_types());
            ctx.registrar::<GraphTypeRegistration>()
                .register_many(graph_types());
        }
    }

    fn compose() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Material, ProductActivation::default())
            .expect("material nodes require no host capability");
        composer
    }

    #[test]
    fn every_material_node_composes() {
        let composer = compose();
        let registered = composed_node_types(composer.registries());
        for id in MATERIAL_NODE_IDS {
            assert!(
                registered
                    .iter()
                    .any(|node| node.id == NodeTypeId::from(*id)),
                "material node `{id}` should compose into the node-type registry"
            );
        }

        let report = composer.finalize().expect("composition is valid");
        let keys = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "node-type")
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys.len(), MATERIAL_NODE_IDS.len());
        assert_eq!(keys[0], "azoth.material.constant-float@1");
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.instance.gem.as_str() == "azoth.material-nodes"),
            "every entry is attributed to the contributing instance"
        );
    }

    #[test]
    fn the_material_library_registered_twice_fails_composition() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Material, ProductActivation::default())
            .unwrap();
        composer
            .add(Material, ProductActivation::default())
            .unwrap();

        let error = composer.finalize().unwrap_err();
        let ComposeError::Duplicate { registry, key, .. } = error else {
            panic!("expected duplicate error, got {error}");
        };
        assert_eq!(registry, "graph-type");
        assert_eq!(key, "azoth.material.graph@1");
    }

    #[test]
    fn material_graph_type_composes() {
        let composer = compose();
        let registered = composed_graph_types(composer.registries());
        assert!(
            registered
                .iter()
                .any(|graph| graph.id.as_str() == MATERIAL_GRAPH_TYPE),
            "material graph type should compose into the graph-type registry"
        );
    }

    #[test]
    fn material_graph_uses_shared_material_source_identity() {
        let composer = compose();
        let graph = composed_graph_types(composer.registries())
            .into_iter()
            .find(|graph| graph.id.as_str() == MATERIAL_GRAPH_TYPE)
            .expect("material graph type composed");

        assert_eq!(
            graph.source_workflow.workflow_id,
            az_material::MATERIAL_GRAPH_SOURCE_SCHEMA_NAME
        );
        assert_eq!(
            graph.source_workflow.default_extension.as_deref(),
            Some(az_material::MATERIAL_GRAPH_EXTENSION)
        );
    }

    #[test]
    fn composed_node_catalog_validates_with_material_nodes() {
        // Building the catalog from the composed nodes validates every
        // descriptor (ports, ids, etc.) — including all material nodes.
        let composer = compose();
        let catalog = NodeTypeCatalog::compose(1, 0, composer.registries())
            .expect("composed node catalog (incl. material nodes) must validate");
        for id in MATERIAL_NODE_IDS {
            assert!(
                catalog
                    .node_types
                    .iter()
                    .any(|node| node.id == NodeTypeId::from(*id)),
                "validated catalog must contain material node `{id}`"
            );
        }
    }

    #[test]
    fn material_nodes_satisfy_material_palette_policy() {
        // Every composed material node must pass the Material graph's palette
        // policy (category `Material/…` + the material capability), so they show
        // in the material editor's palette and only there.
        let composer = compose();
        let graph = composed_graph_types(composer.registries())
            .into_iter()
            .find(|graph| graph.id.as_str() == MATERIAL_GRAPH_TYPE)
            .expect("material graph type composed");
        let policy = &graph.palette_policy;
        assert!(policy.root_categories.iter().any(|root| root == "Material"));
        assert!(
            policy
                .required_node_capabilities
                .iter()
                .any(|cap| cap == MATERIAL_NODE_CAPABILITY)
        );

        for node in composed_node_types(composer.registries())
            .into_iter()
            .filter(|node| node.id.as_str().starts_with("azoth.material."))
        {
            assert_eq!(
                node.category_path.first().map(String::as_str),
                Some("Material"),
                "node `{}` must be under the Material category",
                node.id.as_str()
            );
            assert!(
                node.capabilities
                    .iter()
                    .any(|cap| cap.id == MATERIAL_NODE_CAPABILITY),
                "node `{}` must carry the material capability",
                node.id.as_str()
            );
        }
    }
}
