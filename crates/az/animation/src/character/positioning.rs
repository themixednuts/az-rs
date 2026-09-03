//! Renderer- and physics-independent Cry Mannequin position adjustment.
//!
//! The types in this module are the reusable core behind Cry's
//! `PositionAdjust`, `PositionAdjustAnimPos`,
//! `PositionAdjustAnimPosContinuously`, and `PositionAdjustTargetLocator`
//! procedural clips. World transforms, animation-root locations, and sweep
//! results are supplied by the host; the timing and correction math stays in
//! the animation runtime.

use std::{borrow::Borrow, hash::Hash};

use bevy_math::{Mat3, Quat, Vec3};

use crate::mannequin::AnimationLane;

/// A rigid transform in Cry's `QuatT` composition order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionTransform {
    pub rotation: Quat,
    pub translation: Vec3,
}

impl PositionTransform {
    pub const IDENTITY: Self = Self {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
    };

    #[must_use]
    pub const fn new(translation: Vec3, rotation: Quat) -> Self {
        Self {
            rotation,
            translation,
        }
    }

    #[must_use]
    pub fn inverse(self) -> Self {
        let rotation = self.rotation.inverse();
        Self::new(rotation * -self.translation, rotation)
    }

    /// Compose `rhs` after `self`, matching Cry's `QuatT * QuatT`.
    #[must_use]
    pub fn compose(self, rhs: impl Borrow<Self>) -> Self {
        let rhs = rhs.borrow();
        Self::new(
            self.translation + self.rotation * rhs.translation,
            self.rotation * rhs.rotation,
        )
    }
}

impl Default for PositionTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl From<(Vec3, Quat)> for PositionTransform {
    fn from((translation, rotation): (Vec3, Quat)) -> Self {
        Self::new(translation, rotation)
    }
}

/// One incremental correction emitted by [`PositionAdjuster::update`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionAdjustment {
    pub rotation: Quat,
    pub translation: Vec3,
}

impl PositionAdjustment {
    pub const IDENTITY: Self = Self {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
    };

    #[must_use]
    pub fn is_identity(self) -> bool {
        self.translation == Vec3::ZERO && self.rotation == Quat::IDENTITY
    }
}

impl Default for PositionAdjustment {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Host capability used to apply Cry positioning without retaining a backend
/// reference between frames.
pub trait PositionAdjustmentSink {
    fn apply_position_adjustment(&mut self, adjustment: PositionAdjustment);
}

/// Host values required by the stock `PositionAdjust` procedural clip.
pub trait PositionAdjustRuntime: PositionAdjustmentSink {
    fn entity_position_transform(&self) -> PositionTransform;

    fn action_position_target(&self) -> Option<PositionTransform>;

    fn set_top_animation_full_root_priority(&mut self, lane: AnimationLane);
}

/// Installed state for the stock `PositionAdjust` procedural clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionAdjustState {
    adjuster: PositionAdjuster,
}

/// Action parameters consumed by the `ProceduralAlignment` clip.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool mirrors a distinct authored ProceduralAlignment parameter flag"
)]
pub struct ProceduralAlignmentRequest {
    pub ignore_position: bool,
    pub ignore_rotation: bool,
    pub align_z_axis: bool,
    pub face_position: bool,
    pub offset: Vec3,
}

/// Optional action-controller parameters named `TargetPosition` and
/// `TargetOrientation` by the native implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProceduralAlignmentTarget {
    pub position: Option<Vec3>,
    pub orientation: Option<Quat>,
}

/// Concrete pose-modifier values installed by `ProceduralAlignment::OnEnter`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProceduralAlignmentModifier {
    pub blend_time: f32,
    pub translation: Option<Vec3>,
    pub rotation: Option<Quat>,
}

/// Host capabilities required by the procedural-alignment clip.
pub trait ProceduralAlignmentRuntime {
    fn entity_alignment_transform(&self) -> PositionTransform;

    fn procedural_alignment_target(&self) -> ProceduralAlignmentTarget;

    fn set_procedural_alignment(&mut self, modifier: Option<ProceduralAlignmentModifier>);
}

/// Installed `ProceduralAlignment` state. Cry retains a pose modifier for the
/// duration of the clip and only clears its active bit on exit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProceduralAlignmentState {
    active: bool,
}

impl ProceduralAlignmentState {
    #[must_use]
    pub fn enter(
        runtime: &mut impl ProceduralAlignmentRuntime,
        blend_time: f32,
        request: ProceduralAlignmentRequest,
    ) -> Self {
        let modifier = configure_procedural_alignment(
            runtime.entity_alignment_transform(),
            runtime.procedural_alignment_target(),
            blend_time,
            request,
        );
        let active = modifier.is_some();
        runtime.set_procedural_alignment(modifier);
        Self { active }
    }

    pub fn exit(self, runtime: &mut impl ProceduralAlignmentRuntime) {
        if self.active {
            runtime.set_procedural_alignment(None);
        }
    }

    #[must_use]
    pub const fn is_active(self) -> bool {
        self.active
    }
}

/// Configures the pose modifier installed by `ProceduralAlignment::OnEnter`.
///
/// Position offset is expressed in target-orientation space. Rotation is the
/// target (or Y-forward look-at) orientation relative to the entity. When
/// `align_z_axis` is false the vertical translation is explicitly discarded.
#[must_use]
pub fn configure_procedural_alignment(
    entity: PositionTransform,
    target: ProceduralAlignmentTarget,
    blend_time: f32,
    request: ProceduralAlignmentRequest,
) -> Option<ProceduralAlignmentModifier> {
    if target.position.is_none() && target.orientation.is_none() {
        return None;
    }

    let target_orientation = target.orientation.unwrap_or(Quat::IDENTITY).normalize();
    let translation = (!request.ignore_position)
        .then(|| {
            target.position.map(|position| {
                let mut delta = position + target_orientation * request.offset - entity.translation;
                if !request.align_z_axis {
                    delta.z = 0.0;
                }
                delta
            })
        })
        .flatten();

    let desired_rotation = if request.ignore_rotation {
        None
    } else if request.face_position {
        target
            .position
            .and_then(|position| look_at_y_positive(entity.translation, position))
    } else {
        target.orientation
    };
    let rotation = desired_rotation
        .map(|desired| (desired.normalize() * entity.rotation.inverse()).normalize());

    (translation.is_some() || rotation.is_some()).then_some(ProceduralAlignmentModifier {
        blend_time,
        translation,
        rotation,
    })
}

/// Lumberyard `AZ::Transform::CreateLookAt(..., Axis::YPositive)` with its
/// Z-up degeneracy rule.
#[must_use]
pub fn look_at_y_positive(from: Vec3, to: Vec3) -> Option<Quat> {
    let mut forward = to - from;
    if forward.length_squared() == 0.0 {
        return None;
    }
    forward = forward.normalize();

    let mut up = Vec3::Z;
    if forward.dot(up).abs() > 1.0 - 0.001 {
        up = forward.cross(Vec3::Y);
    }
    let right = forward.cross(up).normalize();
    let up = right.cross(forward).normalize();
    Some(Quat::from_mat3(&Mat3::from_cols(right, forward, up)).normalize())
}

impl PositionAdjustState {
    #[must_use]
    pub fn enter(
        runtime: &mut impl PositionAdjustRuntime,
        lane: AnimationLane,
        blend_time: f32,
        ideal_offset: Vec3,
        ideal_yaw_radians: f32,
        ignore_position: bool,
        ignore_rotation: bool,
    ) -> Self {
        let mut adjuster = PositionAdjuster::new(blend_time);
        runtime.set_top_animation_full_root_priority(lane);
        let entity = runtime.entity_position_transform();
        if let Some(target) = runtime.action_position_target() {
            configure_position_adjust(
                &mut adjuster,
                entity,
                target,
                ideal_offset,
                ideal_yaw_radians,
                ignore_position,
                ignore_rotation,
            );
        } else {
            adjuster.set_target_location(entity);
            adjuster.invalidate();
        }
        Self { adjuster }
    }

    pub fn update(&mut self, runtime: &mut impl PositionAdjustRuntime, time_passed: f32) {
        self.adjuster.update_and_apply(runtime, time_passed);
    }

    #[must_use]
    pub const fn adjuster(&self) -> &PositionAdjuster {
        &self.adjuster
    }
}

/// Cry's `SPositionAdjuster`, including its zero-time edge and incremental
/// quaternion interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionAdjuster {
    target_location: PositionTransform,
    delta: Vec3,
    delta_rotation: Quat,
    last_time: f32,
    target_time: f32,
    invalid: bool,
}

impl Default for PositionAdjuster {
    fn default() -> Self {
        Self {
            target_location: PositionTransform::IDENTITY,
            delta: Vec3::ZERO,
            delta_rotation: Quat::IDENTITY,
            last_time: 0.0,
            target_time: 0.0,
            invalid: true,
        }
    }
}

impl PositionAdjuster {
    #[must_use]
    pub fn new(blend_time: f32) -> Self {
        Self {
            target_time: blend_time.max(0.0),
            invalid: false,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn target_location(&self) -> PositionTransform {
        self.target_location
    }

    pub fn set_target_location(&mut self, target_location: impl Into<PositionTransform>) {
        self.target_location = target_location.into();
    }

    #[must_use]
    pub const fn delta(&self) -> Vec3 {
        self.delta
    }

    pub const fn set_delta(&mut self, delta: Vec3) {
        self.delta = delta;
    }

    #[must_use]
    pub const fn delta_rotation(&self) -> Quat {
        self.delta_rotation
    }

    pub fn set_delta_rotation(&mut self, delta_rotation: Quat) {
        self.delta_rotation = delta_rotation.normalize();
    }

    #[must_use]
    pub const fn elapsed(&self) -> f32 {
        self.last_time
    }

    #[must_use]
    pub const fn remaining(&self) -> f32 {
        self.target_time - self.last_time
    }

    #[must_use]
    pub const fn is_valid(&self) -> bool {
        !self.invalid
    }

    pub const fn invalidate(&mut self) {
        self.invalid = true;
    }

    /// Restarts interpolation from the current pose using the unconsumed blend
    /// duration. This is Cry's `targetTime -= lastTime; lastTime = 0` retarget.
    pub const fn retarget_remaining(&mut self) {
        self.target_time = self.remaining().max(0.0);
        self.last_time = 0.0;
        self.invalid = false;
    }

    /// Emit this frame's incremental correction.
    #[must_use]
    pub fn update(&mut self, time_passed: f32) -> Option<PositionAdjustment> {
        if self.invalid {
            return None;
        }

        let new_time = (self.last_time + time_passed.max(0.0)).min(self.target_time);
        let delta_time = new_time - self.last_time;
        let adjustment = if delta_time > 0.0 {
            let inverse_target_time = self.target_time.recip();
            let total_rotation = Quat::IDENTITY
                .slerp(self.delta_rotation, new_time * inverse_target_time)
                .normalize();
            let previous_rotation = Quat::IDENTITY
                .slerp(self.delta_rotation, self.last_time * inverse_target_time)
                .normalize();
            self.last_time = new_time;
            PositionAdjustment {
                rotation: previous_rotation.inverse() * total_rotation,
                translation: self.delta * (delta_time * inverse_target_time),
            }
        } else if self.target_time == 0.0 {
            self.invalid = true;
            PositionAdjustment {
                rotation: self.delta_rotation,
                translation: self.delta,
            }
        } else {
            PositionAdjustment::IDENTITY
        };

        Some(adjustment)
    }

    pub fn update_and_apply(&mut self, sink: &mut impl PositionAdjustmentSink, time_passed: f32) {
        if let Some(adjustment) = self.update(time_passed) {
            sink.apply_position_adjustment(adjustment);
        }
    }
}

/// Initialize the shipping `PositionAdjust` correction from the action's
/// `TargetPos` value and the authored ideal offset/yaw.
pub fn configure_position_adjust(
    adjuster: &mut PositionAdjuster,
    entity: impl Borrow<PositionTransform>,
    target: impl Borrow<PositionTransform>,
    ideal_offset: Vec3,
    ideal_yaw_radians: f32,
    ignore_position: bool,
    ignore_rotation: bool,
) {
    let entity = entity.borrow();
    let target = target.borrow();
    adjuster.set_target_location(*target);
    adjuster.set_delta(if ignore_position {
        Vec3::ZERO
    } else {
        let actual_offset = target.translation - entity.translation;
        let ideal_offset = target.rotation * ideal_offset;
        actual_offset - ideal_offset
    });

    let ideal_rotation = Quat::from_rotation_z(if ignore_rotation {
        0.0
    } else {
        ideal_yaw_radians
    });
    let actual_rotation = target.rotation * entity.rotation.inverse();
    adjuster.set_delta_rotation(actual_rotation * ideal_rotation.inverse());
}

/// Recompute the correction used by `PositionAdjustAnimPos` after its target
/// or top animation changes.
pub fn configure_animation_position_adjust(
    adjuster: &mut PositionAdjuster,
    entity: impl Borrow<PositionTransform>,
    animation_start: impl Borrow<PositionTransform>,
    ignore_position: bool,
    ignore_rotation: bool,
) {
    let entity = entity.borrow();
    let required = adjuster.target_location().compose(animation_start.borrow());
    if !ignore_position {
        adjuster.set_delta(required.translation - entity.translation);
    }
    if !ignore_rotation {
        adjuster.set_delta_rotation(entity.rotation.inverse() * required.rotation);
    }
    adjuster.retarget_remaining();
}

/// Recompute the stock continuous variant, including the expected movement
/// already queued by the animated-character controller.
pub fn configure_continuous_animation_position_adjust(
    adjuster: &mut PositionAdjuster,
    entity: impl Borrow<PositionTransform>,
    expected_movement: Vec3,
    animation_start: impl Borrow<PositionTransform>,
) {
    let entity = entity.borrow();
    let animation_start = animation_start.borrow();
    let expected_entity =
        PositionTransform::new(entity.translation + expected_movement, entity.rotation);
    let actual_offset = adjuster.target_location().translation - expected_entity.translation;
    let ideal_offset = adjuster.target_location().rotation * -animation_start.translation;
    adjuster.set_delta(actual_offset - ideal_offset);
    adjuster.set_delta_rotation(
        expected_entity.rotation.inverse()
            * adjuster.target_location().rotation
            * animation_start.rotation,
    );
    adjuster.retarget_remaining();
}

/// Parameters for the optional enslaved-scope collision correction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionAdjustParameters {
    pub max_adjustment_speed: f32,
    pub height_offset: f32,
    pub height_multiplier: f32,
    pub width_multiplier: f32,
}

impl Default for CollisionAdjustParameters {
    fn default() -> Self {
        Self {
            max_adjustment_speed: 3.0,
            height_offset: 0.2,
            height_multiplier: 0.4,
            width_multiplier: 0.5,
        }
    }
}

/// Four axis sweep distances returned by the physics backend. A non-hit is
/// represented by a value outside `(0, half_extent)`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CollisionSweepDistances {
    pub x_forward: f32,
    pub x_back: f32,
    pub y_forward: f32,
    pub y_back: f32,
}

/// Apply Cry's asymmetric-axis collision rule and per-frame speed clamp.
#[must_use]
pub fn collision_adjustment(
    half_extents: Vec3,
    distances: impl Borrow<CollisionSweepDistances>,
    max_adjustment_speed: f32,
    time_passed: f32,
) -> Vec3 {
    let distances = distances.borrow();
    let x_forward = distances.x_forward > 0.0 && distances.x_forward < half_extents.x;
    let x_back = distances.x_back > 0.0 && distances.x_back < half_extents.x;
    let y_forward = distances.y_forward > 0.0 && distances.y_forward < half_extents.y;
    let y_back = distances.y_back > 0.0 && distances.y_back < half_extents.y;
    let max_move = max_adjustment_speed.max(0.0) * time_passed.max(0.0);
    let mut adjustment = Vec3::ZERO;

    if x_forward != x_back {
        let movement = if x_forward {
            distances.x_forward - half_extents.x
        } else {
            half_extents.x - distances.x_back
        };
        adjustment.x = movement.clamp(-max_move, max_move);
    }
    if y_forward != y_back {
        let movement = if y_forward {
            distances.y_forward - half_extents.y
        } else {
            half_extents.y - distances.y_back
        };
        adjustment.y = movement.clamp(-max_move, max_move);
    }
    adjustment
}

/// Per-scope state owned by Cry's shared `AdjustPosContext`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScopePositionAdjust<ScopeId> {
    pub scope: ScopeId,
    pub adjuster: PositionAdjuster,
    pub ignore_position: bool,
    pub ignore_rotation: bool,
}

/// Multi-scope position adjustment context. Scope IDs and all host state remain
/// backend-owned; this value contains only Cry's deterministic runtime state.
#[derive(Debug, Clone, Default)]
pub struct PositionAdjustContext<ScopeId> {
    scopes: Vec<ScopePositionAdjust<ScopeId>>,
}

impl<ScopeId> PositionAdjustContext<ScopeId>
where
    ScopeId: Copy + Eq + Hash,
{
    #[must_use]
    pub fn scopes(&self) -> &[ScopePositionAdjust<ScopeId>] {
        &self.scopes
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the argument set CryEngine's procedural positioning clip is entered with"
    )]
    pub fn start(
        &mut self,
        scope: ScopeId,
        target: impl Into<PositionTransform>,
        entity: impl Borrow<PositionTransform>,
        animation_start: impl Borrow<PositionTransform>,
        blend_time: f32,
        ignore_position: bool,
        ignore_rotation: bool,
    ) {
        let entry = self.entry(scope);
        entry.adjuster = PositionAdjuster::new(blend_time);
        entry.adjuster.set_target_location(target);
        entry.ignore_position = ignore_position;
        entry.ignore_rotation = ignore_rotation;
        configure_animation_position_adjust(
            &mut entry.adjuster,
            entity,
            animation_start,
            ignore_position,
            ignore_rotation,
        );
    }

    pub fn retarget(
        &mut self,
        scope: ScopeId,
        target: impl Into<PositionTransform>,
        entity: impl Borrow<PositionTransform>,
        animation_start: impl Borrow<PositionTransform>,
    ) -> bool {
        let Some(entry) = self.scopes.iter_mut().find(|entry| entry.scope == scope) else {
            return false;
        };
        entry.adjuster.set_target_location(target);
        configure_animation_position_adjust(
            &mut entry.adjuster,
            entity,
            animation_start,
            entry.ignore_position,
            entry.ignore_rotation,
        );
        true
    }

    pub fn adjust_targets(&mut self, adjustment: Vec3) {
        for entry in &mut self.scopes {
            let mut target = entry.adjuster.target_location();
            target.translation += adjustment;
            entry.adjuster.set_target_location(target);
        }
    }

    pub fn end(&mut self, scope: ScopeId) -> bool {
        let Some(index) = self.scopes.iter().position(|entry| entry.scope == scope) else {
            return false;
        };
        self.scopes.remove(index);
        true
    }

    fn entry(&mut self, scope: ScopeId) -> &mut ScopePositionAdjust<ScopeId> {
        if let Some(index) = self.scopes.iter().position(|entry| entry.scope == scope) {
            return &mut self.scopes[index];
        }
        self.scopes.push(ScopePositionAdjust {
            scope,
            adjuster: PositionAdjuster::default(),
            ignore_position: false,
            ignore_rotation: false,
        });
        self.scopes
            .last_mut()
            .expect("a position-adjust scope was just appended")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjuster_splits_translation_and_rotation_over_blend_time() {
        let mut adjuster = PositionAdjuster::new(2.0);
        adjuster.set_delta(Vec3::new(4.0, 0.0, 0.0));
        adjuster.set_delta_rotation(Quat::from_rotation_z(std::f32::consts::PI));

        let first = adjuster.update(0.5).expect("active adjustment");
        let second = adjuster.update(1.5).expect("active adjustment");
        assert!((first.translation.x - 1.0).abs() < 0.0001);
        assert!((second.translation.x - 3.0).abs() < 0.0001);
        assert!(
            ((first.rotation * second.rotation).to_axis_angle().1 - std::f32::consts::PI).abs()
                < 0.0001
        );
    }

    #[test]
    fn zero_blend_applies_once() {
        let mut adjuster = PositionAdjuster::new(0.0);
        adjuster.set_delta(Vec3::Y);
        assert_eq!(adjuster.update(0.0).unwrap().translation, Vec3::Y);
        assert_eq!(adjuster.update(0.0), None);
    }

    #[test]
    fn position_adjust_matches_ideal_offset_and_yaw_equations() {
        let entity = PositionTransform::IDENTITY;
        let target = PositionTransform::new(
            Vec3::new(10.0, 0.0, 0.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        );
        let mut adjuster = PositionAdjuster::new(0.0);
        configure_position_adjust(
            &mut adjuster,
            entity,
            target,
            Vec3::Y,
            std::f32::consts::FRAC_PI_2,
            false,
            false,
        );

        assert!((adjuster.delta() - Vec3::new(11.0, 0.0, 0.0)).length() < 0.0001);
        assert!(adjuster.delta_rotation().angle_between(Quat::IDENTITY) < 0.0001);
    }

    #[test]
    fn collision_correction_requires_only_one_side_of_an_axis_to_hit() {
        let distances = CollisionSweepDistances {
            x_forward: 0.25,
            x_back: 0.0,
            y_forward: 0.2,
            y_back: 0.8,
        };
        assert_eq!(
            collision_adjustment(Vec3::new(1.0, 1.0, 1.0), distances, 3.0, 0.1),
            Vec3::new(-0.3, 0.0, 0.0)
        );
    }
}
