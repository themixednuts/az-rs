use super::*;

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn skinned_render_options_defaults_match_lumberyard_source() {
    let options = SkinnedRenderOptions::default();

    assert_eq!(options.opacity, 1.0);
    assert_eq!(options.view_distance_multiplier, 1.0);
    assert_eq!(options.lod_ratio, 100);
    assert!(options.use_vis_areas);
    assert!(options.cast_dynamic_shadows);
    assert!(options.rain_occluder);
    assert!(options.accept_decals);
}
