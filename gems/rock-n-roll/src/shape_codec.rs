use std::io::{Cursor, Read, Write};

use bevy::math::{Vec3, Vec4};

use crate::{
    BoxShape, BvhTree, BvhTreeParts, CapsuleShape, CapsuleUnalignedShape, CompoundChild,
    CompoundShape, ConvexHullExtra, ConvexHullShape, CylinderShape, CylinderUnalignedShape,
    HeightFieldData, HeightFieldShape, MaterialFilter, MeshShape, PhysicalShape, PlaneShape,
    SHAPE_ASSET_VERSION, ScaledShape, ShapeAsset, ShapeAssetFormatError, ShapeData, ShapeKind,
    ShapeObject, ShapeTransform, SoftBodyShape, SphereShape, TransformShape, TriangleShape,
};

const MAGIC: &[u8; 8] = b"AZRNRSH\0";
const MAX_RECURSION_DEPTH: usize = 64;

pub fn write_shape_asset(
    asset: &ShapeAsset,
    mut writer: impl Write,
) -> Result<(), ShapeAssetFormatError> {
    writer.write_all(MAGIC)?;
    write_u32(&mut writer, SHAPE_ASSET_VERSION)?;
    write_objects(&mut writer, &asset.objects)?;
    write_material_filter(&mut writer, &asset.material_filter)?;
    write_shapes(&mut writer, &asset.shapes)?;
    Ok(())
}

pub fn read_shape_asset(bytes: &[u8]) -> Result<ShapeAsset, ShapeAssetFormatError> {
    read_shape_asset_from_reader(Cursor::new(bytes))
}

pub fn read_shape_asset_from_reader(
    mut reader: impl Read,
) -> Result<ShapeAsset, ShapeAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(ShapeAssetFormatError::BadMagic { found: magic });
    }
    read_after_magic(&mut reader)
}

fn read_after_magic(reader: &mut impl Read) -> Result<ShapeAsset, ShapeAssetFormatError> {
    let version = read_u32(reader)?;
    if version != SHAPE_ASSET_VERSION {
        return Err(ShapeAssetFormatError::UnsupportedVersion {
            version,
            expected: SHAPE_ASSET_VERSION,
        });
    }
    Ok(ShapeAsset {
        version,
        objects: read_objects(reader)?,
        material_filter: read_material_filter(reader)?,
        shapes: read_shapes(reader)?,
    })
}

fn write_objects(
    writer: &mut impl Write,
    objects: &[ShapeObject],
) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(objects.len(), "shape objects")?)?;
    for object in objects {
        write_string(writer, &object.name)?;
        write_u16s(writer, &object.material_indices)?;
    }
    Ok(())
}

fn read_objects(reader: &mut impl Read) -> Result<Box<[ShapeObject]>, ShapeAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut objects = Vec::with_capacity(count);
    for _ in 0..count {
        objects.push(ShapeObject {
            name: read_string(reader)?,
            material_indices: read_u16s(reader)?,
        });
    }
    Ok(objects.into_boxed_slice())
}

fn write_material_filter(
    writer: &mut impl Write,
    filter: &MaterialFilter,
) -> Result<(), ShapeAssetFormatError> {
    write_bool(writer, filter.enabled)?;
    write_bool(writer, filter.secondary_geometry)?;
    write_u16s(writer, &filter.indices)
}

fn read_material_filter(reader: &mut impl Read) -> Result<MaterialFilter, ShapeAssetFormatError> {
    Ok(MaterialFilter {
        enabled: read_bool(reader)?,
        secondary_geometry: read_bool(reader)?,
        indices: read_u16s(reader)?,
    })
}

fn write_shapes(
    writer: &mut impl Write,
    shapes: &[PhysicalShape],
) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(shapes.len(), "physical shapes")?)?;
    for shape in shapes {
        write_shape(writer, shape)?;
    }
    Ok(())
}

fn read_shapes(reader: &mut impl Read) -> Result<Box<[PhysicalShape]>, ShapeAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut shapes = Vec::with_capacity(count);
    for _ in 0..count {
        shapes.push(read_shape(reader, 0)?);
    }
    Ok(shapes.into_boxed_slice())
}

fn write_shape(
    writer: &mut impl Write,
    shape: &PhysicalShape,
) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, shape.kind().into())?;
    match &shape.data {
        ShapeData::Box(value) => {
            write_vec3(writer, value.half_extents)?;
            write_f32(writer, value.convex_radius)?;
        }
        ShapeData::Sphere(value) => write_f32(writer, value.radius)?,
        ShapeData::ConvexHull(value) => {
            write_vec3s(writer, &value.vertices)?;
            write_vec4s(writer, &value.planes)?;
            write_f32(writer, value.convex_radius)?;
            write_bool(writer, value.extra.is_some())?;
            if let Some(extra) = &value.extra {
                write_u16s(writer, &extra.data_a)?;
                write_u16s(writer, &extra.data_b)?;
            }
        }
        ShapeData::Cylinder(value) => {
            write_f32(writer, value.half_height)?;
            write_f32(writer, value.radius)?;
            write_f32(writer, value.convex_radius)?;
        }
        ShapeData::CylinderUnaligned(value) => {
            write_vec3(writer, value.endpoint_a)?;
            write_vec3(writer, value.endpoint_b)?;
            write_f32(writer, value.radius)?;
            write_f32(writer, value.convex_radius)?;
        }
        ShapeData::Capsule(value) => {
            write_f32(writer, value.half_height)?;
            write_f32(writer, value.radius)?;
        }
        ShapeData::CapsuleUnaligned(value) => {
            write_vec3(writer, value.endpoint_a)?;
            write_vec3(writer, value.endpoint_b)?;
            write_f32(writer, value.radius)?;
        }
        ShapeData::Triangle(value) => {
            write_vec3(writer, value.a)?;
            write_vec3(writer, value.b)?;
            write_vec3(writer, value.c)?;
            write_f32(writer, value.convex_radius)?;
        }
        ShapeData::Mesh(value) => write_mesh_shape(writer, value)?,
        ShapeData::Compound(value) => {
            write_u32(
                writer,
                checked_u32(value.children.len(), "compound children")?,
            )?;
            for child in &value.children {
                write_transform(writer, &child.transform)?;
                write_shape(writer, &child.shape)?;
            }
        }
        ShapeData::Transform(value) => {
            write_transform(writer, &value.transform)?;
            write_shape(writer, &value.shape)?;
        }
        ShapeData::SoftBody(_) => {}
        ShapeData::Plane(value) => {
            write_vec4(writer, value.plane)?;
            write_vec3(writer, value.aabb_min)?;
            write_vec3(writer, value.aabb_max)?;
        }
        ShapeData::ScaleConvexPolytope(value) | ShapeData::ScaleMesh(value) => {
            write_u32(writer, value.stream_header)?;
            write_vec3(writer, value.scale)?;
            write_shape(writer, &value.shape)?;
        }
        ShapeData::HeightField(value) => write_height_field_shape(writer, value)?,
    }
    write_bytes(writer, &shape.extra)
}

fn read_shape(
    reader: &mut impl Read,
    depth: usize,
) -> Result<PhysicalShape, ShapeAssetFormatError> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(ShapeAssetFormatError::InvalidData(
            "shape recursion limit exceeded",
        ));
    }
    let kind_value = read_u32(reader)?;
    let kind = ShapeKind::try_from(kind_value)
        .map_err(|kind| ShapeAssetFormatError::UnknownShapeKind { kind })?;
    let data = read_shape_data(reader, kind, depth)?;
    let extra = read_bytes(reader)?;
    Ok(PhysicalShape { data, extra })
}

fn read_shape_data(
    reader: &mut impl Read,
    kind: ShapeKind,
    depth: usize,
) -> Result<ShapeData, ShapeAssetFormatError> {
    Ok(match kind {
        ShapeKind::Box => ShapeData::Box(BoxShape {
            half_extents: read_vec3(reader)?,
            convex_radius: read_f32(reader)?,
        }),
        ShapeKind::Sphere => ShapeData::Sphere(SphereShape {
            radius: read_f32(reader)?,
        }),
        ShapeKind::ConvexHull => {
            let vertices = read_vec3s(reader)?;
            let planes = read_vec4s(reader)?;
            let convex_radius = read_f32(reader)?;
            let extra = read_bool(reader)?
                .then(|| {
                    Ok::<_, ShapeAssetFormatError>(ConvexHullExtra {
                        data_a: read_u16s(reader)?,
                        data_b: read_u16s(reader)?,
                    })
                })
                .transpose()?;
            ShapeData::ConvexHull(ConvexHullShape {
                vertices,
                planes,
                convex_radius,
                extra,
            })
        }
        ShapeKind::Cylinder => ShapeData::Cylinder(CylinderShape {
            half_height: read_f32(reader)?,
            radius: read_f32(reader)?,
            convex_radius: read_f32(reader)?,
        }),
        ShapeKind::CylinderUnaligned => ShapeData::CylinderUnaligned(CylinderUnalignedShape {
            endpoint_a: read_vec3(reader)?,
            endpoint_b: read_vec3(reader)?,
            radius: read_f32(reader)?,
            convex_radius: read_f32(reader)?,
        }),
        ShapeKind::Capsule => ShapeData::Capsule(CapsuleShape {
            half_height: read_f32(reader)?,
            radius: read_f32(reader)?,
        }),
        ShapeKind::CapsuleUnaligned => ShapeData::CapsuleUnaligned(CapsuleUnalignedShape {
            endpoint_a: read_vec3(reader)?,
            endpoint_b: read_vec3(reader)?,
            radius: read_f32(reader)?,
        }),
        ShapeKind::Triangle => ShapeData::Triangle(TriangleShape {
            a: read_vec3(reader)?,
            b: read_vec3(reader)?,
            c: read_vec3(reader)?,
            convex_radius: read_f32(reader)?,
        }),
        ShapeKind::Mesh => ShapeData::Mesh(read_mesh_shape(reader)?),
        ShapeKind::Compound => {
            let count = read_u32(reader)? as usize;
            let mut children = Vec::with_capacity(count);
            for _ in 0..count {
                children.push(CompoundChild {
                    transform: read_transform(reader)?,
                    shape: Box::new(read_shape(reader, depth + 1)?),
                });
            }
            ShapeData::Compound(CompoundShape {
                children: children.into_boxed_slice(),
            })
        }
        ShapeKind::Transform => ShapeData::Transform(TransformShape {
            transform: read_transform(reader)?,
            shape: Box::new(read_shape(reader, depth + 1)?),
        }),
        ShapeKind::SoftBody => ShapeData::SoftBody(SoftBodyShape),
        ShapeKind::Plane => ShapeData::Plane(PlaneShape {
            plane: read_vec4(reader)?,
            aabb_min: read_vec3(reader)?,
            aabb_max: read_vec3(reader)?,
        }),
        ShapeKind::ScaleConvexPolytope => ShapeData::ScaleConvexPolytope(ScaledShape {
            stream_header: read_u32(reader)?,
            scale: read_vec3(reader)?,
            shape: Box::new(read_shape(reader, depth + 1)?),
        }),
        ShapeKind::ScaleMesh => ShapeData::ScaleMesh(ScaledShape {
            stream_header: read_u32(reader)?,
            scale: read_vec3(reader)?,
            shape: Box::new(read_shape(reader, depth + 1)?),
        }),
        ShapeKind::HeightField => ShapeData::HeightField(read_height_field_shape(reader)?),
    })
}

fn write_mesh_shape(
    writer: &mut impl Write,
    value: &MeshShape,
) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, value.stream_header)?;
    write_vec3s(writer, &value.vertices)?;
    write_u16s(writer, &value.indices)?;
    write_bool(writer, value.adjacent_triangles.is_some())?;
    if let Some(adjacent_triangles) = &value.adjacent_triangles {
        write_u16s(writer, adjacent_triangles)?;
    }
    write_bvh_tree(writer, &value.bvh)
}

fn read_mesh_shape(reader: &mut impl Read) -> Result<MeshShape, ShapeAssetFormatError> {
    let stream_header = read_u32(reader)?;
    let vertices = read_vec3s(reader)?;
    let indices = read_u16s(reader)?;
    let adjacent_triangles = if read_bool(reader)? {
        Some(read_u16s(reader)?)
    } else {
        None
    };
    let bvh = read_bvh_tree(reader)?;
    Ok(MeshShape {
        stream_header,
        vertices,
        indices,
        adjacent_triangles,
        bvh,
    })
}

fn write_height_field_shape(
    writer: &mut impl Write,
    value: &HeightFieldShape,
) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, value.layout)?;
    write_bool(writer, value.data.is_some())?;
    if let Some(data) = &value.data {
        write_u32(writer, data.version)?;
        write_u32(writer, data.width)?;
        write_u32(writer, data.length)?;
        write_f32(writer, data.height_scale)?;
        write_vec3(writer, data.aabb_min)?;
        write_vec3(writer, data.aabb_max)?;
        write_bytes(writer, &data.samples)?;
    }
    Ok(())
}

fn read_height_field_shape(
    reader: &mut impl Read,
) -> Result<HeightFieldShape, ShapeAssetFormatError> {
    let layout = read_u32(reader)?;
    let data = if read_bool(reader)? {
        Some(HeightFieldData {
            version: read_u32(reader)?,
            width: read_u32(reader)?,
            length: read_u32(reader)?,
            height_scale: read_f32(reader)?,
            aabb_min: read_vec3(reader)?,
            aabb_max: read_vec3(reader)?,
            samples: read_bytes(reader)?,
        })
    } else {
        None
    };
    Ok(HeightFieldShape { layout, data })
}

fn write_bvh_tree(writer: &mut impl Write, value: &BvhTree) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, value.version)?;
    write_u32(writer, value.parts.quantized_nodes_offset)?;
    write_u32(writer, value.parts.subtree_infos_offset)?;
    write_u32(writer, value.parts.triangle_index_map_offset)?;
    write_u32(writer, value.parts.quantized_node_count)?;
    write_u16(writer, value.parts.subtree_info_count)?;
    write_u32(writer, value.parts.triangle_index_count)?;
    write_u16(writer, value.parts.flags)?;
    write_bytes(writer, &value.payload)
}

fn read_bvh_tree(reader: &mut impl Read) -> Result<BvhTree, ShapeAssetFormatError> {
    Ok(BvhTree {
        version: read_u32(reader)?,
        parts: BvhTreeParts {
            quantized_nodes_offset: read_u32(reader)?,
            subtree_infos_offset: read_u32(reader)?,
            triangle_index_map_offset: read_u32(reader)?,
            quantized_node_count: read_u32(reader)?,
            subtree_info_count: read_u16(reader)?,
            triangle_index_count: read_u32(reader)?,
            flags: read_u16(reader)?,
        },
        payload: read_bytes(reader)?,
    })
}

fn write_transform(
    writer: &mut impl Write,
    transform: &ShapeTransform,
) -> Result<(), ShapeAssetFormatError> {
    for row in transform {
        write_vec4(writer, *row)?;
    }
    Ok(())
}

fn read_transform(reader: &mut impl Read) -> Result<ShapeTransform, ShapeAssetFormatError> {
    Ok([read_vec4(reader)?, read_vec4(reader)?, read_vec4(reader)?])
}

fn write_vec3s(writer: &mut impl Write, values: &[Vec3]) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "Vec3 values")?)?;
    for value in values {
        write_vec3(writer, *value)?;
    }
    Ok(())
}

fn read_vec3s(reader: &mut impl Read) -> Result<Box<[Vec3]>, ShapeAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut bytes = vec![0; byte_len(count, 12, "Vec3 bytes")?];
    reader.read_exact(&mut bytes)?;
    Ok(bytes.chunks_exact(12).map(vec3_from_chunk).collect())
}

fn write_vec4s(writer: &mut impl Write, values: &[Vec4]) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "Vec4 values")?)?;
    for value in values {
        write_vec4(writer, *value)?;
    }
    Ok(())
}

fn read_vec4s(reader: &mut impl Read) -> Result<Box<[Vec4]>, ShapeAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut bytes = vec![0; byte_len(count, 16, "Vec4 bytes")?];
    reader.read_exact(&mut bytes)?;
    Ok(bytes.chunks_exact(16).map(vec4_from_chunk).collect())
}

fn write_u16s(writer: &mut impl Write, values: &[u16]) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "u16 values")?)?;
    for value in values {
        write_u16(writer, *value)?;
    }
    Ok(())
}

fn read_u16s(reader: &mut impl Read) -> Result<Box<[u16]>, ShapeAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut bytes = vec![0; byte_len(count, 2, "u16 bytes")?];
    reader.read_exact(&mut bytes)?;
    Ok(bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect())
}

fn write_bytes(writer: &mut impl Write, bytes: &[u8]) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(bytes.len(), "byte block")?)?;
    writer.write_all(bytes)?;
    Ok(())
}

fn read_bytes(reader: &mut impl Read) -> Result<Box<[u8]>, ShapeAssetFormatError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes.into_boxed_slice())
}

fn write_string(writer: &mut impl Write, value: &str) -> Result<(), ShapeAssetFormatError> {
    write_u32(writer, checked_u32(value.len(), "string bytes")?)?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String, ShapeAssetFormatError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

fn write_bool(writer: &mut impl Write, value: bool) -> Result<(), std::io::Error> {
    writer.write_all(&[u8::from(value)])
}

fn read_bool(reader: &mut impl Read) -> Result<bool, std::io::Error> {
    let mut bytes = [0];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0] != 0)
}

fn write_vec3(writer: &mut impl Write, value: Vec3) -> Result<(), std::io::Error> {
    write_f32(writer, value.x)?;
    write_f32(writer, value.y)?;
    write_f32(writer, value.z)
}

fn read_vec3(reader: &mut impl Read) -> Result<Vec3, std::io::Error> {
    Ok(Vec3::new(
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
    ))
}

fn write_vec4(writer: &mut impl Write, value: Vec4) -> Result<(), std::io::Error> {
    for value in value.to_array() {
        write_f32(writer, value)?;
    }
    Ok(())
}

fn read_vec4(reader: &mut impl Read) -> Result<Vec4, std::io::Error> {
    Ok(Vec4::new(
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
    ))
}

fn write_u16(writer: &mut impl Write, value: u16) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u16(reader: &mut impl Read) -> Result<u16, std::io::Error> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_f32(writer: &mut impl Write, value: f32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_f32(reader: &mut impl Read) -> Result<f32, std::io::Error> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

fn vec3_from_chunk(bytes: &[u8]) -> Vec3 {
    Vec3::new(
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
    )
}

fn vec4_from_chunk(bytes: &[u8]) -> Vec4 {
    Vec4::new(
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        f32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
    )
}

fn checked_u32(count: usize, what: &'static str) -> Result<u32, ShapeAssetFormatError> {
    u32::try_from(count).map_err(|_| ShapeAssetFormatError::TooManyItems { what, count })
}

fn byte_len(
    count: usize,
    stride: usize,
    what: &'static str,
) -> Result<usize, ShapeAssetFormatError> {
    count
        .checked_mul(stride)
        .ok_or(ShapeAssetFormatError::InvalidData(what))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_asset_round_trips_binary_format() {
        let asset = ShapeAsset::new(
            vec![ShapeObject {
                name: "primitive".to_string(),
                material_indices: vec![0, 3].into_boxed_slice(),
            }]
            .into_boxed_slice(),
            MaterialFilter {
                enabled: true,
                secondary_geometry: false,
                indices: vec![3].into_boxed_slice(),
            },
            vec![PhysicalShape::new(
                ShapeData::Box(BoxShape {
                    half_extents: Vec3::new(1.0, 2.0, 3.0),
                    convex_radius: 0.25,
                }),
                Vec::new().into_boxed_slice(),
            )]
            .into_boxed_slice(),
        );

        let mut bytes = Vec::new();
        write_shape_asset(&asset, &mut bytes).unwrap();
        let decoded = read_shape_asset(&bytes).unwrap();

        assert_eq!(decoded, asset);
    }
}
