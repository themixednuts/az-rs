use az_core::AssetPathBuf;
use serde::{Deserialize, Serialize};

use crate::{MaterialDomain, MaterialPropertyGroup, ShadingModel};

/// Editable material type source.
///
/// A material type defines the reusable property interface and render-state
/// contract for material instances. It points at the material graph that
/// compiles into shader/runtime products.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialTypeSource {
    pub name: String,

    pub description: String,

    pub domain: MaterialDomain,

    pub blend_mode: crate::BlendMode,

    pub cull_mode: crate::CullMode,

    pub shading_model: ShadingModel,

    pub shader_graph: AssetPathBuf,

    pub property_groups: Vec<MaterialPropertyGroup>,
}
