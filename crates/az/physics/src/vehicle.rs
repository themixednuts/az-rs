use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{PhysicsBodyHandle, PhysicsError, RigidBodyConfiguration, SurfaceIndex};

/// Complete Cry `pe_params_wheel` state plus the geometry-derived wheel frame.
///
/// Cry derives `connection`, `radius`, and the axle direction in
/// `CWheeledVehicleEntity::AddGeometry`. Azoth stores those derived values in
/// the cooked runtime product so every backend receives the same wheel.
#[expect(
    clippy::struct_excessive_bools,
    reason = "driving, can_brake, blocked, can_steer, and ray_cast are the distinct pe_params_wheel flags and must stay one field each"
)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct VehicleWheelConfiguration {
    pub part_id: u32,
    pub connection: Vec3,
    pub suspension_direction: Vec3,
    pub axle_direction: Vec3,
    pub radius: f32,
    /// Half-extent of the cooked wheel geometry along its axle.
    ///
    /// Cry stores `suspension_point::width` directly from the wheel geometry
    /// bounding-box half-size. Ray wheels use it to place the ray on the
    /// outer edge; geometry wheels use it as the swept cylinder half-height.
    pub half_width: f32,
    /// Geometry-derived inverse inertia about the wheel axle.
    pub inverse_inertia: f32,
    pub driving: bool,
    /// Cry axle index. Negative values retain the wheel visually but exclude
    /// it from suspension and tire-force simulation.
    pub axle: i32,
    pub can_brake: bool,
    pub blocked: bool,
    pub can_steer: bool,
    pub suspension_max_length: f32,
    pub suspension_initial_length: f32,
    pub minimum_friction: f32,
    pub maximum_friction: f32,
    pub surface_index: SurfaceIndex,
    pub ray_cast: bool,
    /// Zero requests Cry's mass-distribution stiffness calculation.
    pub stiffness: f32,
    pub stiffness_weight: f32,
    /// A negative value is a fraction of critical damping, exactly as in Cry.
    pub damping: f32,
    pub lateral_friction: f32,
    pub torque_scale: f32,
    pub angular_velocity: f32,
}

impl Default for VehicleWheelConfiguration {
    fn default() -> Self {
        Self {
            part_id: 0,
            connection: Vec3::ZERO,
            suspension_direction: Vec3::NEG_Z,
            axle_direction: Vec3::X,
            radius: 0.5,
            half_width: 0.25,
            inverse_inertia: 1.0,
            driving: false,
            axle: 0,
            can_brake: true,
            blocked: false,
            can_steer: false,
            suspension_max_length: 0.5,
            suspension_initial_length: 0.25,
            minimum_friction: 0.0,
            maximum_friction: 1.0,
            surface_index: SurfaceIndex(0),
            ray_cast: true,
            stiffness: 0.0,
            stiffness_weight: 1.0,
            damping: -0.7,
            lateral_friction: 1.0,
            torque_scale: 1.0,
            angular_velocity: 0.0,
        }
    }
}

impl VehicleWheelConfiguration {
    pub(crate) fn validate(self) -> Result<(), PhysicsError> {
        if !self.connection.is_finite()
            || !unit_vector(self.suspension_direction)
            || !unit_vector(self.axle_direction)
            || self.suspension_direction.dot(self.axle_direction).abs() > 1.0e-4
        {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "wheel frame",
            });
        }
        for (field, value) in [
            ("wheel radius", self.radius),
            ("wheel half width", self.half_width),
            ("wheel inverse inertia", self.inverse_inertia),
            ("suspension maximum length", self.suspension_max_length),
            ("suspension initial length", self.suspension_initial_length),
            ("minimum friction", self.minimum_friction),
            ("maximum friction", self.maximum_friction),
            ("stiffness", self.stiffness),
            ("lateral friction", self.lateral_friction),
            ("torque scale", self.torque_scale),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidVehicleConfiguration { field });
            }
        }
        if self.radius == 0.0
            || self.half_width == 0.0
            || self.inverse_inertia == 0.0
            || self.suspension_initial_length > self.suspension_max_length
            || (self.stiffness == 0.0
                && self.suspension_max_length - self.suspension_initial_length <= f32::EPSILON)
            || self.minimum_friction > self.maximum_friction
            || !self.stiffness_weight.is_finite()
            || !self.damping.is_finite()
            || !self.angular_velocity.is_finite()
        {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "wheel dynamics",
            });
        }
        Ok(())
    }
}

/// Cry `pe_params_car` plus the rigid chassis and cooked wheels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct WheeledVehicleConfiguration {
    pub rigid_body: RigidBodyConfiguration,
    pub axle_friction: f32,
    pub engine_power: f32,
    pub maximum_steer: f32,
    pub engine_maximum_rpm: f32,
    pub brake_torque: f32,
    pub maximum_time_step: f32,
    pub minimum_energy: f32,
    pub damping: f32,
    pub minimum_braking_friction: f32,
    pub maximum_braking_friction: f32,
    pub stabilizer: f32,
    pub engine_minimum_rpm: f32,
    pub engine_shift_up_rpm: f32,
    pub engine_shift_down_rpm: f32,
    pub engine_idle_rpm: f32,
    pub engine_start_rpm: f32,
    pub clutch_speed: f32,
    /// Cry indices are preserved: 0 reverse, 1 neutral, 2+ forward.
    pub gear_ratios: Vec<f32>,
    pub maximum_gear: i32,
    pub minimum_gear: i32,
    pub slip_threshold: f32,
    pub gear_direction_switch_rpm: f32,
    pub dynamic_friction: f32,
    /// Zero selects Ackermann steering. Non-zero selects tracked steering and
    /// is the angle producing a neutral turn.
    pub tracked_neutral_turn_steer: f32,
    pub pull_tilt: f32,
    pub maximum_tilt_cosine: f32,
    pub keep_traction_when_tilted: bool,
    pub wheels: Vec<VehicleWheelConfiguration>,
}

impl Default for WheeledVehicleConfiguration {
    fn default() -> Self {
        Self {
            rigid_body: RigidBodyConfiguration::default(),
            axle_friction: 0.0,
            engine_power: 10_000.0,
            maximum_steer: core::f32::consts::FRAC_PI_4,
            engine_maximum_rpm: radians_per_second_to_rpm(120.0),
            brake_torque: 4_000.0,
            maximum_time_step: 0.02,
            minimum_energy: 0.05 * 0.05,
            damping: 0.01,
            minimum_braking_friction: 0.0,
            maximum_braking_friction: 1.0,
            stabilizer: 0.0,
            engine_minimum_rpm: radians_per_second_to_rpm(6.0),
            engine_shift_up_rpm: radians_per_second_to_rpm(60.0),
            engine_shift_down_rpm: radians_per_second_to_rpm(24.0),
            engine_idle_rpm: radians_per_second_to_rpm(12.0),
            engine_start_rpm: radians_per_second_to_rpm(40.0),
            clutch_speed: 1.0,
            gear_ratios: vec![-1.0, 1.0],
            maximum_gear: 127,
            minimum_gear: 0,
            slip_threshold: 0.05,
            gear_direction_switch_rpm: radians_per_second_to_rpm(1.0),
            dynamic_friction: 1.0,
            tracked_neutral_turn_steer: 0.0,
            pull_tilt: 0.0,
            maximum_tilt_cosine: 0.866,
            keep_traction_when_tilted: false,
            wheels: Vec::new(),
        }
    }
}

impl WheeledVehicleConfiguration {
    pub(crate) fn validate(&self) -> Result<(), PhysicsError> {
        self.rigid_body.validate()?;
        if self.wheels.is_empty() {
            return Err(PhysicsError::InvalidVehicleConfiguration { field: "wheels" });
        }
        if self.gear_ratios.len() < 2 {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "gear ratios",
            });
        }
        for (field, value) in [
            ("axle friction", self.axle_friction),
            ("engine power", self.engine_power),
            ("maximum steer", self.maximum_steer),
            ("engine maximum rpm", self.engine_maximum_rpm),
            ("brake torque", self.brake_torque),
            ("maximum time step", self.maximum_time_step),
            ("minimum energy", self.minimum_energy),
            ("damping", self.damping),
            ("minimum braking friction", self.minimum_braking_friction),
            ("maximum braking friction", self.maximum_braking_friction),
            ("stabilizer", self.stabilizer),
            ("engine minimum rpm", self.engine_minimum_rpm),
            ("engine shift up rpm", self.engine_shift_up_rpm),
            ("engine shift down rpm", self.engine_shift_down_rpm),
            ("engine idle rpm", self.engine_idle_rpm),
            ("engine start rpm", self.engine_start_rpm),
            ("clutch speed", self.clutch_speed),
            ("slip threshold", self.slip_threshold),
            ("gear direction switch rpm", self.gear_direction_switch_rpm),
            ("dynamic friction", self.dynamic_friction),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidVehicleConfiguration { field });
            }
        }
        if self.maximum_time_step == 0.0
            || self.engine_maximum_rpm == 0.0
            || self.maximum_steer > core::f32::consts::PI
            || self.minimum_braking_friction > self.maximum_braking_friction
            || !self.tracked_neutral_turn_steer.is_finite()
            || !self.pull_tilt.is_finite()
            || !self.maximum_tilt_cosine.is_finite()
            || !(-1.0..=1.0).contains(&self.maximum_tilt_cosine)
            || self.minimum_gear < 0
            || self.maximum_gear < self.minimum_gear
            || self.gear_ratios.iter().any(|ratio| !ratio.is_finite())
        {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "drivetrain",
            });
        }
        for wheel in self.wheels.iter().copied() {
            wheel.validate()?;
        }
        Ok(())
    }
}

/// Partial Cry `pe_action_drive` update. `None` retains the current value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct VehicleDriveAction {
    pub pedal: Option<f32>,
    pub pedal_delta: Option<f32>,
    pub steer: Option<f32>,
    pub steer_delta: Option<f32>,
    pub ackermann_offset: Option<f32>,
    pub clutch: Option<f32>,
    pub hand_brake: Option<bool>,
    pub gear: Option<i32>,
}

impl VehicleDriveAction {
    /// Validates the finite/ranged fields supplied by this partial update.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidVehicleConfiguration`] when a supplied
    /// value is non-finite or when clutch/Ackermann values lie outside their
    /// normalized ranges.
    pub fn validate(self) -> Result<(), PhysicsError> {
        for value in [
            self.pedal,
            self.pedal_delta,
            self.steer,
            self.steer_delta,
            self.ackermann_offset,
            self.clutch,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() {
                return Err(PhysicsError::InvalidVehicleConfiguration {
                    field: "drive action",
                });
            }
        }
        if self
            .ackermann_offset
            .is_some_and(|offset| !(0.0..=1.0).contains(&offset))
            || self
                .clutch
                .is_some_and(|clutch| !(0.0..=1.0).contains(&clutch))
        {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "drive action range",
            });
        }
        Ok(())
    }
}

/// Cry `pe_status_vehicle` product.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct VehicleStatus {
    pub steer: f32,
    pub pedal: f32,
    pub hand_brake: bool,
    pub foot_brake: f32,
    pub velocity: Vec3,
    pub wheel_contacts: u32,
    pub current_gear: i32,
    pub engine_rpm: f32,
    pub clutch: f32,
    pub driving_torque: f32,
    pub active_colliders: u32,
}

/// Cry `pe_status_wheel` product.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct VehicleWheelStatus {
    pub wheel: usize,
    pub part_id: u32,
    pub contact: bool,
    pub contact_point: Vec3,
    pub contact_normal: Vec3,
    pub angular_velocity: f32,
    pub slipping: bool,
    pub slip_velocity: Vec3,
    pub contact_surface: Option<SurfaceIndex>,
    pub friction: f32,
    pub suspension_length: f32,
    pub suspension_full_length: f32,
    pub suspension_initial_length: f32,
    pub radius: f32,
    pub torque: f32,
    pub steer: f32,
    pub collider: Option<PhysicsBodyHandle>,
}

/// Cry `pe_status_vehicle_abilities` product.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct VehicleAbilities {
    pub rotation_pivot: Option<Vec3>,
    pub maximum_velocity: f32,
}

/// Backend capability for Cry wheeled/tracked vehicle state.
pub trait PhysicsVehicleBackend {
    /// Applies one driver input set to the vehicle.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresVehicleBody`] when it is not a wheeled
    /// vehicle.
    fn apply_vehicle_drive(
        &mut self,
        body: PhysicsBodyHandle,
        action: VehicleDriveAction,
    ) -> Result<(), PhysicsError>;

    /// Reads the drivetrain state produced by the last step.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresVehicleBody`] when it is not a wheeled
    /// vehicle.
    fn vehicle_status(&self, body: PhysicsBodyHandle) -> Result<VehicleStatus, PhysicsError>;

    /// Reads one wheel's contact and suspension state.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresVehicleBody`] when it is not a wheeled
    /// vehicle, and [`PhysicsError::VehicleWheelNotFound`] when `wheel` is out
    /// of range.
    fn vehicle_wheel_status(
        &self,
        body: PhysicsBodyHandle,
        wheel: usize,
    ) -> Result<VehicleWheelStatus, PhysicsError>;

    /// Reads the turning pivot and maximum velocity for an optional steer angle.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresVehicleBody`] when it is not a wheeled
    /// vehicle, and [`PhysicsError::InvalidVehicleConfiguration`] when `steer`
    /// is present but not finite.
    fn vehicle_abilities(
        &self,
        body: PhysicsBodyHandle,
        steer: Option<f32>,
    ) -> Result<VehicleAbilities, PhysicsError>;
}

impl<B: PhysicsVehicleBackend + ?Sized> PhysicsVehicleBackend for Box<B> {
    fn apply_vehicle_drive(
        &mut self,
        body: PhysicsBodyHandle,
        action: VehicleDriveAction,
    ) -> Result<(), PhysicsError> {
        (**self).apply_vehicle_drive(body, action)
    }

    fn vehicle_status(&self, body: PhysicsBodyHandle) -> Result<VehicleStatus, PhysicsError> {
        (**self).vehicle_status(body)
    }

    fn vehicle_wheel_status(
        &self,
        body: PhysicsBodyHandle,
        wheel: usize,
    ) -> Result<VehicleWheelStatus, PhysicsError> {
        (**self).vehicle_wheel_status(body, wheel)
    }

    fn vehicle_abilities(
        &self,
        body: PhysicsBodyHandle,
        steer: Option<f32>,
    ) -> Result<VehicleAbilities, PhysicsError> {
        (**self).vehicle_abilities(body, steer)
    }
}

#[inline]
fn unit_vector(value: Vec3) -> bool {
    value.is_finite() && (value.length_squared() - 1.0).abs() <= 1.0e-4
}

#[inline]
pub const fn radians_per_second_to_rpm(value: f32) -> f32 {
    value * (60.0 / core::f32::consts::TAU)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "these assert the exact default constants the native engine constructs, so an epsilon comparison would let a wrong constant pass"
    )]
    fn defaults_match_cry_vehicle_construction() {
        let vehicle = WheeledVehicleConfiguration::default();
        assert_eq!(vehicle.engine_power, 10_000.0);
        assert_eq!(vehicle.engine_maximum_rpm, radians_per_second_to_rpm(120.0));
        assert_eq!(vehicle.engine_start_rpm, radians_per_second_to_rpm(40.0));
        assert_eq!(vehicle.gear_ratios, [-1.0, 1.0]);
        assert_eq!(vehicle.maximum_time_step, 0.02);
        assert_eq!(vehicle.maximum_tilt_cosine, 0.866);
    }
}
