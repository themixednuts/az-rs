use bevy::color::{LinearRgba, Srgba};
use bevy::prelude::*;

use super::*;

#[test]
fn material_asset_type_ids_match_lumberyard() {
    assert_eq!(
        MATERIAL_ASSET_TYPE_ID,
        uuid::uuid!("F46985B5-F7FF-4FCB-8E8C-DC240D701841")
    );
    assert_eq!(
        SIMPLE_MATERIAL_ASSET_REFERENCE_TYPE_ID,
        uuid::uuid!("B7B8ECC7-FF89-4A76-A50E-4C6CA2B6E6B4")
    );
    assert_eq!(
        MATERIAL_OVERRIDE_ASSET_TYPE_ID,
        uuid::uuid!("5A8C903D-69F3-4259-8E31-9CB04867BD6E")
    );
    assert_eq!(
        SIMPLE_MATERIAL_OVERRIDE_ASSET_REFERENCE_TYPE_ID,
        uuid::uuid!("19ED7B41-FB10-4DF5-A7FD-697182186D7F")
    );
    assert!(MATERIAL_ASSET_FILE_FILTERS.contains(&"mtl"));
    assert!(TEXTURE_ASSET_FILE_FILTERS.contains(&"dds"));
}

#[test]
fn material_texture_map_preserves_native_names() {
    assert_eq!(
        MaterialTextureMap::from_native_name("[5] Smoothness"),
        MaterialTextureMap::NumberedSmoothness(5)
    );
    assert_eq!(
        MaterialTextureMap::NumberedCustom(3).native_name().as_ref(),
        "[3] Custom"
    );
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn material_definition_maps_to_bevy_standard_material() {
    let material = MaterialDefinition {
        diffuse: Some(Srgba::new(0.2, 0.4, 0.6, 1.0)),
        emittance: Some(LinearRgba::from(Srgba::new(1.0, 0.5, 0.25, 1.0)) * 2.0),
        opacity: 0.5,
        shininess: 127.5,
        ..Default::default()
    };

    let standard = material.standard_material();

    assert_eq!(standard.base_color.to_srgba().alpha, 0.5);
    assert_eq!(standard.alpha_mode, AlphaMode::Blend);
    assert_eq!(
        standard.emissive,
        LinearRgba::from(Srgba::new(1.0, 0.5, 0.25, 1.0)) * 2.0
    );
    assert_eq!(standard.perceptual_roughness, 0.5);
}

#[test]
fn material_asset_round_trips_binary() {
    let asset = MaterialAsset {
        source_path: "materials/road/defaultroad.mtl".to_string(),
        root: MaterialDefinition {
            name: Some("defaultroad".to_string()),
            shader: Some("Illum".to_string()),
            textures: vec![MaterialTextureReference {
                map: MaterialTextureMap::Diffuse,
                image_asset_path: Some("textures/road/defaultroad.dds".to_string()),
                asset_id: None,
                filter: Some(MaterialTextureFilter::Trilinear),
                is_tile_u: true,
                is_tile_v: true,
                texture_type: Some(MaterialTextureType::TwoDimensional),
                texture_modifier: Vec::new(),
            }],
            ..Default::default()
        },
        sub_materials: Vec::new(),
    };

    let mut bytes = Vec::new();
    write_material_asset(&asset, &mut bytes).unwrap();
    let parsed = read_material_asset(&bytes).unwrap();

    assert_eq!(parsed, asset);
    assert_eq!(
        parsed.root.image_asset_path(&MaterialTextureMap::Diffuse),
        Some("textures/road/defaultroad.dds")
    );
}

#[test]
fn material_override_asset_round_trips_binary() {
    let asset = MaterialOverrideAsset {
        source_path: "libs/materialoverrides/example/rim.xml".to_string(),
        max_trigger_distance: Some("close".to_string()),
        materials: vec![MaterialOverrideTarget {
            name: Some("All".to_string()),
            exclude: Some("materials/vfx/_test/occlusion/occlusion_xray".to_string()),
            sub_materials: vec![MaterialOverrideSubTarget {
                name: Some("All".to_string()),
                shader_generation_params: vec![MaterialOverrideSwitch {
                    name: "Rim_Diffuse_Lighting".to_string(),
                    enabled: true,
                    extra_attributes: Vec::new(),
                }],
                texture_maps: vec![MaterialOverrideParamBlock {
                    name: "CustomSecondaryMap".to_string(),
                    params: vec![MaterialOverrideParam {
                        name: "start".to_string(),
                        value_kind: MaterialOverrideValueKind::String,
                        value: "textures/defaults/noise_03_diff.dds".to_string(),
                        extra_attributes: Vec::new(),
                    }],
                    extra_attributes: Vec::new(),
                }],
                shader_params: vec![MaterialOverrideParamBlock {
                    name: "Rim_Fill_Intensity".to_string(),
                    params: vec![MaterialOverrideParam {
                        name: "start".to_string(),
                        value_kind: MaterialOverrideValueKind::Float,
                        value: "0.7".to_string(),
                        extra_attributes: Vec::new(),
                    }],
                    extra_attributes: Vec::new(),
                }],
                extra_attributes: Vec::new(),
            }],
            extra_attributes: Vec::new(),
        }],
        extra_attributes: Vec::new(),
    };

    let mut bytes = Vec::new();
    write_material_override_asset(&asset, &mut bytes).unwrap();
    let parsed = read_material_override_asset(&bytes).unwrap();

    assert_eq!(parsed, asset);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn material_override_asset_maps_to_standard_material() {
    let asset = MaterialOverrideAsset {
        materials: vec![MaterialOverrideTarget {
            sub_materials: vec![MaterialOverrideSubTarget {
                texture_maps: vec![MaterialOverrideParamBlock {
                    name: "CustomSecondaryMap".to_string(),
                    params: vec![MaterialOverrideParam {
                        name: "start".to_string(),
                        value_kind: MaterialOverrideValueKind::String,
                        value: "textures/defaults/noise_03_diff.dds".to_string(),
                        extra_attributes: Vec::new(),
                    }],
                    extra_attributes: Vec::new(),
                }],
                shader_params: vec![MaterialOverrideParamBlock {
                    name: "Opacity".to_string(),
                    params: vec![MaterialOverrideParam {
                        name: "start".to_string(),
                        value_kind: MaterialOverrideValueKind::Float,
                        value: "0.4".to_string(),
                        extra_attributes: Vec::new(),
                    }],
                    extra_attributes: Vec::new(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut loaded_paths = Vec::new();

    let material = asset.standard_material_with_images(|path| {
        loaded_paths.push(path.to_string());
        Handle::<Image>::default()
    });

    assert!(material.base_color_texture.is_some());
    assert_eq!(material.base_color.to_srgba().alpha, 0.4);
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
    assert_eq!(loaded_paths, ["textures/defaults/noise_03_diff.dds"]);
}

#[test]
fn material_definition_maps_image_slots_to_standard_material() {
    let material = MaterialDefinition {
        textures: vec![
            MaterialTextureReference {
                map: MaterialTextureMap::Diffuse,
                image_asset_path: Some("textures/road/defaultroad.dds".to_string()),
                asset_id: None,
                filter: None,
                is_tile_u: true,
                is_tile_v: true,
                texture_type: Some(MaterialTextureType::TwoDimensional),
                texture_modifier: Vec::new(),
            },
            MaterialTextureReference {
                map: MaterialTextureMap::Bumpmap,
                image_asset_path: Some("textures/road/defaultroad_ddna.dds".to_string()),
                asset_id: None,
                filter: None,
                is_tile_u: true,
                is_tile_v: true,
                texture_type: Some(MaterialTextureType::TwoDimensional),
                texture_modifier: Vec::new(),
            },
        ],
        ..Default::default()
    };

    let mut loaded_paths = Vec::new();
    let standard = material.standard_material_with_images(|path| {
        loaded_paths.push(path.to_string());
        Handle::<Image>::default()
    });

    assert!(standard.base_color_texture.is_some());
    assert!(standard.normal_map_texture.is_some());
    assert_eq!(
        loaded_paths,
        [
            "textures/road/defaultroad.dds".to_string(),
            "textures/road/defaultroad_ddna.dds".to_string(),
        ]
    );
}

#[test]
fn material_asset_uses_valid_sub_material_slots() {
    let asset = MaterialAsset {
        root: MaterialDefinition {
            diffuse: Some(Srgba::new(1.0, 1.0, 1.0, 1.0)),
            ..Default::default()
        },
        sub_materials: vec![
            MaterialDefinition {
                diffuse: Some(Srgba::new(1.0, 0.0, 0.0, 1.0)),
                ..Default::default()
            },
            MaterialDefinition {
                diffuse: Some(Srgba::new(0.0, 1.0, 0.0, 1.0)),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    assert_eq!(
        asset.material_for_slot(Some(1)).diffuse,
        Some(Srgba::new(0.0, 1.0, 0.0, 1.0))
    );
    assert_eq!(
        asset.material_for_slot(Some(3)).diffuse,
        Some(Srgba::new(1.0, 1.0, 1.0, 1.0))
    );
    assert_eq!(
        asset.material_for_slot(None).diffuse,
        Some(Srgba::new(1.0, 0.0, 0.0, 1.0))
    );
}

#[test]
fn material_texture_native_enums_are_typed() {
    assert_eq!(
        MaterialTextureFilter::from_native_value(7),
        Some(MaterialTextureFilter::Anisotropic16x)
    );
    assert_eq!(MaterialTextureFilter::Trilinear.native_value(), 3);
    assert_eq!(
        MaterialTextureType::from_native_value(3),
        Some(MaterialTextureType::Cube)
    );
    assert_eq!(MaterialTextureType::TwoDimensional.native_value(), 1);
}

#[test]
fn parses_native_material_color_components() {
    assert_eq!(
        material_color_from_native("0.25,0.5,0.75,1"),
        Some(Srgba::new(0.25, 0.5, 0.75, 1.0))
    );
    assert_eq!(
        material_color_from_native("0.25,0.5,0.75"),
        Some(Srgba::new(0.25, 0.5, 0.75, 1.0))
    );
}

#[test]
fn material_engine_asset_paths_match_pak_source_layout() {
    // Identity-on-pak-source: the extractor lays material products
    // at their pak source paths, so the resolver only normalises +
    // ensures a `.mtl` suffix.
    assert_eq!(
        material_engine_asset_path("Materials/Road/defaultRoad"),
        "materials/road/defaultroad.mtl"
    );
    assert_eq!(
        material_engine_asset_path("objects/foo/bar.mtl"),
        "objects/foo/bar.mtl"
    );
    assert_eq!(
        material_override_engine_asset_path("Libs/MaterialOverrides/Example/Rim.xml"),
        "libs/materialoverrides/example/rim.xml"
    );
}

#[test]
fn material_asset_binding_accepts_pak_source_paths() {
    assert_eq!(
        material_asset_path_from_source("materials/terrain/default/default.mtl"),
        Some("materials/terrain/default/default.mtl".to_string())
    );
    // A path that omits the extension still rounds-trips to a
    // `.mtl` source path (callers occasionally drop the suffix).
    assert_eq!(
        material_asset_path_from_source("materials/road/defaultroad"),
        Some("materials/road/defaultroad.mtl".to_string())
    );
    assert_eq!(material_asset_path_from_source("  "), None);
}
