// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

/// Orbit camera controller for the editor viewport camera: yaw/pitch around a
/// focus point at a distance, driven by normalized mouse deltas forwarded from
/// the viewport panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OrbitCameraController {
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) distance: f32,
    pub(super) focus: Vec3,
    pub(super) speed: f32,
}

impl OrbitCameraController {
    pub(super) fn from_pose(position: Vec3, focus: Vec3) -> Self {
        let offset = position - focus;
        let distance = offset.length().max(0.1);
        Self {
            yaw: offset.x.atan2(offset.z),
            pitch: (offset.y / distance).clamp(-1.0, 1.0).asin(),
            distance,
            focus,
            speed: 4.0,
        }
    }

    /// Unit vector from the focus toward the camera.
    pub(super) fn direction(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }

    pub(super) fn position(&self) -> Vec3 {
        self.focus + self.direction() * self.distance
    }

    /// Camera-space right vector (world Y stays up).
    pub(super) fn right(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin())
    }

    /// Camera-space up vector.
    pub(super) fn up(&self) -> Vec3 {
        Vec3::new(
            -self.pitch.sin() * self.yaw.sin(),
            self.pitch.cos(),
            -self.pitch.sin() * self.yaw.cos(),
        )
    }
}
