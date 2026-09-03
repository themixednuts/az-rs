//! Decoded runtime shape for a processed static mesh product.

/// Stable imported material slot metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshMaterialSlot {
    pub id: u32,
    pub label: String,
    /// Compiled material product path selected by the importer, when present.
    pub default_material: Option<String>,
}

/// One triangle-list primitive. Primitive boundaries are retained because a
/// Bevy `MeshMaterial3d` applies to one mesh entity.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshPrimitive {
    pub label: String,
    pub material_slot: u32,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

#[cfg(feature = "bevy")]
impl MeshPrimitive {
    /// Build the renderer's GPU-uploadable mesh from this decoded product
    /// primitive. Product loading itself stays renderer-independent.
    #[must_use]
    pub fn to_bevy_mesh(&self) -> bevy::mesh::Mesh {
        use bevy::asset::RenderAssetUsages;
        use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions.clone());
        if !self.normals.is_empty() {
            mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals.clone());
        }
        if !self.tangents.is_empty() {
            mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, self.tangents.clone());
        }
        if !self.uv0.is_empty() {
            mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv0.clone());
        }
        mesh.with_inserted_indices(Indices::U32(self.indices.clone()))
    }
}

/// CPU-decoded processed mesh/model product.
#[cfg_attr(feature = "bevy", derive(bevy::asset::Asset, bevy::reflect::TypePath))]
#[derive(Debug, Clone, PartialEq)]
pub struct MeshAsset {
    pub name: String,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub material_slots: Vec<MeshMaterialSlot>,
    pub primitives: Vec<MeshPrimitive>,
}
