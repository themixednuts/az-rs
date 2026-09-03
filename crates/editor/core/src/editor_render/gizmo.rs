// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

/// Capability-driven request added after AZSCENE preview materialization.
#[derive(Component, Debug)]
pub(super) struct EditorComponentGizmoRequest;

/// Retained camera/light visualization attached to a canonically lowered
/// authored entity on the editor's dedicated gizmo render layer.
#[derive(Component, Debug)]
pub(super) struct EditorComponentGizmo;

pub(super) fn install_editor_component_gizmos(world: &mut World, entities: &[Entity]) {
    for &entity in entities {
        if world.get::<EditorComponentGizmoRequest>(entity).is_none() {
            continue;
        }
        let Some(asset) = editor_component_gizmo_asset(world, entity) else {
            continue;
        };
        let handle = world.resource_mut::<Assets<GizmoAsset>>().add(asset);
        world.entity_mut(entity).insert((
            Gizmo {
                handle,
                line_config: GizmoLineConfig {
                    width: 2.0,
                    perspective: false,
                    ..default()
                },
                depth_bias: -0.02,
            },
            RenderLayers::layer(GIZMO_RENDER_LAYER),
            EditorComponentGizmo,
        ));
    }
}

pub(super) fn editor_component_gizmo_asset(world: &World, entity: Entity) -> Option<GizmoAsset> {
    if let Some(projection) = world.get::<Projection>(entity) {
        return Some(camera_component_gizmo(projection));
    }
    if world.get::<DirectionalLight>(entity).is_some() {
        return Some(directional_light_component_gizmo());
    }
    if let Some(light) = world.get::<PointLight>(entity) {
        return Some(point_light_component_gizmo(light.range));
    }
    world
        .get::<SpotLight>(entity)
        .map(|light| spot_light_component_gizmo(light.range, light.inner_angle, light.outer_angle))
}

pub(super) fn camera_component_gizmo(projection: &Projection) -> GizmoAsset {
    let color = Color::srgb(0.25, 0.78, 1.0);
    let (
        near_distance,
        far_distance,
        near_half_width,
        near_half_height,
        far_half_width,
        far_half_height,
    ) = match projection {
        Projection::Perspective(projection) => {
            let near = projection.near.max(0.05);
            let far = (near + 5.0).min(projection.far.max(near + 0.05));
            let aspect = projection.aspect_ratio.max(0.1);
            let tangent = (projection.fov * 0.5).tan();
            let near_height = near * tangent;
            let far_height = far * tangent;
            (
                near,
                far,
                near_height * aspect,
                near_height,
                far_height * aspect,
                far_height,
            )
        }
        Projection::Orthographic(projection) => {
            let near = projection.near.max(0.05);
            let far = (near + 5.0).min(projection.far.max(near + 0.05));
            let area_size = projection.area.size();
            let half_height = if area_size.y.is_finite() && area_size.y > 0.0 {
                area_size.y * 0.5
            } else {
                match projection.scaling_mode {
                    bevy::camera::ScalingMode::FixedVertical { viewport_height } => {
                        viewport_height * projection.scale * 0.5
                    }
                    bevy::camera::ScalingMode::Fixed { height, .. }
                    | bevy::camera::ScalingMode::AutoMin {
                        min_height: height, ..
                    }
                    | bevy::camera::ScalingMode::AutoMax {
                        max_height: height, ..
                    } => height * projection.scale * 0.5,
                    _ => 2.5,
                }
            };
            let half_width = if area_size.x.is_finite() && area_size.x > 0.0 {
                area_size.x * 0.5
            } else {
                half_height * 16.0 / 9.0
            };
            (near, far, half_width, half_height, half_width, half_height)
        }
        Projection::Custom(_) => (0.1, 3.0, 0.08, 0.05, 1.6, 0.9),
    };

    let near = frustum_plane(near_distance, near_half_width, near_half_height);
    let far = frustum_plane(far_distance, far_half_width, far_half_height);
    let mut gizmo = GizmoAsset::default();
    gizmo.lineloop(near, color);
    gizmo.lineloop(far, color);
    for index in 0..4 {
        gizmo.line(near[index], far[index], color);
    }
    // Small body and forward tick make the camera readable even when its
    // clipped frustum is edge-on to the editor view.
    gizmo.lineloop(
        [
            Vec3::new(-0.18, -0.12, 0.0),
            Vec3::new(0.18, -0.12, 0.0),
            Vec3::new(0.18, 0.12, 0.0),
            Vec3::new(-0.18, 0.12, 0.0),
        ],
        color,
    );
    gizmo.line(Vec3::ZERO, Vec3::NEG_Z * 0.5, color);
    gizmo
}

pub(super) fn frustum_plane(distance: f32, half_width: f32, half_height: f32) -> [Vec3; 4] {
    [
        Vec3::new(-half_width, -half_height, -distance),
        Vec3::new(half_width, -half_height, -distance),
        Vec3::new(half_width, half_height, -distance),
        Vec3::new(-half_width, half_height, -distance),
    ]
}

pub(super) fn directional_light_component_gizmo() -> GizmoAsset {
    let color = Color::srgb(1.0, 0.78, 0.18);
    let mut gizmo = GizmoAsset::default();
    draw_circle_xy(&mut gizmo, 0.28, 0.0, color);
    let tip = Vec3::NEG_Z * 2.5;
    gizmo.line(Vec3::ZERO, tip, color);
    for offset in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y] {
        gizmo.line(tip, tip + Vec3::Z * 0.4 + offset * 0.22, color);
    }
    gizmo
}

pub(super) fn point_light_component_gizmo(range: f32) -> GizmoAsset {
    let color = Color::srgb(1.0, 0.68, 0.16);
    let radius = range.clamp(0.05, 25.0);
    let mut gizmo = GizmoAsset::default();
    draw_circle_xy(&mut gizmo, radius, 0.0, color);
    draw_circle_xz(&mut gizmo, radius, color);
    draw_circle_yz(&mut gizmo, radius, color);
    gizmo
}

pub(super) fn spot_light_component_gizmo(
    range: f32,
    inner_angle: f32,
    outer_angle: f32,
) -> GizmoAsset {
    let color = Color::srgb(1.0, 0.60, 0.14);
    let distance = range.clamp(0.05, 25.0);
    let outer_radius = (outer_angle.tan() * distance).abs().min(25.0);
    let inner_radius = (inner_angle.tan() * distance).abs().min(25.0);
    let mut gizmo = GizmoAsset::default();
    draw_circle_xy(&mut gizmo, outer_radius, -distance, color);
    draw_circle_xy(
        &mut gizmo,
        inner_radius,
        -distance,
        Color::srgb(1.0, 0.82, 0.35),
    );
    for corner in [
        Vec3::new(outer_radius, 0.0, -distance),
        Vec3::new(-outer_radius, 0.0, -distance),
        Vec3::new(0.0, outer_radius, -distance),
        Vec3::new(0.0, -outer_radius, -distance),
    ] {
        gizmo.line(Vec3::ZERO, corner, color);
    }
    gizmo.line(Vec3::ZERO, Vec3::NEG_Z * distance, color);
    gizmo
}

pub(super) fn draw_circle_xy(gizmo: &mut GizmoAsset, radius: f32, z: f32, color: Color) {
    gizmo.lineloop(circle_points(radius, |a, b| Vec3::new(a, b, z)), color);
}

pub(super) fn draw_circle_xz(gizmo: &mut GizmoAsset, radius: f32, color: Color) {
    gizmo.lineloop(circle_points(radius, |a, b| Vec3::new(a, 0.0, b)), color);
}

pub(super) fn draw_circle_yz(gizmo: &mut GizmoAsset, radius: f32, color: Color) {
    gizmo.lineloop(circle_points(radius, |a, b| Vec3::new(0.0, a, b)), color);
}

pub(super) fn circle_points(
    radius: f32,
    point: impl Fn(f32, f32) -> Vec3,
) -> impl Iterator<Item = Vec3> {
    const SEGMENTS: u8 = 32;
    (0..SEGMENTS).map(move |step| {
        let angle = f32::from(step) * std::f32::consts::TAU / f32::from(SEGMENTS);
        point(angle.cos() * radius, angle.sin() * radius)
    })
}

/// Explicit reason an editor-only visual proxy was added to an otherwise
/// canonically lowered entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditorPreviewPlaceholder {
    EmptyEntity,
}

/// Slightly enlarged, front-face-culled shell used as a readable hover rim.
#[derive(Component, Debug)]
pub(super) struct HoverOutlineShell;

/// Accent wire box that stays attached to the selected authored entity in all
/// scene-tool modes. Select mode therefore detaches only the transform gizmo.
#[derive(Component, Debug)]
pub(super) struct SelectionBounds;

/// Persistent editor ground grid. Two entities share this marker: minor and
/// major lines use separate semantic theme tones but remain two draw submits.
#[derive(Component, Debug)]
pub(super) struct GroundGrid;

#[derive(Component, Debug)]
pub(super) struct NeutralViewportEntity;

// ---------------------------------------------------------------------------
// Transform gizmos.
//
// Axis meshes attached to the selected authored entity, hit-tested with the
// same accelerated `MeshRayCast` used for scene picking (but filtered to only
// the gizmo axis parts, and excluded from scene picks). Dragging solves
// ray/plane math per axis, snaps from `EditorSceneToolState`, updates the bevy
// transform live, and reports the committed world transform so the viewport
// host can persist it through the authored-edit path.
// ---------------------------------------------------------------------------

/// Which transform gizmo is shown for the selected authored entity. Derived
/// from `EditorSceneToolState.tool` by the viewport host each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoMode {
    /// No gizmo (Select tool): the gizmo is detached.
    None,
    Translate,
    Rotate,
    Scale,
    /// Combined translate, rotate, and scale handles. Each pickable part keeps
    /// its concrete channel so the existing channel-specific drag math is
    /// reused without silently aliasing Transform to Move.
    Universal,
}

/// Transform space the gizmo axes are expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoSpace {
    World,
    Local,
}

/// Origin used to place the transform gizmo for the selected object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoPivot {
    /// The authored transform origin.
    Pivot,
    /// The rendered mesh bounds center.
    Center,
}

/// Snap configuration forwarded from `EditorSceneToolState`. `None` disables
/// snapping for that channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoSnap {
    pub translate_step_meters: Option<f32>,
    pub rotate_step_degrees: Option<f32>,
}

impl GizmoSnap {
    pub const NONE: Self = Self {
        translate_step_meters: None,
        rotate_step_degrees: None,
    };
}

/// One authored transform channel committed by a gizmo drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GizmoCommitValue {
    Position(Vec3),
    Rotation(Quat),
    Scale(Vec3),
}

/// The transform value a completed gizmo drag produced, tagged with the
/// authored `(document_id, object_id)` the caller must persist it against.
#[derive(Clone, Debug, PartialEq)]
pub struct GizmoCommit {
    pub document_id: String,
    pub object_id: String,
    pub value: GizmoCommitValue,
}

pub(super) const fn gizmo_commit_value(
    mode: GizmoMode,
    transform: Transform,
) -> Option<GizmoCommitValue> {
    match mode {
        GizmoMode::Translate => Some(GizmoCommitValue::Position(transform.translation)),
        GizmoMode::Rotate => Some(GizmoCommitValue::Rotation(transform.rotation)),
        GizmoMode::Scale => Some(GizmoCommitValue::Scale(transform.scale)),
        GizmoMode::None | GizmoMode::Universal => None,
    }
}

/// Live playback read-back for the mannequin preview animation player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MannequinPlaybackStatus {
    /// Current seek position (max across the preview's graph nodes).
    pub position_millis: u32,
    /// Whether every preview animation reported finished (non-looping end).
    pub finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    pub(super) const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub(super) const fn unit(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }

    /// Standard editor axis colors (data, not chrome): X red, Y green, Z blue.
    pub(super) const fn color(self) -> Color {
        match self {
            Self::X => Color::srgb(0.90, 0.22, 0.26),
            Self::Y => Color::srgb(0.36, 0.78, 0.30),
            Self::Z => Color::srgb(0.24, 0.49, 0.92),
        }
    }
}

/// Root of the gizmo, positioned at the selected entity and oriented by the
/// active transform space. Its children are the per-axis handles.
#[derive(Component)]
pub(super) struct GizmoRoot;

/// A pickable gizmo handle, tagged with the axis it controls. Excluded from
/// scene picking and used exclusively for gizmo hit-tests.
#[derive(Component, Clone, Copy)]
pub(super) struct GizmoAxisPart {
    pub(super) mode: GizmoMode,
    pub(super) axis: GizmoAxis,
}

/// Active gizmo drag captured on grab, replayed live until release.
#[derive(Clone, Debug)]
pub(super) struct GizmoDrag {
    /// Captured target. Selection/reconciliation after pointer-down must not
    /// redirect an in-flight drag to a different authored entity.
    pub(super) entity: Entity,
    pub(super) document_id: String,
    pub(super) object_id: String,
    pub(super) mode: GizmoMode,
    pub(super) axis: GizmoAxis,
    /// World-space unit axis the drag is constrained to.
    pub(super) axis_dir: Vec3,
    /// Gizmo origin (selected entity translation) at grab time.
    pub(super) origin: Vec3,
    /// Parent world transform at grab time, used to convert world-space gizmo
    /// motion back into the authored parent-relative Transform.
    pub(super) parent_global: GlobalTransform,
    pub(super) start_translation: Vec3,
    pub(super) start_global_rotation: Quat,
    pub(super) start_scale: Vec3,
    /// Axis offset (meters, translate/scale) sampled at grab.
    pub(super) start_offset: f32,
    /// Camera ray at grab, for measuring rotation angle deltas.
    pub(super) start_ray_origin: Vec3,
    pub(super) start_ray_dir: Vec3,
}

pub(super) const GIZMO_SHAFT_LEN: f32 = 1.35;
pub(super) const GIZMO_SHAFT_THICK: f32 = 0.05;
pub(super) const GIZMO_TIP: f32 = 0.16;
pub(super) const GIZMO_ARROW_HEIGHT: f32 = 0.32;
pub(super) const GIZMO_RING_MAJOR: f32 = 1.1;
pub(super) const GIZMO_RING_MINOR: f32 = 0.055;
/// Sensitivity mapping axis-drag meters to a scale multiplier delta.
pub(super) const GIZMO_SCALE_SENSITIVITY: f32 = 0.6;

/// Intersect a ray with a plane; `None` when the ray is parallel to the plane.
pub(super) fn ray_plane_intersection(
    ray_origin: Vec3,
    ray_dir: Vec3,
    plane_point: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> {
    let denom = ray_dir.dot(plane_normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_point - ray_origin).dot(plane_normal) / denom;
    if !t.is_finite() {
        return None;
    }
    Some(ray_origin + ray_dir * t)
}

/// The plane containing `axis_dir` whose normal is most aligned with the view
/// direction, giving the cleanest ray intersection for an axis-constrained
/// drag. `None` when the view direction is (near) parallel to the axis.
pub(super) fn axis_drag_plane_normal(axis_dir: Vec3, view_dir: Vec3) -> Option<Vec3> {
    let normal = view_dir - axis_dir * view_dir.dot(axis_dir);
    let normal = normal.normalize_or_zero();
    (normal.length_squared() > 1e-8).then_some(normal)
}

/// Signed offset (meters) of the mouse ray along `axis_dir` from `axis_origin`,
/// via ray/plane intersection against the most camera-facing plane containing
/// the axis. The drag delta is the difference of two such offsets.
pub(super) fn axis_drag_offset(
    ray_origin: Vec3,
    ray_dir: Vec3,
    axis_origin: Vec3,
    axis_dir: Vec3,
    view_dir: Vec3,
) -> Option<f32> {
    let plane_normal = axis_drag_plane_normal(axis_dir, view_dir)?;
    let hit = ray_plane_intersection(ray_origin, ray_dir, axis_origin, plane_normal)?;
    Some((hit - axis_origin).dot(axis_dir))
}

pub(super) fn local_translation_after_world_delta(
    start_translation: Vec3,
    parent_global: GlobalTransform,
    world_delta: Vec3,
) -> Vec3 {
    start_translation
        + parent_global
            .affine()
            .inverse()
            .transform_vector3(world_delta)
}

pub(super) fn local_rotation_from_global(
    parent_global: GlobalTransform,
    global_rotation: Quat,
) -> Quat {
    (parent_global.rotation().inverse() * global_rotation).normalize()
}

/// Quantize `value` to the nearest multiple of `step` (no-op for `step <= 0`).
pub(super) fn snap_scalar(value: f32, step: f32) -> f32 {
    if step > 0.0 && step.is_finite() {
        (value / step).round() * step
    } else {
        value
    }
}

/// Signed rotation (radians) around `axis_normal` between where two rays strike
/// the rotation plane through `center`. `None` on a degenerate configuration.
pub(super) fn rotation_drag_angle(
    grab_origin: Vec3,
    grab_dir: Vec3,
    cursor_origin: Vec3,
    cursor_dir: Vec3,
    center: Vec3,
    axis_normal: Vec3,
) -> Option<f32> {
    let a = (ray_plane_intersection(grab_origin, grab_dir, center, axis_normal)? - center)
        .normalize_or_zero();
    let b = (ray_plane_intersection(cursor_origin, cursor_dir, center, axis_normal)? - center)
        .normalize_or_zero();
    if a.length_squared() < 1e-8 || b.length_squared() < 1e-8 {
        return None;
    }
    let cross = a.cross(b).dot(axis_normal);
    let dot = a.dot(b).clamp(-1.0, 1.0);
    Some(cross.atan2(dot))
}

pub(super) fn selection_bounds_mesh() -> Mesh {
    let lo = -0.515;
    let hi = 0.515;
    let corners = [
        Vec3::new(lo, lo, lo),
        Vec3::new(hi, lo, lo),
        Vec3::new(hi, hi, lo),
        Vec3::new(lo, hi, lo),
        Vec3::new(lo, lo, hi),
        Vec3::new(hi, lo, hi),
        Vec3::new(hi, hi, hi),
        Vec3::new(lo, hi, hi),
    ];
    let edge_indices = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    let positions = edge_indices
        .into_iter()
        .flat_map(|(start, end)| [corners[start], corners[end]])
        .collect::<Vec<_>>();
    Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
}

pub(super) fn selection_bounds_transform(aabb: &Aabb) -> Transform {
    let center = Vec3::from(aabb.center);
    let full_extents = Vec3::from(aabb.half_extents) * 2.0;
    // `selection_bounds_mesh` intentionally overdraws a unit cube by 3%.
    Transform::from_translation(center).with_scale(full_extents / 1.03)
}

pub(super) fn fit_selection_bounds_to_real_aabb(
    mut bounds: Query<(&ChildOf, &mut Transform), With<SelectionBounds>>,
    aabbs: Query<&Aabb>,
) {
    for (parent, mut transform) in &mut bounds {
        let Ok(aabb) = aabbs.get(parent.parent()) else {
            continue;
        };
        *transform = selection_bounds_transform(aabb);
    }
}

/// Spawn the per-axis handle entities for one gizmo axis as children of the
/// gizmo root, oriented for the given mode. Every handle carries
/// [`GizmoAxisPart`] so it is hit-testable and excluded from scene picks.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_gizmo_axis(
    world: &mut World,
    root: Entity,
    mode: GizmoMode,
    axis: GizmoAxis,
    shaft: &Handle<Mesh>,
    translate_tip: &Handle<Mesh>,
    scale_tip: &Handle<Mesh>,
    ring: &Handle<Mesh>,
    material: Handle<StandardMaterial>,
) {
    if mode == GizmoMode::Universal {
        for channel in [GizmoMode::Translate, GizmoMode::Rotate, GizmoMode::Scale] {
            spawn_gizmo_axis(
                world,
                root,
                channel,
                axis,
                shaft,
                translate_tip,
                scale_tip,
                ring,
                material.clone(),
            );
        }
        return;
    }
    let unit = axis.unit();
    let shaft_rotation = Quat::from_rotation_arc(Vec3::X, unit);
    match mode {
        GizmoMode::Translate => {
            world.spawn((
                Mesh3d(shaft.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(unit * (GIZMO_SHAFT_LEN * 0.5))
                    .with_rotation(shaft_rotation),
                Visibility::default(),
                RenderLayers::layer(GIZMO_RENDER_LAYER),
                GizmoAxisPart { mode, axis },
                ChildOf(root),
            ));
            world.spawn((
                Mesh3d(translate_tip.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(
                    unit * GIZMO_ARROW_HEIGHT.mul_add(0.35, GIZMO_SHAFT_LEN),
                )
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, unit)),
                Visibility::default(),
                RenderLayers::layer(GIZMO_RENDER_LAYER),
                GizmoAxisPart { mode, axis },
                ChildOf(root),
            ));
        }
        GizmoMode::Scale => {
            world.spawn((
                Mesh3d(shaft.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(unit * (GIZMO_SHAFT_LEN * 0.5))
                    .with_rotation(shaft_rotation),
                Visibility::default(),
                RenderLayers::layer(GIZMO_RENDER_LAYER),
                GizmoAxisPart { mode, axis },
                ChildOf(root),
            ));
            world.spawn((
                Mesh3d(scale_tip.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(unit * GIZMO_SHAFT_LEN),
                Visibility::default(),
                RenderLayers::layer(GIZMO_RENDER_LAYER),
                GizmoAxisPart { mode, axis },
                ChildOf(root),
            ));
        }
        GizmoMode::Rotate => {
            let ring_rotation = Quat::from_rotation_arc(Vec3::Y, unit);
            world.spawn((
                Mesh3d(ring.clone()),
                MeshMaterial3d(material),
                Transform::from_rotation(ring_rotation),
                Visibility::default(),
                RenderLayers::layer(GIZMO_RENDER_LAYER),
                GizmoAxisPart { mode, axis },
                ChildOf(root),
            ));
        }
        GizmoMode::None | GizmoMode::Universal => {}
    }
}
