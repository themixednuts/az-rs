//! Offline conversion from public Cry geometry chunks to GLB 2.0.
//!
//! The bridge accepts [`cry_chunk::CryModel`] and builds a small owned scene
//! before serialization. It does not interpret animation or physics chunks.

mod convert;
mod glb;

use cry_chunk::CryModel;

pub use convert::{ConversionError, MaterialInput, PbrMaterial};

/// Convert a parsed Cry model with neutral metallic-roughness materials.
///
/// # Errors
///
/// Returns [`ConversionError`] when required public streams are absent,
/// malformed, or use an unsupported encoding.
pub fn to_glb(model: &CryModel<'_>) -> Result<Vec<u8>, ConversionError> {
    to_glb_with_materials(model, |_| PbrMaterial::default())
}

/// Convert a parsed Cry model and resolve neutral PBR inputs by material slot.
///
/// The resolver receives stable chunk and slot identifiers plus the public
/// material name. Returning the same input for the same descriptor makes the
/// complete GLB byte-for-byte deterministic.
///
/// # Errors
///
/// Returns [`ConversionError`] when conversion or GLB serialization fails.
pub fn to_glb_with_materials<F>(
    model: &CryModel<'_>,
    resolve_material: F,
) -> Result<Vec<u8>, ConversionError>
where
    F: Fn(&MaterialInput<'_>) -> PbrMaterial,
{
    let scene = convert::convert(model, &resolve_material)?;
    glb::serialize(&scene)
}
