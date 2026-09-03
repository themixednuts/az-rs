//! Vertex shape asset transformation.

use thiserror::Error;

use az_gem_lmbr_central::{
    VertexShapeAsset as EngineVertexShapeAsset, VertexShapeMetadata as EngineVertexShapeMetadata,
    VertexShapeReserved as EngineVertexShapeReserved,
};

/// Converts a native `.vshapec` payload into the engine vertex-shape asset.
///
/// # Errors
///
/// Returns [`VertexShapeTransformError::Parse`] wrapping any
/// [`lmbr_central_vshape::ParseError`] the reader raises — a truncated header,
/// a vertex or metadata count that runs past the end of the payload, or a
/// metadata string that is not UTF-8.
pub fn transform_vertex_shape_asset(
    bytes: &[u8],
) -> Result<EngineVertexShapeAsset, VertexShapeTransformError> {
    let source = lmbr_central_vshape::parse_vertex_shape_asset(bytes)?;
    Ok(EngineVertexShapeAsset::new(
        source.version(),
        source.vertices().iter().collect(),
        source
            .metadata()
            .iter()
            .map(|entry| EngineVertexShapeMetadata::new(entry.key, entry.value))
            .collect(),
        source.height(),
        EngineVertexShapeReserved::new(
            source.reserved().first,
            source.reserved().second,
            source.reserved().third,
        ),
    ))
}

#[derive(Debug, Error)]
pub enum VertexShapeTransformError {
    #[error("parse vertex shape asset: {0}")]
    Parse(#[from] lmbr_central_vshape::ParseError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    #[test]
    fn transforms_vertex_shape_to_engine_asset() {
        let asset = transform_vertex_shape_asset(&sample_bytes()).unwrap();

        assert_eq!(asset.version, 0);
        assert_eq!(asset.vertices, vec![Vec3::new(1.0, 2.0, 0.0)]);
        assert_eq!(asset.metadata[0].key, "TerritoryId");
        assert_eq!(asset.metadata[0].value, "14:@Example");
        assert_exact(asset.height, 32.0);
        assert_eq!(asset.reserved.first, 0);
        assert_eq!(asset.reserved.second, 0);
        assert_eq!(asset.reserved.third, 7);
    }

    /// Compares a round-tripped `f32` bit-exactly.
    ///
    /// The fixture writes the same little-endian pattern the reader decodes,
    /// so any difference is a decode bug rather than accumulated error; an
    /// epsilon window would hide exactly the bugs this asserts against.
    #[track_caller]
    fn assert_exact(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    fn sample_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        write_string(&mut bytes, "TerritoryId");
        write_string(&mut bytes, "14:@Example");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&32.0f32.to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes
    }

    fn write_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}
