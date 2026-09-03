use az_core::AssetPathBuf;
use serde::{Deserialize, Serialize};

use crate::MaterialPropertyBinding;

/// Editable material instance source.
///
/// A material instance selects a material type and supplies typed property
/// overrides. Shader graph selection is owned by the material type so builders
/// have one resolution path: material -> type -> graph -> products.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialSource {
    pub name: String,

    pub material_type: AssetPathBuf,

    pub parent: Option<AssetPathBuf>,

    pub property_values: Vec<MaterialPropertyBinding>,
}
