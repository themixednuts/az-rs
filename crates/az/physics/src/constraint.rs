use std::num::NonZeroU64;

use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{PhysicsBodyHandle, PhysicsError, PhysicsPose, PhysicsSceneId};

/// Stable engine handle for a constraint in one isolated physics scene.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Reflect,
)]
pub struct PhysicsConstraintHandle {
    scene: PhysicsSceneId,
    value: NonZeroU64,
}

impl PhysicsConstraintHandle {
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

/// The lead side of a constraint. `CryPhysics` calls this `pBuddy` and permits
/// the special world entity as well as an ordinary physical entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintTarget {
    World,
    Body(PhysicsBodyHandle),
}

/// Solver formulation selected explicitly by the domain that owns the joint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintSolverModel {
    /// General graph constraint. Supports loops and force/torque tearing.
    #[default]
    Impulse,
    /// Reduced-coordinate tree used for stable articulated bodies.
    ReducedCoordinate,
}

impl From<PhysicsBodyHandle> for ConstraintTarget {
    fn from(body: PhysicsBodyHandle) -> Self {
        Self::Body(body)
    }
}

/// One translational or rotational degree of freedom.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintAxis {
    LinearX,
    LinearY,
    LinearZ,
    AngularX,
    AngularY,
    AngularZ,
}

impl ConstraintAxis {
    pub const ALL: [Self; 6] = [
        Self::LinearX,
        Self::LinearY,
        Self::LinearZ,
        Self::AngularX,
        Self::AngularY,
        Self::AngularZ,
    ];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Compact set of translational or rotational constraint axes.
///
/// A mask is used instead of a backend enum so coupled rows remain a
/// solver-neutral engine contract. Linear and angular coupling groups are
/// validated separately by [`ConstraintDescriptor`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct ConstraintAxisMask(u8);

impl ConstraintAxisMask {
    pub const NONE: Self = Self(0);
    pub const LINEAR: Self = Self(
        (1 << ConstraintAxis::LinearX as u8)
            | (1 << ConstraintAxis::LinearY as u8)
            | (1 << ConstraintAxis::LinearZ as u8),
    );
    pub const ANGULAR: Self = Self(
        (1 << ConstraintAxis::AngularX as u8)
            | (1 << ConstraintAxis::AngularY as u8)
            | (1 << ConstraintAxis::AngularZ as u8),
    );

    #[must_use]
    pub const fn from_axis(axis: ConstraintAxis) -> Self {
        Self(1 << axis as u8)
    }

    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !0x3f == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, axis: ConstraintAxis) -> bool {
        self.0 & Self::from_axis(axis).0 != 0
    }

    #[must_use]
    pub const fn is_subset(self, axes: Self) -> bool {
        self.0 & !axes.0 == 0
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub fn first(self) -> Option<ConstraintAxis> {
        ConstraintAxis::ALL
            .into_iter()
            .find(|axis| self.contains(*axis))
    }
}

impl From<ConstraintAxis> for ConstraintAxisMask {
    fn from(axis: ConstraintAxis) -> Self {
        Self::from_axis(axis)
    }
}

impl core::ops::BitOr for ConstraintAxisMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for ConstraintAxisMask {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A hard or soft Cry joint limit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintLimit {
    pub minimum: f32,
    pub maximum: f32,
    /// Velocity fraction restored when a hard limit is reached.
    pub restitution: f32,
    /// Distance/angle before the limit where resistance begins (`qdashpot`).
    pub contact_distance: f32,
    /// Dashpot damping applied in the limit vicinity (`kdashpot`).
    pub damping: f32,
    /// Soft-limit stiffness. Zero selects a hard solver limit.
    pub stiffness: f32,
}

impl ConstraintLimit {
    #[must_use]
    pub const fn hard(minimum: f32, maximum: f32) -> Self {
        Self {
            minimum,
            maximum,
            restitution: 0.0,
            contact_distance: 0.0,
            damping: 0.0,
            stiffness: 0.0,
        }
    }

    fn validate(self) -> Result<(), PhysicsError> {
        if !self.minimum.is_finite() || !self.maximum.is_finite() || self.minimum > self.maximum {
            return Err(PhysicsError::InvalidConstraintConfiguration {
                field: "limit range",
            });
        }
        if !self.restitution.is_finite() || !(0.0..=1.0).contains(&self.restitution) {
            return Err(PhysicsError::InvalidConstraintConfiguration {
                field: "limit restitution",
            });
        }
        for (field, value) in [
            ("limit contact distance", self.contact_distance),
            ("limit damping", self.damping),
            ("limit stiffness", self.stiffness),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidConstraintConfiguration { field });
            }
        }
        Ok(())
    }
}

/// Motion permitted along one constraint axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintMotion {
    Free,
    Locked,
    Limited(ConstraintLimit),
}

/// Servo/velocity drive for one constraint axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintDrive {
    pub target_position: f32,
    pub target_velocity: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub maximum_force: f32,
}

impl ConstraintDrive {
    fn validate(self) -> Result<(), PhysicsError> {
        for (field, value) in [
            ("drive target position", self.target_position),
            ("drive target velocity", self.target_velocity),
            ("drive stiffness", self.stiffness),
            ("drive damping", self.damping),
            ("drive maximum force", self.maximum_force),
        ] {
            if !value.is_finite()
                || matches!(
                    field,
                    "drive stiffness" | "drive damping" | "drive maximum force"
                ) && value < 0.0
            {
                return Err(PhysicsError::InvalidConstraintConfiguration { field });
            }
        }
        Ok(())
    }
}

/// Complete state for one degree of freedom.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintAxisConfiguration {
    pub motion: ConstraintMotion,
    pub drive: Option<ConstraintDrive>,
}

impl ConstraintAxisConfiguration {
    pub const FREE: Self = Self {
        motion: ConstraintMotion::Free,
        drive: None,
    };
    pub const LOCKED: Self = Self {
        motion: ConstraintMotion::Locked,
        drive: None,
    };

    fn validate(self) -> Result<(), PhysicsError> {
        if let ConstraintMotion::Limited(limit) = self.motion {
            limit.validate()?;
        }
        if let Some(drive) = self.drive {
            drive.validate()?;
        }
        Ok(())
    }
}

impl Default for ConstraintAxisConfiguration {
    fn default() -> Self {
        Self::FREE
    }
}

/// Six-degree-of-freedom constraint state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintAxes(pub [ConstraintAxisConfiguration; 6]);

impl ConstraintAxes {
    pub const FREE: Self = Self([ConstraintAxisConfiguration::FREE; 6]);
    pub const FIXED: Self = Self([ConstraintAxisConfiguration::LOCKED; 6]);
    pub const SPHERICAL: Self = Self([
        ConstraintAxisConfiguration::LOCKED,
        ConstraintAxisConfiguration::LOCKED,
        ConstraintAxisConfiguration::LOCKED,
        ConstraintAxisConfiguration::FREE,
        ConstraintAxisConfiguration::FREE,
        ConstraintAxisConfiguration::FREE,
    ]);

    #[must_use]
    pub const fn get(self, axis: ConstraintAxis) -> ConstraintAxisConfiguration {
        self.0[axis.index()]
    }

    pub const fn set(&mut self, axis: ConstraintAxis, configuration: ConstraintAxisConfiguration) {
        self.0[axis.index()] = configuration;
    }
}

impl Default for ConstraintAxes {
    fn default() -> Self {
        Self::FREE
    }
}

impl AsRef<[ConstraintAxisConfiguration; 6]> for ConstraintAxes {
    fn as_ref(&self) -> &[ConstraintAxisConfiguration; 6] {
        &self.0
    }
}

/// One solver row shared by two or three axes.
///
/// Coupled linear axes model radial/rope limits. Coupled angular axes model a
/// cone around the remaining frame axis. The participating independent axis
/// configurations must remain free so the shared row has one unambiguous
/// limit and drive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintCoupling {
    pub axes: ConstraintAxisMask,
    pub limit: Option<ConstraintLimit>,
    pub drive: Option<ConstraintDrive>,
}

impl ConstraintCoupling {
    #[must_use]
    pub const fn limited(axes: ConstraintAxisMask, limit: ConstraintLimit) -> Self {
        Self {
            axes,
            limit: Some(limit),
            drive: None,
        }
    }

    fn validate(
        self,
        expected_axes: ConstraintAxisMask,
        independent_axes: &ConstraintAxes,
    ) -> Result<(), PhysicsError> {
        if self.axes.len() < 2 || !self.axes.is_subset(expected_axes) {
            return Err(PhysicsError::InvalidConstraintConfiguration {
                field: "coupled axes",
            });
        }
        if self.limit.is_none() && self.drive.is_none() {
            return Err(PhysicsError::InvalidConstraintConfiguration {
                field: "coupled row",
            });
        }
        for axis in ConstraintAxis::ALL {
            if self.axes.contains(axis)
                && independent_axes.get(axis) != ConstraintAxisConfiguration::FREE
            {
                return Err(PhysicsError::InvalidConstraintConfiguration {
                    field: "coupled independent axis",
                });
            }
        }
        if let Some(limit) = self.limit {
            limit.validate()?;
        }
        if let Some(drive) = self.drive {
            drive.validate()?;
        }
        Ok(())
    }
}

/// `RockNRoll` constraint families retained by the diagnostic serializer.
///
/// These discriminants are a capability boundary, not construction options.
/// Importers may preserve them as diagnostic evidence, but must not translate
/// them into [`ConstraintDescriptor`] or a backend approximation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOnlyRockNRollConstraintFamily {
    Axis = 5,
    Path = 6,
    SoftBodyAnchor = 7,
}

impl DiagnosticOnlyRockNRollConstraintFamily {
    /// Stable diagnostic serializer group name.
    #[must_use]
    pub const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Axis => "Axis",
            Self::Path => "Path",
            Self::SoftBodyAnchor => "SoftBodyAnchor",
        }
    }
}

/// Solver-neutral equivalent of `CryPhysics` `pe_action_add_constraint`, with
/// O3DE-style local frames and explicit six-axis motion.
///
/// This descriptor covers evidenced, backend-representable rows and coupling.
/// It deliberately has no Axis/Path/SoftBodyAnchor family selector; those
/// `RockNRoll` names remain diagnostic metadata. See
/// [`DiagnosticOnlyRockNRollConstraintFamily`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintDescriptor {
    pub parent: ConstraintTarget,
    pub child: PhysicsBodyHandle,
    pub parent_frame: PhysicsPose,
    pub child_frame: PhysicsPose,
    pub axes: ConstraintAxes,
    /// Optional radial row shared by two or three linear frame axes.
    pub linear_coupling: Option<ConstraintCoupling>,
    /// Optional cone row shared by two angular frame axes.
    pub angular_coupling: Option<ConstraintCoupling>,
    pub solver_model: ConstraintSolverModel,
    pub enabled: bool,
    /// False implements Cry's `constraint_ignore_buddy` flag.
    pub contacts_enabled: bool,
    /// Maximum linear reaction force before tearing. `None` is unbreakable.
    pub break_force: Option<f32>,
    /// Maximum angular reaction torque before tearing. `None` is unbreakable.
    pub break_torque: Option<f32>,
    /// Maximum absolute solver-row impulse before tearing. This preserves
    /// `RockNRoll` `BreakImpulse` semantics without converting it to force.
    pub break_impulse: Option<f32>,
    pub damping: f32,
    /// Radius used by higher-level reattachment logic after a break.
    pub sensor_radius: f32,
}

impl ConstraintDescriptor {
    #[must_use]
    pub const fn fixed(parent: ConstraintTarget, child: PhysicsBodyHandle) -> Self {
        Self {
            parent,
            child,
            parent_frame: PhysicsPose::IDENTITY,
            child_frame: PhysicsPose::IDENTITY,
            axes: ConstraintAxes::FIXED,
            linear_coupling: None,
            angular_coupling: None,
            solver_model: ConstraintSolverModel::Impulse,
            enabled: true,
            contacts_enabled: true,
            break_force: None,
            break_torque: None,
            break_impulse: None,
            damping: 0.0,
            sensor_radius: 0.0,
        }
    }

    #[must_use]
    pub const fn spherical(parent: ConstraintTarget, child: PhysicsBodyHandle) -> Self {
        Self {
            axes: ConstraintAxes::SPHERICAL,
            ..Self::fixed(parent, child)
        }
    }

    #[must_use]
    pub const fn scene(&self) -> PhysicsSceneId {
        self.child.scene()
    }

    /// Validates scenes, frames, axis limits/drives, and break thresholds.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] when the descriptor crosses scene boundaries
    /// or contains a non-finite/invalid constraint value.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if let ConstraintTarget::Body(parent) = self.parent
            && parent.scene() != self.child.scene()
        {
            return Err(PhysicsError::ConstraintSceneMismatch {
                parent: parent.scene(),
                child: self.child.scene(),
            });
        }
        validate_pose(self.parent_frame, "parent frame")?;
        validate_pose(self.child_frame, "child frame")?;
        for axis in self.axes.0 {
            axis.validate()?;
        }
        if let Some(coupling) = self.linear_coupling {
            coupling.validate(ConstraintAxisMask::LINEAR, &self.axes)?;
        }
        if let Some(coupling) = self.angular_coupling {
            coupling.validate(ConstraintAxisMask::ANGULAR, &self.axes)?;
        }
        for (field, value) in [
            ("break force", self.break_force),
            ("break torque", self.break_torque),
            ("break impulse", self.break_impulse),
        ] {
            if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
                return Err(PhysicsError::InvalidConstraintConfiguration { field });
            }
        }
        if self.solver_model == ConstraintSolverModel::ReducedCoordinate
            && (self.break_force.is_some()
                || self.break_torque.is_some()
                || self.break_impulse.is_some())
        {
            return Err(PhysicsError::InvalidConstraintConfiguration {
                field: "reduced-coordinate break threshold",
            });
        }
        for (field, value) in [
            ("constraint damping", self.damping),
            ("constraint sensor radius", self.sensor_radius),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidConstraintConfiguration { field });
            }
        }
        Ok(())
    }
}

fn validate_pose(pose: PhysicsPose, field: &'static str) -> Result<(), PhysicsError> {
    let rotation_length = pose.rotation.length_squared();
    if !pose.translation.is_finite()
        || !pose.rotation.is_finite()
        || !rotation_length.is_finite()
        || rotation_length <= f32::EPSILON
    {
        return Err(PhysicsError::InvalidConstraintConfiguration { field });
    }
    Ok(())
}

/// Why a breakable constraint was removed by the solver adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintBreakReason {
    Force,
    Torque,
    Impulse,
}

/// Runtime state corresponding to Cry's public constraint/joint status.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ConstraintStatus {
    pub enabled: bool,
    pub broken: bool,
    pub break_reason: Option<ConstraintBreakReason>,
    pub linear_impulse: Vec3,
    pub angular_impulse: Vec3,
}

/// Constraint capability implemented by every engine physics backend.
pub trait PhysicsConstraintBackend: std::fmt::Debug + Send + Sync + 'static {
    /// Creates one solver constraint from an already validated descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when a referenced body is not
    /// registered, [`PhysicsError::ConstraintHandleExhausted`] when the handle
    /// space is full, and [`PhysicsError::InvalidConstraintConfiguration`] for a
    /// descriptor row the backend cannot represent.
    fn create_constraint(
        &mut self,
        descriptor: &ConstraintDescriptor,
    ) -> Result<PhysicsConstraintHandle, PhysicsError>;
    /// Re-applies a descriptor to an existing solver constraint.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::ConstraintNotFound`] when `constraint` is not
    /// registered, [`PhysicsError::BodyNotFound`] when a referenced body is
    /// gone, and [`PhysicsError::InvalidConstraintConfiguration`] for a
    /// descriptor row the backend cannot represent.
    fn update_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
        descriptor: &ConstraintDescriptor,
    ) -> Result<(), PhysicsError>;
    /// Destroys one solver constraint.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::ConstraintNotFound`] when `constraint` is not
    /// registered in this backend.
    fn remove_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<(), PhysicsError>;
    /// Reads the enable/break state and reaction impulses of one constraint.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::ConstraintNotFound`] when `constraint` is not
    /// registered in this backend.
    fn constraint_status(
        &self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<ConstraintStatus, PhysicsError>;
}

impl<B: PhysicsConstraintBackend + ?Sized> PhysicsConstraintBackend for Box<B> {
    fn create_constraint(
        &mut self,
        descriptor: &ConstraintDescriptor,
    ) -> Result<PhysicsConstraintHandle, PhysicsError> {
        (**self).create_constraint(descriptor)
    }

    fn update_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
        descriptor: &ConstraintDescriptor,
    ) -> Result<(), PhysicsError> {
        (**self).update_constraint(constraint, descriptor)
    }

    fn remove_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<(), PhysicsError> {
        (**self).remove_constraint(constraint)
    }

    fn constraint_status(
        &self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<ConstraintStatus, PhysicsError> {
        (**self).constraint_status(constraint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(scene: u64, value: u64) -> PhysicsBodyHandle {
        PhysicsBodyHandle::in_scene(
            PhysicsSceneId::new(scene),
            NonZeroU64::new(value).expect("test handle is nonzero"),
        )
    }

    #[test]
    fn diagnostic_only_constraint_families_preserve_native_discriminants() {
        assert_eq!(DiagnosticOnlyRockNRollConstraintFamily::Axis as u8, 5);
        assert_eq!(DiagnosticOnlyRockNRollConstraintFamily::Path as u8, 6);
        assert_eq!(
            DiagnosticOnlyRockNRollConstraintFamily::SoftBodyAnchor as u8,
            7
        );
        assert_eq!(
            DiagnosticOnlyRockNRollConstraintFamily::Axis.diagnostic_name(),
            "Axis"
        );
        assert_eq!(
            DiagnosticOnlyRockNRollConstraintFamily::Path.diagnostic_name(),
            "Path"
        );
        assert_eq!(
            DiagnosticOnlyRockNRollConstraintFamily::SoftBodyAnchor.diagnostic_name(),
            "SoftBodyAnchor"
        );
    }

    #[test]
    fn fixed_constraint_locks_all_six_axes() {
        let descriptor = ConstraintDescriptor::fixed(ConstraintTarget::World, body(3, 1));
        assert!(
            descriptor
                .axes
                .0
                .iter()
                .all(|axis| axis.motion == ConstraintMotion::Locked)
        );
        assert!(descriptor.validate().is_ok());
    }

    #[test]
    fn constraints_cannot_cross_scene_boundaries() {
        let descriptor = ConstraintDescriptor::fixed(body(1, 1).into(), body(2, 1));
        assert_eq!(
            descriptor.validate(),
            Err(PhysicsError::ConstraintSceneMismatch {
                parent: PhysicsSceneId::new(1),
                child: PhysicsSceneId::new(2),
            })
        );
    }

    #[test]
    fn angular_cone_is_one_coupled_row() {
        let mut descriptor = ConstraintDescriptor::spherical(ConstraintTarget::World, body(1, 1));
        descriptor.angular_coupling = Some(ConstraintCoupling::limited(
            ConstraintAxisMask::from(ConstraintAxis::AngularY)
                | ConstraintAxisMask::from(ConstraintAxis::AngularZ),
            ConstraintLimit::hard(0.0, core::f32::consts::FRAC_PI_4),
        ));
        assert!(descriptor.validate().is_ok());
    }

    #[test]
    fn coupled_axes_cannot_also_have_independent_rows() {
        let mut descriptor = ConstraintDescriptor::spherical(ConstraintTarget::World, body(1, 1));
        descriptor.axes.set(
            ConstraintAxis::AngularY,
            ConstraintAxisConfiguration {
                motion: ConstraintMotion::Limited(ConstraintLimit::hard(-1.0, 1.0)),
                drive: None,
            },
        );
        descriptor.angular_coupling = Some(ConstraintCoupling::limited(
            ConstraintAxisMask::from(ConstraintAxis::AngularY)
                | ConstraintAxisMask::from(ConstraintAxis::AngularZ),
            ConstraintLimit::hard(0.0, 1.0),
        ));
        assert_eq!(
            descriptor.validate(),
            Err(PhysicsError::InvalidConstraintConfiguration {
                field: "coupled independent axis",
            })
        );
    }
}
