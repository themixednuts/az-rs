use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::GradientSampleParams;

const HASH_COMBINE_MAGIC: u64 = 0x9e37_79b9;
const RANDOM_OUTPUT_MAX: u64 = u8::MAX as u64;

/// Random gradient configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/RandomGradientComponent.h:29`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct RandomGradientConfig {
    pub random_seed: i32,
    pub gradient_scale: i32,
}

impl Default for RandomGradientConfig {
    fn default() -> Self {
        Self {
            random_seed: 13,
            gradient_scale: 1,
        }
    }
}

impl RandomGradientConfig {
    #[must_use]
    pub const fn normalized_random_seed(&self) -> u64 {
        self.random_seed.cast_unsigned() as u64 + 2
    }

    #[must_use]
    pub const fn effective_gradient_scale(&self) -> f32 {
        // `gradient_scale` is an editor-authored integer and the C++ source
        // does the same `float(m_gradientScale)` widening; there is no checked
        // `i32 -> f32` in std.
        #[allow(clippy::cast_precision_loss)]
        let scale = self.gradient_scale as f32;
        if self.gradient_scale > 0 { scale } else { 1.0 }
    }

    /// Samples deterministic position-based random noise.
    ///
    /// O3DE reference: `Gems/GradientSignal/Code/Source/Components/RandomGradientComponent.cpp:145`.
    #[must_use]
    pub fn sample_value(&self, params: GradientSampleParams) -> f32 {
        let position = params.position / self.effective_gradient_scale();
        stable_position_random(position.x, position.y, self.normalized_random_seed())
    }
}

/// Runtime random gradient component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/RandomGradientComponent.h:41`.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.RandomGradientComponent", version = 1)]
pub struct RandomGradientComponent {
    pub configuration: RandomGradientConfig,
}

impl RandomGradientComponent {
    #[must_use]
    pub const fn new(configuration: RandomGradientConfig) -> Self {
        Self { configuration }
    }

    #[must_use]
    pub fn sample_value(&self, params: GradientSampleParams) -> f32 {
        self.configuration.sample_value(params)
    }
}

// The three `x * seed + y` terms are hashed through `f32::to_bits`, so they are
// not rewritten as `mul_add`: the fused form rounds differently, and one changed
// bit changes the hash — and with it every sampled value — completely.
#[allow(clippy::suboptimal_flops)]
fn stable_position_random(x: f32, y: f32, seed: u64) -> f32 {
    // The C++ source hashes `float(seed)`, so seeds past 2^24 lose low bits
    // there too; reproducing that rounding is what keeps this port on the
    // Lumberyard golden master.
    #[allow(clippy::cast_precision_loss)]
    let seed_f32 = seed as f32;
    let mut result = 0;
    hash_combine_f32(&mut result, x * seed_f32 + y);
    hash_combine_f32(&mut result, y * seed_f32 + x);
    hash_combine_f32(&mut result, x * y * seed_f32);

    // `result % 255` is `0..=254`, so the byte narrowing is exact.
    let bucket = (result % RANDOM_OUTPUT_MAX) as u8;
    f32::from(bucket) / f32::from(u8::MAX)
}

fn hash_combine_f32(seed: &mut u64, value: f32) {
    let value_hash = f32_hash(value);
    *seed ^= value_hash
        .wrapping_add(HASH_COMBINE_MAGIC)
        .wrapping_add(seed.wrapping_shl(6))
        .wrapping_add(*seed >> 2);
}

fn f32_hash(value: f32) -> u64 {
    match value.to_bits() {
        0x8000_0000 => 0,
        bits => u64::from(bits),
    }
}
