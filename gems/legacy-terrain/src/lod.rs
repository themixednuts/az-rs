//! Terrain LOD.

use bevy::prelude::*;

/// Narrow a LOD level index to the `u8` the terrain components store.
///
/// Saturates at [`u8::MAX`]. Past 255 configured levels the coarsest level is
/// the only sensible answer; wrapping would select the finest one instead.
pub(crate) fn lod_level(level: usize) -> u8 {
    u8::try_from(level).unwrap_or(u8::MAX)
}

/// Terrain LOD configuration.
#[derive(Resource, Debug, Clone, Reflect)]
#[reflect(Resource)]
pub struct TerrainLodConfig {
    /// LOD transition distances (meters)
    pub lod_distances: Vec<f32>,

    /// Enable smooth LOD transitions (morphing)
    pub enable_morphing: bool,

    /// Morphing blend distance (meters)
    pub morph_distance: f32,
}

impl Default for TerrainLodConfig {
    fn default() -> Self {
        Self {
            lod_distances: vec![256.0, 512.0, 1024.0, 2048.0],
            enable_morphing: true,
            morph_distance: 32.0,
        }
    }
}

/// Terrain LOD component.
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct TerrainLod {
    /// Current LOD level (0 = highest detail)
    pub current_level: u8,

    /// Target LOD level (for smooth transitions)
    pub target_level: u8,

    /// Morphing blend factor (0.0 = current, 1.0 = target)
    pub morph_factor: f32,

    /// LOD level count
    pub level_count: u8,
}

impl Default for TerrainLod {
    fn default() -> Self {
        Self {
            current_level: 0,
            target_level: 0,
            morph_factor: 0.0,
            level_count: 4,
        }
    }
}

impl TerrainLod {
    /// Create a new LOD component.
    #[must_use]
    pub const fn new(level_count: u8) -> Self {
        Self {
            current_level: 0,
            target_level: 0,
            morph_factor: 0.0,
            level_count,
        }
    }

    /// Update LOD level based on distance.
    ///
    /// # Parameters
    /// - `distance`: Distance from camera in meters
    /// - `config`: LOD configuration
    /// - `delta_seconds`: Time delta from last frame (use `Time::delta_secs()`)
    pub fn update_for_distance(
        &mut self,
        distance: f32,
        config: &TerrainLodConfig,
        delta_seconds: f32,
    ) {
        // Find target LOD level
        let mut target = lod_level(config.lod_distances.len());
        for (level, &threshold) in config.lod_distances.iter().enumerate() {
            if distance < threshold {
                target = lod_level(level);
                break;
            }
        }

        self.target_level = target.min(self.level_count - 1);

        // Update morph factor if morphing enabled
        if config.enable_morphing && self.current_level != self.target_level {
            // Gradually transition to target level
            let transition_speed = 2.0; // transitions per second
            let delta = transition_speed * delta_seconds;

            if self.current_level < self.target_level {
                self.morph_factor += delta;
                if self.morph_factor >= 1.0 {
                    self.current_level = self.target_level;
                    self.morph_factor = 0.0;
                }
            } else {
                self.morph_factor -= delta;
                if self.morph_factor <= 0.0 {
                    self.current_level = self.target_level;
                    self.morph_factor = 0.0;
                }
            }
        } else {
            self.current_level = self.target_level;
            self.morph_factor = 0.0;
        }
    }

    /// Get vertex skip amount for current LOD level.
    #[must_use]
    pub const fn vertex_skip(&self) -> usize {
        1 << self.current_level // 1, 2, 4, 8, ...
    }
}
