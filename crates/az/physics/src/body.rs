use std::num::NonZeroU64;

use az_core::crc::Crc32;
use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    BuoyancyStatus, CollisionClass, CollisionFilter, FluidAreaConfiguration,
    LivingBodyConfiguration, PhysicsError, PhysicsPose, RigidBodyBuoyancy,
};

/// Stable identity for one isolated physics scene.
#[derive(
    bevy_ecs::component::Component,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct PhysicsSceneId(u64);

impl PhysicsSceneId {
    /// Scene used by ordinary single-world clients and the primary server
    /// instance.
    pub const DEFAULT: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable engine handle for a body in an isolated physics scene.
#[derive(
    bevy_ecs::component::Component,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct PhysicsBodyHandle {
    scene: PhysicsSceneId,
    value: NonZeroU64,
}

impl PhysicsBodyHandle {
    /// Constructs a handle in [`PhysicsSceneId::DEFAULT`]. Solver adapters
    /// serving a scene registry use [`Self::in_scene`] instead.
    #[must_use]
    pub const fn new(value: NonZeroU64) -> Self {
        Self::in_scene(PhysicsSceneId::DEFAULT, value)
    }

    #[must_use]
    pub const fn in_scene(scene: PhysicsSceneId, value: NonZeroU64) -> Self {
        Self { scene, value }
    }

    #[must_use]
    pub const fn scene(self) -> PhysicsSceneId {
        self.scene
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.value.get()
    }
}

/// Engine-independent identity associated with a body.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Reflect,
)]
pub struct PhysicsEntityId(pub u64);

/// Surface/material identifier used by collision and ray-query results.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct SurfaceIndex(pub i32);

/// Cry physical-entity kind (`pe_type`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalEntityType {
    None,
    Static,
    Rigid,
    WheeledVehicle,
    Living,
    Particle,
    Articulated,
    Rope,
    Soft,
    Area,
}

/// Cry simulation class.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum SimulationClass {
    Static,
    SleepingRigid,
    ActiveRigid,
    Living,
    Independent,
    Trigger,
    Deleted,
}

/// Solver-neutral rigid-body motion category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum RigidBodyMotion {
    #[default]
    Dynamic,
    KinematicPosition,
    KinematicVelocity,
}

/// `RockNRoll` sleeping-condition discriminants used by the rigid-body
/// evaluator.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum RockNRollSleepMode {
    /// Native mode 0: reset eligibility and never deactivate automatically.
    Disabled = 0,
    /// Native mode 1: instantaneous linear and angular speed squared.
    InstantaneousVelocity = 1,
    /// Native mode 2: `0.1 * current + 0.9 * previous` speed squared.
    SmoothedVelocity = 2,
    /// Native mode 3: instantaneous translational plus rotational energy.
    InstantaneousEnergy = 3,
    /// Native mode 4 and the default: smoothed kinetic energy.
    #[default]
    SmoothedEnergy = 4,
}

/// Runtime ownership of rigid-body sleeping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum RigidBodySleepPolicy {
    /// Delegate to the solver's linear/angular velocity thresholds.
    #[default]
    SolverVelocityThresholds,
    /// `CryPhysics` `CRigidEntity::Update` energy threshold and support gate.
    CryEnergy,
    /// An engine/gem-level evaluator owns eligibility, listener vetoes, and
    /// island deactivation; the solver must not sleep the body independently.
    External,
    /// `RockNRoll` evaluates the selected native condition and deactivates only
    /// when every dynamic member of the contact/constraint island qualifies.
    RockNRoll(RockNRollSleepMode),
}

/// Velocity-damping integration selected by an authored physics system.
///
/// Solver adapters keep their native stable damping for ordinary bodies. The
/// linear-step model preserves engines that apply `clamp(1 - damping * dt)`
/// and an explicit low-speed decrement as part of each substep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum RigidBodyDampingModel {
    #[default]
    Solver,
    LinearStep {
        low_speed_decrement: f32,
    },
}

impl RigidBodyDampingModel {
    fn validate(self) -> Result<(), PhysicsError> {
        match self {
            Self::Solver => Ok(()),
            Self::LinearStep {
                low_speed_decrement,
            } if low_speed_decrement.is_finite() && low_speed_decrement >= 0.0 => Ok(()),
            Self::LinearStep { .. } => Err(PhysicsError::InvalidRigidBodyScalar {
                field: "low_speed_decrement",
            }),
        }
    }
}

/// Solver-neutral representation of the five `RockNRoll` continuous collision
/// values.
///
/// Numeric labels remain explicit because reflection does not expose names for
/// every mode.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ContinuousCollisionMode {
    #[default]
    Disabled = 0,
    Mode1 = 1,
    Mode2 = 2,
    OrderedTimeOfImpact = 3,
    ReverseDisplacementSweep = 4,
}

/// Positional tolerance used by `RockNRoll`'s hit-projection cleanup.
///
/// This value is independent of the `0.05` broadphase/prediction margin.
pub const CONTINUOUS_HIT_PROJECTION_EPSILON: f32 = 0.01;

/// Fractions produced by `RockNRoll`'s modes 1/2 post-hit projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContinuousHitProjection {
    /// Fraction of the predicted pose retained after the hit.
    pub pose_fraction: f32,
    /// Fraction of velocity along the hit normal left on the body.
    pub normal_velocity_retained_fraction: f32,
}

/// Evaluates the scalar part of `RockNRoll`'s modes 1/2 projection cleanup.
///
/// `hit_fraction` is the shape-pair cast fraction, `body_displacement` is the
/// body's translation distance over the substep, and
/// `reference_point_displacement` is the distance travelled by the recorded
/// shape/reference point including rotation. Inputs originate from validated
/// finite body state.
#[must_use]
pub fn continuous_hit_projection(
    hit_fraction: f32,
    body_displacement: f32,
    reference_point_displacement: f32,
    distance_factor: f32,
) -> ContinuousHitProjection {
    debug_assert!(hit_fraction.is_finite());
    debug_assert!(body_displacement.is_finite() && body_displacement >= 0.0);
    debug_assert!(reference_point_displacement.is_finite() && reference_point_displacement >= 0.0);
    debug_assert!(distance_factor.is_finite() && distance_factor >= 0.0);

    let normal_velocity_retained_fraction = if body_displacement > CONTINUOUS_HIT_PROJECTION_EPSILON
    {
        (reference_point_displacement.mul_add(distance_factor, -CONTINUOUS_HIT_PROJECTION_EPSILON)
            / body_displacement)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pose_fraction = if body_displacement == 0.0 {
        1.0
    } else {
        (hit_fraction + CONTINUOUS_HIT_PROJECTION_EPSILON / body_displacement).clamp(0.0, 1.0)
    };

    ContinuousHitProjection {
        pose_fraction,
        normal_velocity_retained_fraction,
    }
}

impl ContinuousCollisionMode {
    #[inline]
    #[must_use]
    pub const fn uses_hit_projection(self) -> bool {
        matches!(self, Self::Mode1 | Self::Mode2)
    }

    #[inline]
    #[must_use]
    pub const fn uses_ordered_time_of_impact(self) -> bool {
        matches!(self, Self::OrderedTimeOfImpact)
    }

    #[inline]
    #[must_use]
    pub const fn reverses_sweep_displacement(self) -> bool {
        matches!(self, Self::ReverseDisplacementSweep)
    }

    /// Returns whether this mode turns positive-separation contacts into
    /// frictionless speculative normal constraints.
    ///
    /// The native mode-4 solver branch is distinct from its reversed
    /// broadphase proxy: it projects a retained positive-separation contact
    /// onto the opposite witness and uses `-separation / dt` as its normal
    /// velocity bias.
    #[inline]
    #[must_use]
    pub const fn uses_speculative_normal_constraints(self) -> bool {
        matches!(self, Self::ReverseDisplacementSweep)
    }
}

/// Entity categories accepted by scene queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct PhysicalEntityTypes(u32);

impl PhysicalEntityTypes {
    pub const NONE: Self = Self(0);
    pub const STATIC: Self = Self(1 << 0);
    pub const DYNAMIC: Self = Self(1 << 1);
    pub const LIVING: Self = Self(1 << 2);
    pub const INDEPENDENT: Self = Self(1 << 3);
    pub const TERRAIN: Self = Self(1 << 4);
    pub const ALL: Self = Self(
        Self::STATIC.0 | Self::DYNAMIC.0 | Self::LIVING.0 | Self::INDEPENDENT.0 | Self::TERRAIN.0,
    );

    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & Self::ALL.0)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl Default for PhysicalEntityTypes {
    fn default() -> Self {
        Self::ALL
    }
}

impl std::ops::BitOr for PhysicalEntityTypes {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for PhysicalEntityTypes {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

/// Principal axis for cylinders and capsules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum Axis3 {
    X,
    Y,
    #[default]
    Z,
}

/// Solver-independent primitive collider shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ColliderShape {
    Sphere {
        radius: f32,
    },
    Cuboid {
        half_extents: Vec3,
    },
    Capsule {
        axis: Axis3,
        half_height: f32,
        radius: f32,
    },
    Cylinder {
        axis: Axis3,
        half_height: f32,
        radius: f32,
    },
    RoundedCuboid {
        half_extents: Vec3,
        border_radius: f32,
    },
    RoundedCylinder {
        axis: Axis3,
        half_height: f32,
        radius: f32,
        border_radius: f32,
    },
    CapsuleSegment {
        endpoint_a: Vec3,
        endpoint_b: Vec3,
        radius: f32,
    },
    RoundedCylinderSegment {
        endpoint_a: Vec3,
        endpoint_b: Vec3,
        radius: f32,
        border_radius: f32,
    },
    ConvexHull {
        points: Vec<Vec3>,
        border_radius: f32,
    },
    Triangle {
        vertices: [Vec3; 3],
        border_radius: f32,
    },
    TriangleMesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
    Plane {
        normal: Vec3,
        offset: f32,
        aabb_min: Vec3,
        aabb_max: Vec3,
    },
    HeightField {
        width: u32,
        length: u32,
        heights: Vec<f32>,
        aabb_min: Vec3,
        aabb_max: Vec3,
        up_axis: Axis3,
    },
}

impl ColliderShape {
    /// Returns a positive closed-shape volume when it can be recovered without
    /// backend-specific geometry. `None` intentionally triggers Cry's equal
    /// per-part mass fallback.
    #[must_use]
    pub fn volume(&self) -> Option<f32> {
        let pi = core::f32::consts::PI;
        match self {
            Self::Sphere { radius } => Some((4.0 / 3.0) * pi * radius.powi(3)),
            Self::Cuboid { half_extents } => {
                Some(8.0 * half_extents.x * half_extents.y * half_extents.z)
            }
            Self::Capsule {
                half_height,
                radius,
                ..
            } => {
                let segment_length = 2.0 * half_height;
                Some(
                    ((4.0 / 3.0) * pi)
                        .mul_add(radius.powi(3), pi * radius.powi(2) * segment_length),
                )
            }
            Self::CapsuleSegment {
                endpoint_a,
                endpoint_b,
                radius,
            } => Some(((4.0 / 3.0) * pi).mul_add(
                radius.powi(3),
                pi * radius.powi(2) * endpoint_a.distance(*endpoint_b),
            )),
            Self::Cylinder {
                half_height,
                radius,
                ..
            } => Some(pi * radius.powi(2) * (2.0 * half_height)),
            Self::TriangleMesh { vertices, indices } => {
                let signed = indices.iter().fold(0.0, |volume, triangle| {
                    let a = vertices[triangle[0] as usize];
                    let b = vertices[triangle[1] as usize];
                    let c = vertices[triangle[2] as usize];
                    volume + a.dot(b.cross(c)) / 6.0
                });
                (signed.abs() > f32::EPSILON).then_some(signed.abs())
            }
            Self::RoundedCuboid { .. }
            | Self::RoundedCylinder { .. }
            | Self::RoundedCylinderSegment { .. }
            | Self::ConvexHull { .. }
            | Self::Triangle { .. }
            | Self::Plane { .. }
            | Self::HeightField { .. } => None,
        }
    }

    /// Validates finite, positive primitive dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] when any dimension is non-finite or outside
    /// the range accepted by physics backends.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        match self {
            Self::Sphere { radius } => validate_positive(*radius, "radius"),
            Self::Cuboid { half_extents } => validate_half_extents(*half_extents),
            Self::Capsule {
                half_height,
                radius,
                ..
            }
            | Self::Cylinder {
                half_height,
                radius,
                ..
            } => {
                validate_positive(*half_height, "half_height")?;
                validate_positive(*radius, "radius")
            }
            Self::RoundedCuboid {
                half_extents,
                border_radius,
            } => {
                validate_half_extents(*half_extents)?;
                validate_non_negative(*border_radius, "border_radius")
            }
            Self::RoundedCylinder {
                half_height,
                radius,
                border_radius,
                ..
            } => {
                validate_positive(*half_height, "half_height")?;
                validate_positive(*radius, "radius")?;
                validate_non_negative(*border_radius, "border_radius")
            }
            Self::CapsuleSegment {
                endpoint_a,
                endpoint_b,
                radius,
            }
            | Self::RoundedCylinderSegment {
                endpoint_a,
                endpoint_b,
                radius,
                ..
            } => {
                validate_segment(*endpoint_a, *endpoint_b)?;
                validate_positive(*radius, "radius")?;
                if let Self::RoundedCylinderSegment { border_radius, .. } = self {
                    validate_non_negative(*border_radius, "border_radius")?;
                }
                Ok(())
            }
            Self::ConvexHull {
                points,
                border_radius,
            } => {
                validate_vertices(points, 4, "convex hull")?;
                validate_non_negative(*border_radius, "border_radius")
            }
            Self::Triangle {
                vertices,
                border_radius,
            } => validate_triangle(vertices, *border_radius),
            Self::TriangleMesh { vertices, indices } => validate_triangle_mesh(vertices, indices),
            Self::Plane {
                normal,
                offset,
                aabb_min,
                aabb_max,
            } => validate_plane(*normal, *offset, *aabb_min, *aabb_max),
            Self::HeightField {
                width,
                length,
                heights,
                aabb_min,
                aabb_max,
                ..
            } => validate_height_field(*width, *length, heights, *aabb_min, *aabb_max),
        }
    }
}

/// Rejects box half extents that are non-finite or not positive on every axis.
fn validate_half_extents(half_extents: Vec3) -> Result<(), PhysicsError> {
    if half_extents.is_finite() && half_extents.cmpgt(Vec3::ZERO).all() {
        Ok(())
    } else {
        Err(PhysicsError::InvalidColliderHalfExtents)
    }
}

/// Rejects a triangle collider whose vertices are non-finite or collinear, or
/// whose border radius is negative.
fn validate_triangle(vertices: &[Vec3; 3], border_radius: f32) -> Result<(), PhysicsError> {
    validate_vertices(vertices, 3, "triangle")?;
    if (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .length_squared()
        <= f32::EPSILON
    {
        return Err(PhysicsError::InvalidColliderTopology("degenerate triangle"));
    }
    validate_non_negative(border_radius, "border_radius")
}

/// Rejects a triangle mesh with non-finite vertices, no indices, or an index
/// past the end of the vertex buffer.
fn validate_triangle_mesh(vertices: &[Vec3], indices: &[[u32; 3]]) -> Result<(), PhysicsError> {
    validate_vertices(vertices, 3, "triangle mesh")?;
    if indices.is_empty()
        || indices
            .iter()
            .flatten()
            .any(|&index| index as usize >= vertices.len())
    {
        return Err(PhysicsError::InvalidColliderTopology(
            "triangle mesh indices are empty or out of range",
        ));
    }
    Ok(())
}

/// Rejects a plane collider with a degenerate normal, a non-finite offset, or
/// bounds that are non-finite or inverted.
fn validate_plane(
    normal: Vec3,
    offset: f32,
    aabb_min: Vec3,
    aabb_max: Vec3,
) -> Result<(), PhysicsError> {
    if !normal.is_finite()
        || normal.length_squared() <= f32::EPSILON
        || !offset.is_finite()
        || !aabb_min.is_finite()
        || !aabb_max.is_finite()
        || !aabb_max.cmpgt(aabb_min).all()
    {
        return Err(PhysicsError::InvalidColliderTopology(
            "plane normal, offset, or bounds are invalid",
        ));
    }
    Ok(())
}

/// Rejects a height field whose grid is smaller than two samples per axis, whose
/// sample count does not match `width * length`, or whose samples or bounds are
/// non-finite or inverted.
fn validate_height_field(
    width: u32,
    length: u32,
    heights: &[f32],
    aabb_min: Vec3,
    aabb_max: Vec3,
) -> Result<(), PhysicsError> {
    let count = (width as usize).checked_mul(length as usize);
    if width < 2
        || length < 2
        || count != Some(heights.len())
        || heights.iter().any(|height| !height.is_finite())
        || !aabb_min.is_finite()
        || !aabb_max.is_finite()
        || !aabb_max.cmpgt(aabb_min).all()
    {
        return Err(PhysicsError::InvalidColliderTopology(
            "height field dimensions, samples, or bounds are invalid",
        ));
    }
    Ok(())
}

fn validate_segment(endpoint_a: Vec3, endpoint_b: Vec3) -> Result<(), PhysicsError> {
    if endpoint_a.is_finite()
        && endpoint_b.is_finite()
        && endpoint_a.distance_squared(endpoint_b) > f32::EPSILON
    {
        Ok(())
    } else {
        Err(PhysicsError::InvalidColliderTopology(
            "segment endpoints must be finite and distinct",
        ))
    }
}

fn validate_vertices(
    vertices: &[Vec3],
    minimum: usize,
    description: &'static str,
) -> Result<(), PhysicsError> {
    if vertices.len() >= minimum && vertices.iter().all(|vertex| vertex.is_finite()) {
        Ok(())
    } else {
        Err(PhysicsError::InvalidColliderTopology(description))
    }
}

fn validate_positive(value: f32, field: &'static str) -> Result<(), PhysicsError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(PhysicsError::InvalidColliderScalar { field })
    }
}

/// Runtime collider tag corresponding to Lumberyard's case-insensitive
/// `Physics::Shape::GetTag()` value.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Reflect,
)]
#[repr(transparent)]
pub struct ColliderTag(Crc32);

impl ColliderTag {
    pub const NONE: Self = Self(Crc32::ZERO);

    #[inline]
    #[must_use]
    pub const fn from_crc(crc: Crc32) -> Self {
        Self(crc)
    }

    #[inline]
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self(Crc32::from_str_lower(name))
    }

    #[inline]
    #[must_use]
    pub const fn crc(self) -> Crc32 {
        self.0
    }
}

impl From<Crc32> for ColliderTag {
    #[inline]
    fn from(value: Crc32) -> Self {
        Self(value)
    }
}

impl From<ColliderTag> for Crc32 {
    #[inline]
    fn from(value: ColliderTag) -> Self {
        value.0
    }
}

impl From<&str> for ColliderTag {
    #[inline]
    fn from(value: &str) -> Self {
        Self::from_name(value)
    }
}

impl AsRef<Crc32> for ColliderTag {
    #[inline]
    fn as_ref(&self) -> &Crc32 {
        &self.0
    }
}

/// Collider shape, local pose, material, and Cry collision classification.
#[expect(
    clippy::struct_excessive_bools,
    reason = "sensor, simulated, in_scene_queries, buoyancy_enabled, and interacts_with_triggers are five independent engine participation flags a collider can hold in any combination"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ColliderConfiguration {
    pub shape: ColliderShape,
    pub local_pose: PhysicsPose,
    pub collision_class: CollisionClass,
    /// Optional compiled `RockNRoll` category filter. This remains
    /// orthogonal to `CryPhysics`' `SCollisionClass`; the Rapier adapter applies
    /// each rule only to peers participating in that filtering domain.
    pub collision_filter: Option<CollisionFilter>,
    pub surface_index: SurfaceIndex,
    pub surface_pierceability: u8,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    /// Explicit mass contribution for this collider. When present it takes
    /// precedence over density and lets authoring reproduce per-part mass
    /// distribution without exposing solver-native mass properties.
    pub mass: Option<f32>,
    pub sensor: bool,
    /// Whether this shape participates in solver contacts. Trigger shapes
    /// participate as sensors regardless of this flag, matching Lumberyard.
    #[serde(default = "default_true")]
    pub simulated: bool,
    /// Whether ray casts, overlaps, and shape casts can see this collider.
    #[serde(default = "default_true")]
    pub in_scene_queries: bool,
    /// Whether this part contributes medium resistance, displaced volume, and
    /// buoyancy. This is `CryPhysics` `geom_floats`; it is enabled by default in
    /// `pe_geomparams` and remains independent of contact/query participation.
    #[serde(default = "default_true")]
    pub buoyancy_enabled: bool,
    /// Whether this collider receives overlaps from trigger/area colliders.
    pub interacts_with_triggers: bool,
    /// Case-insensitive collider identifier used by per-shape APIs.
    #[serde(default)]
    pub tag: ColliderTag,
    /// Solver rest distance. Negative values are allowed.
    #[serde(default)]
    pub rest_offset: f32,
    /// Geometric distance at which contacts begin to be generated.
    #[serde(default = "default_contact_offset")]
    pub contact_offset: f32,
}

const fn default_true() -> bool {
    true
}

const fn default_contact_offset() -> f32 {
    0.02
}

/// Authored collider collection shared by asset-driven and entity-driven
/// body components. Bodies borrow this slice while constructing descriptors.
#[derive(
    bevy_ecs::component::Component,
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct PhysicsColliderSet(pub Vec<ColliderConfiguration>);

/// One geometry instance before a body component assigns material, filtering,
/// trigger, or mass behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct PhysicsShapeInstance {
    pub shape: ColliderShape,
    pub local_pose: PhysicsPose,
}

impl From<ColliderShape> for PhysicsShapeInstance {
    fn from(shape: ColliderShape) -> Self {
        Self {
            shape,
            local_pose: PhysicsPose::IDENTITY,
        }
    }
}

/// Backend-neutral geometry product emitted by shape components. Collider and
/// trigger components consume this same product, so geometry has one runtime
/// construction path.
#[derive(
    bevy_ecs::component::Component,
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct PhysicsShapeSet(pub Vec<PhysicsShapeInstance>);

/// Product-side triangle geometry consumed by mesh-collider components.
///
/// Asset builders/loaders populate this from their native runtime mesh product;
/// physics does not know or decode any authoring or legacy mesh format.
#[derive(
    bevy_ecs::component::Component,
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    Reflect,
)]
pub struct PhysicsMeshGeometry {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}

impl PhysicsMeshGeometry {
    /// Validates the product topology using the same rules as a runtime
    /// triangle-mesh collider.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidColliderTopology`] when there are fewer
    /// than three vertices, a vertex is non-finite, `indices` is empty, or an
    /// index points past the end of `vertices`.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        ColliderShape::TriangleMesh {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
        }
        .validate()
    }
}

impl PhysicsColliderSet {
    /// Distributes a total body mass across parts by geometry volume. If any
    /// part has no positive solver volume, `CryPhysics` falls back to equal mass
    /// per part; callers get that same deterministic behavior here.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidRigidBodyScalar`] with field `mass` when
    /// `total_mass` is non-finite or not greater than zero, and
    /// [`PhysicsError::MissingCollider`] when the set is empty.
    ///
    /// # Panics
    ///
    /// Panics if a part volume is `None` on the volume-weighted branch. That
    /// branch is only taken when `all_have_volume` has already proved every
    /// entry is `Some`, so the `expect` is unreachable.
    pub fn distribute_mass(&mut self, total_mass: f32) -> Result<(), PhysicsError> {
        if !total_mass.is_finite() || total_mass <= 0.0 {
            return Err(PhysicsError::InvalidRigidBodyScalar { field: "mass" });
        }
        if self.0.is_empty() {
            return Err(PhysicsError::MissingCollider);
        }
        let volumes = self
            .0
            .iter()
            .map(|collider| collider.shape.volume())
            .collect::<Vec<_>>();
        let total_volume = volumes.iter().flatten().sum::<f32>();
        let all_have_volume = volumes
            .iter()
            .all(|volume| volume.is_some_and(|volume| volume > 0.0));
        #[expect(
            clippy::cast_precision_loss,
            reason = "part_count is an authored collider count for one body, orders of magnitude below f32's 24-bit exact integer range"
        )]
        let part_count = self.0.len() as f32;
        for (collider, volume) in self.0.iter_mut().zip(volumes) {
            collider.mass = Some(if all_have_volume {
                total_mass * volume.expect("all part volumes were checked") / total_volume
            } else {
                total_mass / part_count
            });
        }
        Ok(())
    }
}

impl AsRef<[ColliderConfiguration]> for PhysicsColliderSet {
    fn as_ref(&self) -> &[ColliderConfiguration] {
        &self.0
    }
}

impl AsRef<[PhysicsShapeInstance]> for PhysicsShapeSet {
    fn as_ref(&self) -> &[PhysicsShapeInstance] {
        &self.0
    }
}

impl std::borrow::Borrow<[PhysicsShapeInstance]> for PhysicsShapeSet {
    fn borrow(&self) -> &[PhysicsShapeInstance] {
        &self.0
    }
}

impl FromIterator<PhysicsShapeInstance> for PhysicsShapeSet {
    fn from_iter<T: IntoIterator<Item = PhysicsShapeInstance>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for PhysicsShapeSet {
    type Item = PhysicsShapeInstance;
    type IntoIter = std::vec::IntoIter<PhysicsShapeInstance>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl std::borrow::Borrow<[ColliderConfiguration]> for PhysicsColliderSet {
    fn borrow(&self) -> &[ColliderConfiguration] {
        &self.0
    }
}

impl FromIterator<ColliderConfiguration> for PhysicsColliderSet {
    fn from_iter<T: IntoIterator<Item = ColliderConfiguration>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for PhysicsColliderSet {
    type Item = ColliderConfiguration;
    type IntoIter = std::vec::IntoIter<ColliderConfiguration>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl ColliderConfiguration {
    /// Minimum authored separation between contact-generation and rest
    /// distances, matching `Physics::ColliderConfiguration::ContactOffsetDelta`.
    pub const CONTACT_OFFSET_DELTA: f32 = 0.01;

    /// Validates geometry and material coefficients.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] for invalid shape dimensions or non-finite,
    /// negative material values.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        self.shape.validate()?;
        validate_non_negative(self.friction, "friction")?;
        validate_non_negative(self.restitution, "restitution")?;
        validate_non_negative(self.density, "density")?;
        if let Some(mass) = self.mass {
            validate_positive(mass, "mass")?;
        }
        if !self.rest_offset.is_finite()
            || !self.contact_offset.is_finite()
            || self.contact_offset < 0.0
            || self.contact_offset < self.rest_offset + Self::CONTACT_OFFSET_DELTA
        {
            return Err(PhysicsError::InvalidColliderOffsets {
                rest_offset: self.rest_offset,
                contact_offset: self.contact_offset,
                minimum_delta: Self::CONTACT_OFFSET_DELTA,
            });
        }
        Ok(())
    }

    /// Applies the same correction used by Lumberyard's rest-offset editor
    /// callback, after rejecting non-finite input.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidColliderOffsets`] when `rest_offset` is
    /// non-finite. A finite value that is too close to the contact offset is
    /// clamped rather than rejected.
    pub fn set_rest_offset(&mut self, rest_offset: f32) -> Result<(), PhysicsError> {
        if !rest_offset.is_finite() {
            return Err(PhysicsError::InvalidColliderOffsets {
                rest_offset,
                contact_offset: self.contact_offset,
                minimum_delta: Self::CONTACT_OFFSET_DELTA,
            });
        }
        self.rest_offset = if rest_offset > self.contact_offset - Self::CONTACT_OFFSET_DELTA {
            (self.contact_offset - Self::CONTACT_OFFSET_DELTA).max(0.0)
        } else {
            rest_offset
        };
        Ok(())
    }

    /// Applies the same correction used by Lumberyard's contact-offset editor
    /// callback, after rejecting non-finite input.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidColliderOffsets`] when `contact_offset`
    /// is non-finite or negative. A finite non-negative value that is too close
    /// to the rest offset is clamped rather than rejected.
    pub fn set_contact_offset(&mut self, contact_offset: f32) -> Result<(), PhysicsError> {
        if !contact_offset.is_finite() || contact_offset < 0.0 {
            return Err(PhysicsError::InvalidColliderOffsets {
                rest_offset: self.rest_offset,
                contact_offset,
                minimum_delta: Self::CONTACT_OFFSET_DELTA,
            });
        }
        self.contact_offset = if contact_offset < self.rest_offset + Self::CONTACT_OFFSET_DELTA {
            (self.rest_offset + Self::CONTACT_OFFSET_DELTA).min(1.0)
        } else {
            contact_offset
        };
        Ok(())
    }
}

impl Default for ColliderConfiguration {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere { radius: 0.5 },
            local_pose: PhysicsPose::IDENTITY,
            collision_class: CollisionClass::default(),
            collision_filter: None,
            surface_index: SurfaceIndex::default(),
            surface_pierceability: 0,
            friction: 0.5,
            restitution: 0.0,
            density: 1.0,
            mass: None,
            sensor: false,
            simulated: true,
            in_scene_queries: true,
            buoyancy_enabled: true,
            interacts_with_triggers: true,
            tag: ColliderTag::NONE,
            rest_offset: 0.0,
            contact_offset: 0.02,
        }
    }
}

fn validate_non_negative(value: f32, field: &'static str) -> Result<(), PhysicsError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(PhysicsError::InvalidMaterialCoefficient { field })
    }
}

/// Dynamic rigid-body settings shared by authoring and runtime construction.
#[allow(
    clippy::struct_excessive_bools,
    reason = "these booleans are the directly reflected AzFramework configuration surface"
)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RigidBodyConfiguration {
    pub initial_linear_velocity: Vec3,
    pub initial_angular_velocity: Vec3,
    pub center_of_mass_offset: Vec3,
    pub mass: f32,
    /// Effective material density in mass per unit volume. Cry exposes this as
    /// `pe_simulation_params::density` and recomputes every part mass when it
    /// changes.
    pub density: f32,
    /// Authored body-space principal inertia. `None` derives inertia from the
    /// collider set when `compute_inertia_tensor` is enabled.
    pub principal_inertia: Option<Vec3>,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub damping_model: RigidBodyDampingModel,
    pub buoyancy: RigidBodyBuoyancy,
    pub sleep_min_energy: f32,
    pub sleep_linear_velocity_threshold: f32,
    pub sleep_angular_velocity_threshold: f32,
    pub sleep_duration: f32,
    pub sleep_policy: RigidBodySleepPolicy,
    /// Maximum integration step for this body. Cry exposes this through
    /// `pe_simulation_params::maxTimeStep`; vehicle bodies retain it as the
    /// chassis-contact step limit alongside their wheel-only limit.
    pub maximum_time_step: f32,
    pub max_angular_velocity: f32,
    /// Optional time-step-independent cap on radians rotated per substep.
    pub max_angular_displacement: Option<f32>,
    pub start_asleep: bool,
    pub can_sleep: bool,
    pub interpolate_motion: bool,
    pub gravity_enabled: bool,
    pub simulated: bool,
    pub motion: RigidBodyMotion,
    pub continuous_collision_mode: ContinuousCollisionMode,
    /// Broadphase/contact prediction margin used by continuous modes.
    ///
    /// `RockNRoll` hardcodes this independently from the reflected per-body
    /// distance factor and sphere radius.
    pub continuous_prediction_distance: f32,
    /// Multiplier used by modes 1/2 when retaining normal velocity after a
    /// projected hit. The native descriptor default is `0.3`.
    pub continuous_distance_factor: f32,
    /// Reserved reflected descriptor scalar retained for descriptor parity.
    /// Runtime code constructs, copies, and serializes this value but does not
    /// otherwise consume it.
    pub continuous_sphere_radius: f32,
    pub ccd_min_advance_coefficient: f32,
    pub ccd_friction_enabled: bool,
    pub compute_center_of_mass: bool,
    pub compute_inertia_tensor: bool,
    pub compute_mass: bool,
    pub include_all_shapes_in_mass_calculation: bool,
    pub independent: bool,
}

impl RigidBodyConfiguration {
    /// Validates finite rigid-body values before solver construction.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] when mass, damping, sleep, velocity, or `CCD`
    /// values cannot be represented by a solver.
    pub fn validate(self) -> Result<(), PhysicsError> {
        for (value, field) in [
            (self.mass, "mass"),
            (self.density, "density"),
            (self.linear_damping, "linear_damping"),
            (self.angular_damping, "angular_damping"),
            (self.sleep_min_energy, "sleep_min_energy"),
            (
                self.sleep_linear_velocity_threshold,
                "sleep_linear_velocity_threshold",
            ),
            (
                self.sleep_angular_velocity_threshold,
                "sleep_angular_velocity_threshold",
            ),
            (self.sleep_duration, "sleep_duration"),
            (self.maximum_time_step, "maximum_time_step"),
            (self.max_angular_velocity, "max_angular_velocity"),
            (
                self.ccd_min_advance_coefficient,
                "ccd_min_advance_coefficient",
            ),
            (
                self.continuous_prediction_distance,
                "continuous_prediction_distance",
            ),
            (
                self.continuous_distance_factor,
                "continuous_distance_factor",
            ),
            (self.continuous_sphere_radius, "continuous_sphere_radius"),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidRigidBodyScalar { field });
            }
        }
        if self.mass == 0.0 {
            return Err(PhysicsError::InvalidRigidBodyScalar { field: "mass" });
        }
        if self.maximum_time_step == 0.0 {
            return Err(PhysicsError::InvalidRigidBodyScalar {
                field: "maximum_time_step",
            });
        }
        if self
            .principal_inertia
            .is_some_and(|inertia| !inertia.is_finite() || !inertia.cmpgt(Vec3::ZERO).all())
        {
            return Err(PhysicsError::InvalidRigidBodyScalar {
                field: "principal_inertia",
            });
        }
        if self
            .max_angular_displacement
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err(PhysicsError::InvalidRigidBodyScalar {
                field: "max_angular_displacement",
            });
        }
        self.damping_model.validate()?;
        self.buoyancy.validate()?;
        Ok(())
    }
}

impl Default for RigidBodyConfiguration {
    fn default() -> Self {
        Self {
            initial_linear_velocity: Vec3::ZERO,
            initial_angular_velocity: Vec3::ZERO,
            center_of_mass_offset: Vec3::ZERO,
            mass: 1.0,
            density: 1.0,
            principal_inertia: None,
            linear_damping: 0.05,
            angular_damping: 0.15,
            damping_model: RigidBodyDampingModel::Solver,
            buoyancy: RigidBodyBuoyancy::default(),
            sleep_min_energy: 0.5,
            sleep_linear_velocity_threshold: 0.4,
            sleep_angular_velocity_threshold: 0.5,
            sleep_duration: 2.0,
            sleep_policy: RigidBodySleepPolicy::SolverVelocityThresholds,
            maximum_time_step: 0.02,
            max_angular_velocity: 100.0,
            max_angular_displacement: None,
            start_asleep: false,
            can_sleep: true,
            interpolate_motion: false,
            gravity_enabled: true,
            simulated: true,
            motion: RigidBodyMotion::Dynamic,
            continuous_collision_mode: ContinuousCollisionMode::Disabled,
            continuous_prediction_distance: 0.05,
            continuous_distance_factor: 0.3,
            continuous_sphere_radius: 1.0,
            ccd_min_advance_coefficient: 0.15,
            ccd_friction_enabled: false,
            compute_center_of_mass: true,
            compute_inertia_tensor: true,
            compute_mass: true,
            include_all_shapes_in_mass_calculation: false,
            independent: false,
        }
    }
}

/// Shape-driven kinematic character used by `RockNRoll`'s controller component.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct CharacterBodyConfiguration {
    pub up_direction: Vec3,
    pub max_slope: f32,
    pub contact_distance: f32,
    pub solver_max_iterations: u32,
    pub asynchronous: bool,
    pub mass: f32,
}

impl CharacterBodyConfiguration {
    /// Validates the controller's up axis, slope limit, and solver settings.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidCharacterConfiguration`] naming the first
    /// offending field: `up_direction` when it is non-finite or not unit-length,
    /// `max_slope` when it is non-finite or outside zero through pi radians,
    /// `contact_distance` when it is non-finite or negative,
    /// `solver_max_iterations` when it is zero, and `mass` when it is
    /// non-finite or not greater than zero.
    pub fn validate(self) -> Result<(), PhysicsError> {
        if !self.up_direction.is_finite()
            || (self.up_direction.length_squared() - 1.0).abs() > 1.0e-4
        {
            return Err(PhysicsError::InvalidCharacterConfiguration {
                field: "up_direction",
            });
        }
        if !self.max_slope.is_finite() || !(0.0..=core::f32::consts::PI).contains(&self.max_slope) {
            return Err(PhysicsError::InvalidCharacterConfiguration { field: "max_slope" });
        }
        if !self.contact_distance.is_finite() || self.contact_distance < 0.0 {
            return Err(PhysicsError::InvalidCharacterConfiguration {
                field: "contact_distance",
            });
        }
        if self.solver_max_iterations == 0 {
            return Err(PhysicsError::InvalidCharacterConfiguration {
                field: "solver_max_iterations",
            });
        }
        if !self.mass.is_finite() || self.mass <= 0.0 {
            return Err(PhysicsError::InvalidCharacterConfiguration { field: "mass" });
        }
        Ok(())
    }
}

impl Default for CharacterBodyConfiguration {
    fn default() -> Self {
        Self {
            up_direction: Vec3::Y,
            max_slope: core::f32::consts::FRAC_PI_2,
            contact_distance: 0.01,
            solver_max_iterations: 10,
            asynchronous: true,
            mass: 80.0,
        }
    }
}

/// Physical behavior of a body. Living bodies generate their primary
/// cylinder/capsule directly from [`LivingBodyConfiguration`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    Static {
        terrain: bool,
    },
    Rigid(RigidBodyConfiguration),
    WheeledVehicle(crate::WheeledVehicleConfiguration),
    /// One rigid link in a reduced-coordinate or impulse articulation.
    Articulated(RigidBodyConfiguration),
    Particle(crate::ParticleBodyConfiguration),
    Rope(crate::RopeBodyConfiguration),
    Soft(crate::SoftBodyConfiguration),
    /// Linked triangular deformable used by `RockNRoll`. This is distinct from
    /// the `Soft` product above.
    LinkedSoft(crate::LinkedSoftBodyConfiguration),
    Living(LivingBodyConfiguration),
    Character(CharacterBodyConfiguration),
    /// A non-contact body that participates only in spatial queries. This is
    /// the solver-neutral product for gameplay hit volumes and similar
    /// registries that Cry/AOI keeps outside ordinary rigid-body simulation.
    Query(QueryBodyConfiguration),
    Area,
    FluidArea(FluidAreaConfiguration),
}

/// Classification and motion of a query-only body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub struct QueryBodyConfiguration {
    pub physical_entity_type: PhysicalEntityType,
    pub query_type: PhysicalEntityTypes,
    pub dynamic: bool,
}

impl QueryBodyConfiguration {
    /// Moving living hit-volume registration used by area-of-interest physics
    /// traits. Every solver adapter forces these colliders to sensors.
    #[must_use]
    pub const fn living() -> Self {
        Self {
            physical_entity_type: PhysicalEntityType::Living,
            query_type: PhysicalEntityTypes::LIVING,
            dynamic: true,
        }
    }

    fn validate(self) -> Result<(), PhysicsError> {
        if self.query_type == PhysicalEntityTypes::NONE {
            return Err(PhysicsError::InvalidSpatialQueryResultCount);
        }
        match self.physical_entity_type {
            PhysicalEntityType::Static
            | PhysicalEntityType::Rigid
            | PhysicalEntityType::Living
            | PhysicalEntityType::Area => Ok(()),
            unsupported => Err(PhysicsError::UnsupportedEntityType(unsupported)),
        }
    }
}

impl BodyKind {
    #[must_use]
    pub const fn physical_entity_type(&self) -> PhysicalEntityType {
        match self {
            Self::Static { .. } => PhysicalEntityType::Static,
            Self::Rigid(_) => PhysicalEntityType::Rigid,
            Self::WheeledVehicle(_) => PhysicalEntityType::WheeledVehicle,
            Self::Articulated(_) => PhysicalEntityType::Articulated,
            Self::Particle(_) => PhysicalEntityType::Particle,
            Self::Rope(_) => PhysicalEntityType::Rope,
            Self::Soft(_) | Self::LinkedSoft(_) => PhysicalEntityType::Soft,
            // Cry has no separate controller entity type: a character controller
            // registers as a living entity.
            Self::Living(_) | Self::Character(_) => PhysicalEntityType::Living,
            Self::Query(config) => config.physical_entity_type,
            // A fluid/buoyancy volume is an area entity carrying medium data.
            Self::Area | Self::FluidArea(_) => PhysicalEntityType::Area,
        }
    }

    #[must_use]
    pub const fn query_type(&self) -> PhysicalEntityTypes {
        match self {
            Self::Static { terrain: true } => PhysicalEntityTypes::TERRAIN,
            Self::Static { terrain: false } => PhysicalEntityTypes::STATIC,
            Self::Rigid(config) if config.independent => PhysicalEntityTypes::INDEPENDENT,
            Self::Articulated(config) if config.independent => PhysicalEntityTypes::INDEPENDENT,
            // A wheeled vehicle is queried as an ordinary dynamic body; it has no
            // independent-simulation variant of its own. Solver-coupled rigid and
            // articulated bodies answer with that same dynamic query type, so the
            // three share one arm. Neither rigid nor wheeled-vehicle values can
            // match the articulated guard above, so the order is unchanged for them.
            Self::Rigid(_) | Self::WheeledVehicle(_) | Self::Articulated(_) => {
                PhysicalEntityTypes::DYNAMIC
            }
            // Every deformable family is simulated independently of the rigid
            // solver island, so they share the independent query type.
            Self::Particle(_) | Self::Rope(_) | Self::Soft(_) | Self::LinkedSoft(_) => {
                PhysicalEntityTypes::INDEPENDENT
            }
            // Cry has no separate controller entity type: a character controller
            // is queried as a living entity.
            Self::Living(_) | Self::Character(_) => PhysicalEntityTypes::LIVING,
            Self::Query(config) => config.query_type,
            // Areas are never returned by entity-type queries, medium or not.
            Self::Area | Self::FluidArea(_) => PhysicalEntityTypes::NONE,
        }
    }
}

/// Complete backend-neutral body construction request.
#[derive(
    bevy_ecs::component::Component, Debug, Clone, PartialEq, Serialize, Deserialize, Reflect,
)]
pub struct BodyDescriptor {
    pub entity_id: Option<PhysicsEntityId>,
    pub pose: PhysicsPose,
    pub kind: BodyKind,
    /// Extra colliders. A living body's primary collider is always generated
    /// from its dimensions before these are added.
    pub colliders: Vec<ColliderConfiguration>,
}

/// Last materialization error for an authored [`BodyDescriptor`].
#[derive(bevy_ecs::component::Component, Debug, Clone, PartialEq)]
pub struct PhysicsBodyError(pub PhysicsError);

impl BodyDescriptor {
    /// Validates the complete construction request.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] for a missing required collider or any invalid
    /// nested body, living-body, collider, or material configuration.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if !self.colliders.is_empty() {
            match &self.kind {
                BodyKind::Rope(_) => {
                    return Err(PhysicsError::InvalidRopeConfiguration {
                        field: "external colliders",
                    });
                }
                BodyKind::Soft(_) => {
                    return Err(PhysicsError::InvalidSoftBodyConfiguration {
                        field: "external colliders",
                    });
                }
                BodyKind::LinkedSoft(_) => {
                    return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                        field: "external colliders",
                    });
                }
                _ => {}
            }
        }
        match &self.kind {
            BodyKind::Static { .. }
            | BodyKind::Rigid(_)
            | BodyKind::WheeledVehicle(_)
            | BodyKind::Articulated(_)
            | BodyKind::Character(_)
            | BodyKind::Query(_)
            | BodyKind::Area
            | BodyKind::FluidArea(_)
                if self.colliders.is_empty() =>
            {
                return Err(PhysicsError::MissingCollider);
            }
            BodyKind::Rigid(config) | BodyKind::Articulated(config) => config.validate()?,
            BodyKind::WheeledVehicle(config) => config.validate()?,
            BodyKind::Particle(config) => config.validate()?,
            BodyKind::Rope(config) => config.validate()?,
            BodyKind::Soft(config) => config.validate()?,
            BodyKind::LinkedSoft(config) => config.validate()?,
            BodyKind::Living(config) => config.validate()?,
            BodyKind::Character(config) => config.validate()?,
            BodyKind::Query(config) => config.validate()?,
            BodyKind::FluidArea(configuration) => configuration.validate()?,
            BodyKind::Static { .. } | BodyKind::Area => {}
        }
        for collider in &self.colliders {
            collider.validate()?;
        }
        Ok(())
    }
}

/// Current solver-independent body state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct BodyStatus {
    pub pose: PhysicsPose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    /// Net linear acceleration observed during the most recent simulation
    /// step. Sleeping bodies report zero, matching `pe_status_dynamics`.
    pub linear_acceleration: Vec3,
    /// Net angular acceleration observed during the most recent simulation
    /// step. Sleeping bodies report zero, matching `pe_status_dynamics`.
    pub angular_acceleration: Vec3,
    pub mass: f32,
    /// Effective body mass divided by the summed collider volume.
    pub density: f32,
    pub kinetic_energy: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Cry energy threshold used by the body's sleep policy.
    pub sleep_min_energy: f32,
    pub buoyancy: RigidBodyBuoyancy,
    pub buoyancy_status: BuoyancyStatus,
    pub simulation_class: SimulationClass,
    pub awake: bool,
    pub kinematic: bool,
    pub simulated: bool,
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests lock exact reflected default values"
)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn rigid_defaults_match_azframework_defaults() {
        let config = RigidBodyConfiguration::default();
        assert_eq!(config.mass, 1.0);
        assert_eq!(config.linear_damping, 0.05);
        assert_eq!(config.angular_damping, 0.15);
        assert_eq!(config.sleep_min_energy, 0.5);
        assert_eq!(config.max_angular_velocity, 100.0);
        assert_eq!(config.continuous_prediction_distance, 0.05);
        assert_eq!(config.continuous_distance_factor, 0.3);
        assert_eq!(config.continuous_sphere_radius, 1.0);
        assert!(config.gravity_enabled);
        assert!(config.compute_mass);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn continuous_hit_projection_matches_native_scalar_cleanup() {
        let projection = continuous_hit_projection(0.25, 2.0, 1.5, 0.3);
        assert_eq!(projection.pose_fraction, 0.255);
        assert!((projection.normal_velocity_retained_fraction - 0.22).abs() < 1.0e-6);
    }

    #[test]
    fn continuous_hit_projection_preserves_native_zero_displacement_branch() {
        let projection = continuous_hit_projection(0.25, 0.0, 0.0, 0.3);
        assert_eq!(projection.pose_fraction, 1.0);
        assert_eq!(projection.normal_velocity_retained_fraction, 0.0);
    }

    #[test]
    fn physical_entity_masks_preserve_reflected_values() {
        assert_eq!(PhysicalEntityTypes::STATIC.bits(), 1);
        assert_eq!(PhysicalEntityTypes::DYNAMIC.bits(), 2);
        assert_eq!(PhysicalEntityTypes::LIVING.bits(), 4);
        assert_eq!(PhysicalEntityTypes::INDEPENDENT.bits(), 8);
        assert_eq!(PhysicalEntityTypes::TERRAIN.bits(), 16);
        assert_eq!(PhysicalEntityTypes::ALL.bits(), 31);
    }

    #[test]
    fn collider_validation_rejects_invalid_geometry() {
        let collider = ColliderConfiguration {
            shape: ColliderShape::Sphere { radius: 0.0 },
            ..Default::default()
        };
        assert_eq!(
            collider.validate(),
            Err(PhysicsError::InvalidColliderScalar { field: "radius" })
        );
    }

    #[test]
    fn collider_defaults_match_lumberyard_runtime_configuration() {
        let collider = ColliderConfiguration::default();
        assert!(collider.simulated);
        assert!(collider.in_scene_queries);
        assert!(collider.buoyancy_enabled);
        assert_eq!(collider.tag, ColliderTag::NONE);
        assert_eq!(collider.rest_offset, 0.0);
        assert_eq!(collider.contact_offset, 0.02);
        assert!(collider.validate().is_ok());
    }

    #[test]
    fn collider_offset_callbacks_preserve_native_delta_corrections() {
        let mut collider = ColliderConfiguration::default();
        collider.set_rest_offset(0.02).unwrap();
        assert_eq!(collider.rest_offset, 0.01);

        collider.set_contact_offset(0.0).unwrap();
        assert_eq!(collider.contact_offset, 0.02);
        assert!(collider.validate().is_ok());

        collider.rest_offset = -0.25;
        collider.contact_offset = 0.0;
        assert!(collider.validate().is_ok());
    }

    #[test]
    fn collider_validation_rejects_contact_offset_below_rest_delta() {
        let collider = ColliderConfiguration {
            rest_offset: 0.02,
            contact_offset: 0.02,
            ..ColliderConfiguration::default()
        };
        assert_eq!(
            collider.validate(),
            Err(PhysicsError::InvalidColliderOffsets {
                rest_offset: 0.02,
                contact_offset: 0.02,
                minimum_delta: ColliderConfiguration::CONTACT_OFFSET_DELTA,
            })
        );
    }

    #[test]
    fn physics_pose_default_is_identity() {
        assert_eq!(
            PhysicsPose::default(),
            PhysicsPose {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            }
        );
    }
}
