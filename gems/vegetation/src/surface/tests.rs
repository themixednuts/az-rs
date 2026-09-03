use az_core::crc::Crc32;

use super::*;

#[test]
fn vegetation_surface_tags_use_az_crc_values() {
    assert_eq!(
        VegetationSurfaceTag::from_name("terrain"),
        VegetationSurfaceTag::from_crc32(Crc32::from_str_lower("terrain"))
    );
    assert_eq!(
        VegetationSurfaceTag::default(),
        VegetationSurfaceTag::UNASSIGNED
    );
}

#[test]
fn surface_mask_weights_keep_maximum_value_per_tag() {
    let mut masks = vec![VegetationSurfaceTagWeight::new(
        VegetationSurfaceTag::TERRAIN,
        0.25,
    )];

    add_max_surface_weight(&mut masks, VegetationSurfaceTag::TERRAIN, 0.1);
    add_max_surface_weight(&mut masks, VegetationSurfaceTag::TERRAIN, 0.75);
    add_max_surface_weight(&mut masks, VegetationSurfaceTag::TERRAIN_HOLE, 1.0);

    assert_eq!(
        masks,
        vec![
            VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN, 0.75),
            VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN_HOLE, 1.0),
        ]
    );
}

#[test]
fn surface_mask_weights_merge_sources_by_maximum_value() {
    let mut masks = vec![VegetationSurfaceTagWeight::new(
        VegetationSurfaceTag::TERRAIN,
        0.25,
    )];

    merge_max_surface_weights(
        &mut masks,
        [
            VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN, 0.5),
            VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN_HOLE, 1.0),
        ],
    );

    assert_eq!(
        masks,
        vec![
            VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN, 0.5),
            VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN_HOLE, 1.0),
        ]
    );
}

#[test]
fn surface_tag_weight_matching_uses_tags_and_weight_range() {
    let masks = vec![
        VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN, 0.5),
        VegetationSurfaceTagWeight::new(VegetationSurfaceTag::TERRAIN_HOLE, 1.0),
    ];

    assert!(has_valid_surface_tags(&[VegetationSurfaceTag::TERRAIN]));
    assert!(!has_valid_surface_tags(&[VegetationSurfaceTag::UNASSIGNED]));
    assert!(has_matching_surface_tag_weight(
        &masks,
        &[VegetationSurfaceTag::TERRAIN],
        0.1,
        0.75
    ));
    assert!(!has_matching_surface_tag_weight(
        &masks,
        &[VegetationSurfaceTag::TERRAIN],
        0.75,
        1.0
    ));
}
