use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::convert::{ConversionError, Scene, SceneMesh};

const ARRAY_BUFFER: u32 = 34_962;
const ELEMENT_ARRAY_BUFFER: u32 = 34_963;
const FLOAT: u32 = 5_126;
const UNSIGNED_BYTE: u32 = 5_121;
const UNSIGNED_SHORT: u32 = 5_123;
const UNSIGNED_INT: u32 = 5_125;

pub fn serialize(scene: &Scene) -> Result<Vec<u8>, ConversionError> {
    let mut builder = Builder::default();
    let mesh_json = scene
        .meshes
        .iter()
        .map(|mesh| builder.mesh(mesh))
        .collect::<Vec<_>>();
    let skin_json = scene.skin.as_ref().map(|skin| builder.skin(skin));
    let nodes = scene
        .nodes
        .iter()
        .map(|node| {
            let mut value = Map::new();
            value.insert("name".into(), json!(node.name));
            value.insert("matrix".into(), json!(node.matrix.column_major()));
            if !node.children.is_empty() {
                value.insert("children".into(), json!(node.children));
            }
            if let Some(mesh) = node.mesh {
                value.insert("mesh".into(), json!(mesh));
            }
            if let Some(skin) = node.skin {
                value.insert("skin".into(), json!(skin));
            }
            Value::Object(value)
        })
        .collect::<Vec<_>>();
    let materials = scene
        .materials
        .iter()
        .map(|material| {
            json!({
                "name": material.name,
                "pbrMetallicRoughness": {
                    "baseColorFactor": material.pbr.base_color_factor,
                    "metallicFactor": material.pbr.metallic_factor,
                    "roughnessFactor": material.pbr.roughness_factor,
                }
            })
        })
        .collect::<Vec<_>>();
    let mut document = Map::new();
    document.insert(
        "asset".into(),
        json!({"generator": "cry-to-gltf", "version": "2.0"}),
    );
    document.insert("scene".into(), json!(0));
    document.insert("scenes".into(), json!([{"nodes": scene.root_nodes}]));
    document.insert("nodes".into(), Value::Array(nodes));
    document.insert("meshes".into(), Value::Array(mesh_json));
    if !materials.is_empty() {
        document.insert("materials".into(), Value::Array(materials));
    }
    if let Some(skin) = skin_json {
        document.insert("skins".into(), json!([skin]));
    }
    document.insert("accessors".into(), Value::Array(builder.accessors));
    document.insert("bufferViews".into(), Value::Array(builder.buffer_views));
    document.insert(
        "buffers".into(),
        json!([{"byteLength": builder.binary.len()}]),
    );
    encode_glb(&Value::Object(document), builder.binary)
}

#[derive(Default)]
struct Builder {
    binary: Vec<u8>,
    buffer_views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    fn mesh(&mut self, mesh: &SceneMesh) -> Value {
        let mut attributes = BTreeMap::new();
        let (position_min, position_max) = bounds(&mesh.positions);
        attributes.insert(
            "POSITION",
            self.f32_accessor(
                &mesh.positions,
                "VEC3",
                Some(position_min),
                Some(position_max),
                Some(ARRAY_BUFFER),
            ),
        );
        if let Some(values) = &mesh.normals {
            attributes.insert(
                "NORMAL",
                self.f32_accessor(values, "VEC3", None, None, Some(ARRAY_BUFFER)),
            );
        }
        if let Some(values) = &mesh.texture_coordinates {
            attributes.insert(
                "TEXCOORD_0",
                self.f32_accessor(values, "VEC2", None, None, Some(ARRAY_BUFFER)),
            );
        }
        if let Some(values) = &mesh.colors {
            attributes.insert(
                "COLOR_0",
                self.u8_accessor(values, "VEC4", true, ARRAY_BUFFER),
            );
        }
        if let Some(influences) = &mesh.bone_influences {
            attributes.insert(
                "JOINTS_0",
                self.u16_accessor(&influences.primary.joints, "VEC4", ARRAY_BUFFER),
            );
            attributes.insert(
                "WEIGHTS_0",
                self.u8_accessor(&influences.primary.weights, "VEC4", true, ARRAY_BUFFER),
            );
            if let Some(secondary) = &influences.secondary {
                attributes.insert(
                    "JOINTS_1",
                    self.u16_accessor(&secondary.joints, "VEC4", ARRAY_BUFFER),
                );
                attributes.insert(
                    "WEIGHTS_1",
                    self.u8_accessor(&secondary.weights, "VEC4", true, ARRAY_BUFFER),
                );
            }
        }
        let primitives = mesh
            .primitives
            .iter()
            .map(|primitive| {
                let index_accessor =
                    self.u32_accessor(&primitive.indices, "SCALAR", ELEMENT_ARRAY_BUFFER);
                let mut value = Map::new();
                value.insert("attributes".into(), json!(attributes));
                value.insert("indices".into(), json!(index_accessor));
                value.insert("mode".into(), json!(4));
                if let Some(material) = primitive.material {
                    value.insert("material".into(), json!(material));
                }
                Value::Object(value)
            })
            .collect::<Vec<_>>();
        json!({"name": mesh.name, "primitives": primitives})
    }

    fn skin(&mut self, skin: &crate::convert::SceneSkin) -> Value {
        let matrices = skin
            .inverse_bind_matrices
            .iter()
            .map(|matrix| matrix.column_major())
            .collect::<Vec<_>>();
        let inverse_bind_matrices = self.f32_accessor(&matrices, "MAT4", None, None, None);
        let mut value = Map::new();
        value.insert("joints".into(), json!(skin.joint_nodes));
        value.insert("inverseBindMatrices".into(), json!(inverse_bind_matrices));
        if skin.root_joint_nodes.len() == 1 {
            value.insert("skeleton".into(), json!(skin.root_joint_nodes[0]));
        }
        Value::Object(value)
    }

    fn f32_accessor<const N: usize>(
        &mut self,
        values: &[[f32; N]],
        kind: &'static str,
        min: Option<[f32; N]>,
        max: Option<[f32; N]>,
        target: Option<u32>,
    ) -> usize {
        let mut bytes = Vec::with_capacity(values.len().saturating_mul(N).saturating_mul(4));
        for value in values {
            for component in value {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        let view = self.buffer_view(&bytes, target);
        let mut accessor = Map::new();
        accessor.insert("bufferView".into(), json!(view));
        accessor.insert("componentType".into(), json!(FLOAT));
        accessor.insert("count".into(), json!(values.len()));
        accessor.insert("type".into(), json!(kind));
        if let Some(min) = min {
            accessor.insert("min".into(), json!(min.as_slice()));
        }
        if let Some(max) = max {
            accessor.insert("max".into(), json!(max.as_slice()));
        }
        self.push_accessor(Value::Object(accessor))
    }

    fn u8_accessor<const N: usize>(
        &mut self,
        values: &[[u8; N]],
        kind: &'static str,
        normalized: bool,
        target: u32,
    ) -> usize {
        let bytes = values.iter().flatten().copied().collect::<Vec<_>>();
        let view = self.buffer_view(&bytes, Some(target));
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": UNSIGNED_BYTE,
            "count": values.len(),
            "normalized": normalized,
            "type": kind,
        }))
    }

    fn u16_accessor<const N: usize>(
        &mut self,
        values: &[[u16; N]],
        kind: &'static str,
        target: u32,
    ) -> usize {
        let mut bytes = Vec::with_capacity(values.len().saturating_mul(N).saturating_mul(2));
        for value in values {
            for component in value {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        let view = self.buffer_view(&bytes, Some(target));
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": UNSIGNED_SHORT,
            "count": values.len(),
            "type": kind,
        }))
    }

    fn u32_accessor(&mut self, values: &[u32], kind: &'static str, target: u32) -> usize {
        let mut bytes = Vec::with_capacity(values.len().saturating_mul(4));
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let view = self.buffer_view(&bytes, Some(target));
        self.push_accessor(json!({
            "bufferView": view,
            "componentType": UNSIGNED_INT,
            "count": values.len(),
            "type": kind,
        }))
    }

    fn buffer_view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        pad_to_four(&mut self.binary, 0);
        let offset = self.binary.len();
        self.binary.extend_from_slice(bytes);
        let mut view = Map::new();
        view.insert("buffer".into(), json!(0));
        view.insert("byteLength".into(), json!(bytes.len()));
        view.insert("byteOffset".into(), json!(offset));
        if let Some(target) = target {
            view.insert("target".into(), json!(target));
        }
        let index = self.buffer_views.len();
        self.buffer_views.push(Value::Object(view));
        index
    }

    fn push_accessor(&mut self, accessor: Value) -> usize {
        let index = self.accessors.len();
        self.accessors.push(accessor);
        index
    }
}

fn bounds(values: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for value in values {
        for index in 0..3 {
            min[index] = min[index].min(value[index]);
            max[index] = max[index].max(value[index]);
        }
    }
    (min, max)
}

fn encode_glb(document: &Value, mut binary: Vec<u8>) -> Result<Vec<u8>, ConversionError> {
    let mut json =
        serde_json::to_vec(document).map_err(|error| ConversionError::Json(error.to_string()))?;
    pad_to_four(&mut json, b' ');
    pad_to_four(&mut binary, 0);
    let total_len = 12_usize
        .checked_add(8)
        .and_then(|value| value.checked_add(json.len()))
        .and_then(|value| value.checked_add(8))
        .and_then(|value| value.checked_add(binary.len()))
        .ok_or(ConversionError::SizeOverflow { context: "GLB" })?;
    let total_len =
        u32::try_from(total_len).map_err(|_| ConversionError::SizeOverflow { context: "GLB" })?;
    let json_len = u32::try_from(json.len()).map_err(|_| ConversionError::SizeOverflow {
        context: "GLB JSON",
    })?;
    let binary_len = u32::try_from(binary.len()).map_err(|_| ConversionError::SizeOverflow {
        context: "GLB binary",
    })?;
    let mut output = Vec::with_capacity(usize::try_from(total_len).unwrap_or(0));
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2_u32.to_le_bytes());
    output.extend_from_slice(&total_len.to_le_bytes());
    output.extend_from_slice(&json_len.to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json);
    output.extend_from_slice(&binary_len.to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(&binary);
    Ok(output)
}

fn pad_to_four(bytes: &mut Vec<u8>, byte: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(byte);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn column_major_matrix_is_stable() {
        let matrix = crate::convert::Mat4([
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [13.0, 14.0, 15.0, 16.0],
        ]);
        let actual = matrix.column_major();
        let expected: [f32; 16] = [
            1.0, 5.0, 9.0, 13.0, 2.0, 6.0, 10.0, 14.0, 3.0, 7.0, 11.0, 15.0, 4.0, 8.0, 12.0, 16.0,
        ];
        assert!(
            actual
                .into_iter()
                .zip(expected)
                .all(|(actual, expected)| actual.to_bits() == expected.to_bits())
        );
    }
}
