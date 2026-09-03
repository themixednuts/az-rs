use super::*;

// Both floats are asserted against the very literals `Default` assigns them,
// so the comparison is bit-identical by construction.
#[allow(clippy::float_cmp)]
#[test]
fn river_defaults_match_lumberyard_source() {
    let river = RiverComponent::default();

    assert_eq!(river.material_path, DEFAULT_RIVER_MATERIAL);
    assert_eq!(river.water_volume_depth, 10.0);
    assert_eq!(river.tile_width, 1.0);
    assert!(river.water_caustics);
    assert!(!river.physicalize);
}
