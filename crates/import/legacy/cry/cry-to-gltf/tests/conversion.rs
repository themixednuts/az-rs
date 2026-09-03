use cry_chunk::{CHUNK_FILE_VERSION, ChunkFileSignature, CryModel, MeshStreamType};
use cry_to_gltf::{ConversionError, PbrMaterial, to_glb, to_glb_with_materials};

const NODE: u16 = 0x100b;
const MATERIAL_NAME: u16 = 0x1014;
const DATA_STREAM: u16 = 0x1016;
const MESH_SUBSETS: u16 = 0x1017;
const MESH: u16 = 0x1000;
const COMPILED_BONES: u16 = 0x2000;

#[test]
fn writes_a_valid_skinned_glb_with_public_geometry_streams() {
    let bytes = fixture(PositionEncoding::Float, true, BoneEncoding::Global);
    let model = CryModel::parse(&bytes).unwrap();
    let glb = to_glb_with_materials(&model, |material| {
        assert_eq!(material.chunk_id, 9);
        assert_eq!(material.slot, 0);
        assert_eq!(material.name, "neutral");
        PbrMaterial {
            base_color_factor: [0.25, 0.5, 0.75, 1.0],
            metallic_factor: 0.2,
            roughness_factor: 0.8,
        }
    })
    .unwrap();

    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
    assert_eq!(
        usize::try_from(u32::from_le_bytes(glb[8..12].try_into().unwrap())).unwrap(),
        glb.len()
    );
    assert_eq!(&glb[16..20], b"JSON");
    let json_len = usize::try_from(u32::from_le_bytes(glb[12..16].try_into().unwrap())).unwrap();
    let binary_header = 20 + json_len;
    assert_eq!(&glb[binary_header + 4..binary_header + 8], b"BIN\0");
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.as_deref().unwrap();
    assert!(parsed.accessors().all(|accessor| accessor.view().is_some()));
    assert!(parsed.views().all(|view| view.offset() % 4 == 0));

    let mesh = parsed.meshes().next().unwrap();
    let primitive = mesh.primitives().next().unwrap();
    let reader = primitive.reader(|_| Some(blob));
    let positions = reader.read_positions().unwrap().collect::<Vec<_>>();
    let normals = reader.read_normals().unwrap().collect::<Vec<_>>();
    let uvs = reader
        .read_tex_coords(0)
        .unwrap()
        .into_f32()
        .collect::<Vec<_>>();
    let colors = reader
        .read_colors(0)
        .unwrap()
        .into_rgba_u8()
        .collect::<Vec<_>>();
    let indices = reader
        .read_indices()
        .unwrap()
        .into_u32()
        .collect::<Vec<_>>();
    let joints = reader
        .read_joints(0)
        .unwrap()
        .into_u16()
        .collect::<Vec<_>>();
    let weights = reader
        .read_weights(0)
        .unwrap()
        .into_f32()
        .collect::<Vec<_>>();
    assert_eq!(
        positions,
        vec![[0.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
    );
    assert_eq!(normals, vec![[0.0, 1.0, 0.0]; 3]);
    assert_eq!(uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    assert_eq!(
        colors,
        vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
    );
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(joints, vec![[0, 0, 0, 0]; 3]);
    assert!(
        weights
            .iter()
            .all(|weight| weight[0].to_bits() == 1.0_f32.to_bits())
    );

    let root = parsed
        .nodes()
        .find(|node| node.name() == Some("root"))
        .unwrap();
    let (translation, _, _) = root.transform().decomposed();
    assert_floats(translation, [-1.0, 3.0, 2.0]);
    assert_eq!(root.children().next().unwrap().name(), Some("child"));
    assert_eq!(root.mesh().unwrap().index(), 0);
    assert_eq!(root.skin().unwrap().index(), 0);

    let material = primitive.material().pbr_metallic_roughness();
    assert_floats(material.base_color_factor(), [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(material.metallic_factor().to_bits(), 0.2_f32.to_bits());
    assert_eq!(material.roughness_factor().to_bits(), 0.8_f32.to_bits());

    let skin = parsed.skins().next().unwrap();
    let joint = skin.joints().next().unwrap();
    let (joint_translation, _, _) = joint.transform().decomposed();
    assert_floats(joint_translation, [-2.0, 0.0, 0.0]);
    let inverse_bind = skin
        .reader(|_| Some(blob))
        .read_inverse_bind_matrices()
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(inverse_bind[3][0].to_bits(), 2.0_f32.to_bits());
}

#[test]
fn emits_deterministic_bytes_and_accepts_public_half_positions() {
    let bytes = fixture(PositionEncoding::Half, true, BoneEncoding::Global);
    let model = CryModel::parse(&bytes).unwrap();
    let first = to_glb(&model).unwrap();
    let second = to_glb(&model).unwrap();
    assert_eq!(first, second);

    let parsed = gltf::Gltf::from_slice(&first).unwrap();
    let blob = parsed.blob.as_deref().unwrap();
    let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let positions = primitive
        .reader(|_| Some(blob))
        .read_positions()
        .unwrap()
        .collect::<Vec<_>>();
    assert_floats(positions[1], [-1.0, 0.0, 0.0]);
}

#[test]
fn accepts_absent_normals_and_rejects_unsupported_positions_precisely() {
    let missing_bytes = fixture(PositionEncoding::Float, false, BoneEncoding::Global);
    let missing = CryModel::parse(&missing_bytes).unwrap();
    let glb = to_glb(&missing).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.as_deref().unwrap();
    let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.reader(|_| Some(blob)).read_normals().is_none());

    let unsupported_bytes = fixture(PositionEncoding::Unsupported, true, BoneEncoding::Global);
    let unsupported = CryModel::parse(&unsupported_bytes).unwrap();
    assert_eq!(
        to_glb(&unsupported).unwrap_err(),
        ConversionError::UnsupportedElementSize {
            stream_id: 4,
            stream_type: MeshStreamType::Positions,
            element_size: 16,
            supported: &[8, 12],
        }
    );
}

#[test]
fn keeps_empty_mesh_nodes_without_emitting_invalid_gltf_meshes() {
    let bytes = empty_mesh_fixture();
    let model = CryModel::parse(&bytes).unwrap();
    let glb = to_glb(&model).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();

    assert_eq!(parsed.nodes().count(), 1);
    assert_eq!(parsed.meshes().count(), 0);
    assert!(parsed.nodes().next().unwrap().mesh().is_none());
}

#[test]
fn accepts_doubled_bone_streams_but_exposes_secondary_weights_only_when_flagged() {
    for (encoding, expect_secondary) in [
        (BoneEncoding::GlobalDoubledHidden, false),
        (BoneEncoding::GlobalDoubledExposed, true),
    ] {
        let bytes = fixture(PositionEncoding::Float, true, encoding);
        let model = CryModel::parse(&bytes).unwrap();
        let glb = to_glb(&model).unwrap();
        let parsed = gltf::Gltf::from_slice(&glb).unwrap();
        let blob = parsed.blob.as_deref().unwrap();
        let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(
            primitive.reader(|_| Some(blob)).read_joints(1).is_some(),
            expect_secondary
        );
    }
}

#[test]
fn converts_public_subset_local_bone_mappings() {
    let bytes = fixture(PositionEncoding::Float, true, BoneEncoding::SubsetLocal);
    let model = CryModel::parse(&bytes).unwrap();
    let glb = to_glb(&model).unwrap();
    let parsed = gltf::Gltf::from_slice(&glb).unwrap();
    let blob = parsed.blob.as_deref().unwrap();
    let primitive = parsed.meshes().next().unwrap().primitives().next().unwrap();
    let reader = primitive.reader(|_| Some(blob));
    let joints = reader
        .read_joints(0)
        .unwrap()
        .into_u16()
        .collect::<Vec<_>>();
    let weights = reader
        .read_weights(0)
        .unwrap()
        .into_f32()
        .collect::<Vec<_>>();
    assert_eq!(joints, vec![[0, 0, 0, 0]; 3]);
    assert!(
        weights
            .iter()
            .all(|weight| weight[0].to_bits() == 1.0_f32.to_bits())
    );
}

#[test]
fn rejects_subset_local_bone_mapping_outside_the_subset_vertex_window() {
    let bytes = fixture(
        PositionEncoding::Float,
        true,
        BoneEncoding::SubsetLocalOutsideRange,
    );
    let model = CryModel::parse(&bytes).unwrap();
    assert_eq!(
        to_glb(&model).unwrap_err(),
        ConversionError::VertexOutsideSubset {
            mesh_id: 2,
            subset: 0,
            vertex: 2,
            first_vertex: 0,
            vertex_end: 2,
        }
    );
}

#[derive(Clone, Copy)]
enum PositionEncoding {
    Float,
    Half,
    Unsupported,
}

#[derive(Clone, Copy)]
enum BoneEncoding {
    Global,
    GlobalDoubledHidden,
    GlobalDoubledExposed,
    SubsetLocal,
    SubsetLocalOutsideRange,
}

fn fixture(
    position_encoding: PositionEncoding,
    include_normals: bool,
    bone_encoding: BoneEncoding,
) -> Vec<u8> {
    let mut chunks = vec![
        (
            NODE,
            0x0824,
            20,
            node_payload("root", 2, 0, 9, [100.0, 200.0, 300.0]),
        ),
        (
            NODE,
            0x0824,
            12,
            node_payload("child", -1, 20, 0, [0.0, 0.0, 100.0]),
        ),
        (
            MESH,
            0x0801,
            2,
            mesh_payload(include_normals, bone_encoding),
        ),
        (MESH_SUBSETS, 0x0800, 3, subsets_payload(bone_encoding)),
        (
            DATA_STREAM,
            0x0801,
            4,
            stream_payload(
                MeshStreamType::Positions,
                0,
                3,
                position_size(position_encoding),
                &position_data(position_encoding),
            ),
        ),
        (
            DATA_STREAM,
            0x0801,
            6,
            stream_payload(
                MeshStreamType::TextureCoordinates,
                0,
                3,
                8,
                &f32_data(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            ),
        ),
        (
            DATA_STREAM,
            0x0801,
            7,
            stream_payload(
                MeshStreamType::Colors,
                0,
                3,
                4,
                &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255],
            ),
        ),
        (
            DATA_STREAM,
            0x0801,
            8,
            stream_payload(MeshStreamType::Indices, 0, 3, 2, &u16_data(&[0, 1, 2])),
        ),
        (MATERIAL_NAME, 0x0802, 9, material_payload("neutral")),
        (
            DATA_STREAM,
            0x0801,
            10,
            stream_payload(
                MeshStreamType::BoneMapping,
                0,
                bone_mapping_count(bone_encoding),
                bone_element_size(bone_encoding),
                &bone_mapping_data(bone_encoding),
            ),
        ),
        (COMPILED_BONES, 0x0800, 11, bones_payload()),
    ];
    if include_normals {
        chunks.push((
            DATA_STREAM,
            0x0801,
            5,
            stream_payload(
                MeshStreamType::Normals,
                0,
                3,
                12,
                &f32_data(&[[0.0, 0.0, 1.0]; 3]),
            ),
        ));
    }
    chunk_file(&chunks)
}

fn node_payload(
    name: &str,
    object_id: i32,
    parent_id: i32,
    material_id: i32,
    translation: [f32; 3],
) -> Vec<u8> {
    let mut payload = vec![0; 204];
    put_string(&mut payload[..64], name);
    put_i32(&mut payload, 64, object_id);
    put_i32(&mut payload, 68, parent_id);
    put_i32(&mut payload, 76, material_id);
    for index in 0..4 {
        put_f32(&mut payload, 84 + (index * 5) * 4, 1.0);
    }
    for (index, value) in translation.into_iter().enumerate() {
        put_f32(&mut payload, 84 + (12 + index) * 4, value);
    }
    payload
}

fn mesh_payload(include_normals: bool, bone_encoding: BoneEncoding) -> Vec<u8> {
    let mut payload = vec![0; 264];
    if matches!(bone_encoding, BoneEncoding::GlobalDoubledExposed) {
        put_i32(&mut payload, 0, 0x4);
    }
    put_i32(&mut payload, 8, 3);
    put_i32(&mut payload, 12, 3);
    put_i32(&mut payload, 16, 1);
    put_i32(&mut payload, 20, 3);
    put_i32(&mut payload, 28, 4);
    if include_normals {
        put_i32(&mut payload, 32, 5);
    }
    put_i32(&mut payload, 36, 6);
    put_i32(&mut payload, 40, 7);
    put_i32(&mut payload, 48, 8);
    put_i32(&mut payload, 64, 10);
    payload
}

fn subsets_payload(bone_encoding: BoneEncoding) -> Vec<u8> {
    let extra = match bone_encoding {
        BoneEncoding::Global
        | BoneEncoding::GlobalDoubledHidden
        | BoneEncoding::GlobalDoubledExposed => 0,
        BoneEncoding::SubsetLocal | BoneEncoding::SubsetLocalOutsideRange => 260,
    };
    let mut payload = vec![0; 52 + extra];
    if matches!(
        bone_encoding,
        BoneEncoding::SubsetLocal | BoneEncoding::SubsetLocalOutsideRange
    ) {
        put_i32(&mut payload, 0, 0x2);
    }
    put_i32(&mut payload, 4, 1);
    put_i32(&mut payload, 20, 3);
    put_i32(
        &mut payload,
        28,
        if matches!(bone_encoding, BoneEncoding::SubsetLocalOutsideRange) {
            2
        } else {
            3
        },
    );
    if matches!(
        bone_encoding,
        BoneEncoding::SubsetLocal | BoneEncoding::SubsetLocalOutsideRange
    ) {
        put_u32(&mut payload, 52, 1);
    }
    payload
}

fn stream_payload(
    stream_type: MeshStreamType,
    stream_index: i32,
    count: i32,
    element_size: i32,
    data: &[u8],
) -> Vec<u8> {
    let mut payload = vec![0; 28];
    put_i32(&mut payload, 4, stream_type as i32);
    put_i32(&mut payload, 8, stream_index);
    put_i32(&mut payload, 12, count);
    put_i32(&mut payload, 16, element_size);
    payload.extend_from_slice(data);
    payload
}

fn material_payload(name: &str) -> Vec<u8> {
    let mut payload = vec![0; 132];
    put_string(&mut payload[..128], name);
    payload.extend_from_slice(&(-1_i32).to_le_bytes());
    payload
}

fn bones_payload() -> Vec<u8> {
    let mut payload = vec![0; 32 + 584];
    let base = 32;
    put_u32(&mut payload, base, 1);
    matrix34(&mut payload, base + 216, [-2.0, 0.0, 0.0]);
    matrix34(&mut payload, base + 264, [2.0, 0.0, 0.0]);
    put_string(&mut payload[base + 312..base + 568], "joint");
    payload
}

fn matrix34(payload: &mut [u8], offset: usize, translation: [f32; 3]) {
    for (index, value) in translation.into_iter().enumerate() {
        put_f32(payload, offset + (index * 5) * 4, 1.0);
        put_f32(payload, offset + (index * 4 + 3) * 4, value);
    }
}

const fn bone_element_size(encoding: BoneEncoding) -> i32 {
    match encoding {
        BoneEncoding::Global
        | BoneEncoding::GlobalDoubledHidden
        | BoneEncoding::GlobalDoubledExposed => 12,
        BoneEncoding::SubsetLocal | BoneEncoding::SubsetLocalOutsideRange => 8,
    }
}

fn bone_mapping_data(encoding: BoneEncoding) -> Vec<u8> {
    let stride = usize::try_from(bone_element_size(encoding)).unwrap();
    let count = usize::try_from(bone_mapping_count(encoding)).unwrap();
    let mut bytes = vec![0; count * stride];
    for vertex in 0..count {
        bytes[vertex * stride + stride - 4] = 255;
    }
    bytes
}

const fn bone_mapping_count(encoding: BoneEncoding) -> i32 {
    match encoding {
        BoneEncoding::GlobalDoubledHidden | BoneEncoding::GlobalDoubledExposed => 6,
        BoneEncoding::Global
        | BoneEncoding::SubsetLocal
        | BoneEncoding::SubsetLocalOutsideRange => 3,
    }
}

fn empty_mesh_fixture() -> Vec<u8> {
    let mut mesh = vec![0; 264];
    put_i32(&mut mesh, 0, 0x1);
    chunk_file(&[
        (NODE, 0x0824, 20, node_payload("empty", 2, 0, 0, [0.0; 3])),
        (MESH, 0x0801, 2, mesh),
    ])
}

const fn position_size(encoding: PositionEncoding) -> i32 {
    match encoding {
        PositionEncoding::Float => 12,
        PositionEncoding::Half => 8,
        PositionEncoding::Unsupported => 16,
    }
}

fn position_data(encoding: PositionEncoding) -> Vec<u8> {
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    match encoding {
        PositionEncoding::Float => f32_data(&positions),
        PositionEncoding::Half => {
            let mut bytes = Vec::new();
            for position in positions {
                for value in position {
                    bytes.extend_from_slice(&half::f16::from_f32(value).to_bits().to_le_bytes());
                }
                bytes.extend_from_slice(&half::f16::from_f32(1.0).to_bits().to_le_bytes());
            }
            bytes
        }
        PositionEncoding::Unsupported => vec![0; 48],
    }
}

fn f32_data<const N: usize>(values: &[[f32; N]]) -> Vec<u8> {
    values
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn u16_data(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn chunk_file(chunks: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&ChunkFileSignature::Cry.bytes());
    bytes.extend_from_slice(&CHUNK_FILE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(chunks.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    let mut offset = 16 + chunks.len() * 16;
    for (kind, version, id, payload) in chunks {
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        offset += payload.len();
    }
    for (_, _, _, payload) in chunks {
        bytes.extend_from_slice(payload);
    }
    bytes
}

fn put_string(bytes: &mut [u8], value: &str) {
    bytes[..value.len()].copy_from_slice(value.as_bytes());
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
    put_u32(bytes, offset, value.to_bits());
}

fn assert_floats<const N: usize>(actual: [f32; N], expected: [f32; N]) {
    assert!(
        actual
            .into_iter()
            .zip(expected)
            .all(|(actual, expected)| actual.to_bits() == expected.to_bits())
    );
}
