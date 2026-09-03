use super::*;

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn mesh_render_options_defaults_match_lumberyard_source() {
    let options = MeshRenderOptions::default();

    assert_eq!(options.opacity, 1.0);
    assert_eq!(options.view_distance_multiplier, 1.0);
    assert_eq!(options.lod_ratio, 100);
    assert!(options.use_vis_areas);
    assert!(options.cast_shadows);
    assert!(options.rain_occluder);
    assert!(options.accept_decals);
    assert!(options.affect_navmesh);
    assert!(!options.receive_wind);
    assert!(!options.is_static());
}

#[test]
fn mesh_render_options_static_state_controls_gi() {
    let mut options = MeshRenderOptions {
        has_static_transform: true,
        ..Default::default()
    };

    assert!(options.is_static());
    assert!(options.affects_gi());

    options.dynamic_mesh = true;
    assert!(!options.is_static());
    assert!(!options.affects_gi());
}
