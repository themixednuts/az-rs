//! `FastNoise` Gem component data and Bevy integration.
//!
//! O3DE reference: `Gems/FastNoise/Code/Source/FastNoiseGradientComponent.h:35`.

use az_derive::{AzComponent, AzRtti};
use az_gem_contract::{Contribution, GemContext, contribution};
use az_prefab::{Prefab, PrefabType, ReflectPrefab};
use bevy::prelude::*;
use bracket_noise::prelude::{
    CellularDistanceFunction as NativeCellularDistanceFunction,
    CellularReturnType as NativeCellularReturnType, FastNoise as NativeFastNoise,
    FractalType as NativeFractalType, Interp as NativeInterpolation, NoiseType as NativeNoiseType,
};
use uuid::{Uuid, uuid};

/// Current `FastNoiseGem::FastNoiseGradientComponent` AZ component UUID.
pub const FAST_NOISE_GRADIENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("173AD9DC-DD96-459F-B64E-1EA470767ABE");

/// Current `FastNoiseGem::FastNoiseGradientConfig` AZ RTTI UUID.
pub const FAST_NOISE_GRADIENT_CONFIG_TYPE_ID: Uuid = uuid!("F5047B8D-A81F-4C8C-930E-FF1711535D89");

pub const DEFAULT_SEED: i32 = 1;
pub const DEFAULT_FREQUENCY: f32 = 1.0;
pub const DEFAULT_OCTAVES: i32 = 4;
pub const DEFAULT_LACUNARITY: f32 = 2.0;
pub const DEFAULT_GAIN: f32 = 0.5;
pub const DEFAULT_CELLULAR_JITTER: f32 = 0.45;

/// `FastNoise` interpolation mode.
///
/// O3DE reference: `Gems/FastNoise/Code/External/FastNoise/FastNoise.h:46`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FastNoiseInterpolation {
    Linear,
    Hermite,
    #[default]
    Quintic,
}

impl FastNoiseInterpolation {
    #[must_use]
    pub const fn from_native(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Linear),
            1 => Some(Self::Hermite),
            2 => Some(Self::Quintic),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native(self) -> i32 {
        match self {
            Self::Linear => 0,
            Self::Hermite => 1,
            Self::Quintic => 2,
        }
    }

    #[must_use]
    pub const fn to_generator(self) -> NativeInterpolation {
        match self {
            Self::Linear => NativeInterpolation::Linear,
            Self::Hermite => NativeInterpolation::Hermite,
            Self::Quintic => NativeInterpolation::Quintic,
        }
    }
}

/// `FastNoise` output algorithm.
///
/// O3DE reference: `Gems/FastNoise/Code/External/FastNoise/FastNoise.h:45`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FastNoiseType {
    Value,
    ValueFractal,
    Perlin,
    #[default]
    PerlinFractal,
    Simplex,
    SimplexFractal,
    Cellular,
    WhiteNoise,
    Cubic,
    CubicFractal,
}

impl FastNoiseType {
    #[must_use]
    pub const fn from_native(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Value),
            1 => Some(Self::ValueFractal),
            2 => Some(Self::Perlin),
            3 => Some(Self::PerlinFractal),
            4 => Some(Self::Simplex),
            5 => Some(Self::SimplexFractal),
            6 => Some(Self::Cellular),
            7 => Some(Self::WhiteNoise),
            8 => Some(Self::Cubic),
            9 => Some(Self::CubicFractal),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native(self) -> i32 {
        match self {
            Self::Value => 0,
            Self::ValueFractal => 1,
            Self::Perlin => 2,
            Self::PerlinFractal => 3,
            Self::Simplex => 4,
            Self::SimplexFractal => 5,
            Self::Cellular => 6,
            Self::WhiteNoise => 7,
            Self::Cubic => 8,
            Self::CubicFractal => 9,
        }
    }

    #[must_use]
    pub const fn to_generator(self) -> NativeNoiseType {
        match self {
            Self::Value => NativeNoiseType::Value,
            Self::ValueFractal => NativeNoiseType::ValueFractal,
            Self::Perlin => NativeNoiseType::Perlin,
            Self::PerlinFractal => NativeNoiseType::PerlinFractal,
            Self::Simplex => NativeNoiseType::Simplex,
            Self::SimplexFractal => NativeNoiseType::SimplexFractal,
            Self::Cellular => NativeNoiseType::Cellular,
            Self::WhiteNoise => NativeNoiseType::WhiteNoise,
            Self::Cubic => NativeNoiseType::Cubic,
            Self::CubicFractal => NativeNoiseType::CubicFractal,
        }
    }
}

/// `FastNoise` fractal combiner.
///
/// O3DE reference: `Gems/FastNoise/Code/External/FastNoise/FastNoise.h:47`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FastNoiseFractalType {
    #[default]
    Fbm,
    Billow,
    RigidMulti,
}

impl FastNoiseFractalType {
    #[must_use]
    pub const fn from_native(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Fbm),
            1 => Some(Self::Billow),
            2 => Some(Self::RigidMulti),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native(self) -> i32 {
        match self {
            Self::Fbm => 0,
            Self::Billow => 1,
            Self::RigidMulti => 2,
        }
    }

    #[must_use]
    pub const fn to_generator(self) -> NativeFractalType {
        match self {
            Self::Fbm => NativeFractalType::FBM,
            Self::Billow => NativeFractalType::Billow,
            Self::RigidMulti => NativeFractalType::RigidMulti,
        }
    }
}

/// `FastNoise` cellular distance function.
///
/// O3DE reference: `Gems/FastNoise/Code/External/FastNoise/FastNoise.h:48`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FastNoiseCellularDistanceFunction {
    #[default]
    Euclidean,
    Manhattan,
    Natural,
}

impl FastNoiseCellularDistanceFunction {
    #[must_use]
    pub const fn from_native(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Euclidean),
            1 => Some(Self::Manhattan),
            2 => Some(Self::Natural),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native(self) -> i32 {
        match self {
            Self::Euclidean => 0,
            Self::Manhattan => 1,
            Self::Natural => 2,
        }
    }

    #[must_use]
    pub const fn to_generator(self) -> NativeCellularDistanceFunction {
        match self {
            Self::Euclidean => NativeCellularDistanceFunction::Euclidean,
            Self::Manhattan => NativeCellularDistanceFunction::Manhattan,
            Self::Natural => NativeCellularDistanceFunction::Natural,
        }
    }
}

/// `FastNoise` cellular return mode.
///
/// O3DE reference: `Gems/FastNoise/Code/External/FastNoise/FastNoise.h:49`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FastNoiseCellularReturnType {
    #[default]
    CellValue,
    NoiseLookup,
    Distance,
    Distance2,
    Distance2Add,
    Distance2Sub,
    Distance2Mul,
    Distance2Div,
}

impl FastNoiseCellularReturnType {
    #[must_use]
    pub const fn from_native(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::CellValue),
            1 => Some(Self::NoiseLookup),
            2 => Some(Self::Distance),
            3 => Some(Self::Distance2),
            4 => Some(Self::Distance2Add),
            5 => Some(Self::Distance2Sub),
            6 => Some(Self::Distance2Mul),
            7 => Some(Self::Distance2Div),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native(self) -> i32 {
        match self {
            Self::CellValue => 0,
            Self::NoiseLookup => 1,
            Self::Distance => 2,
            Self::Distance2 => 3,
            Self::Distance2Add => 4,
            Self::Distance2Sub => 5,
            Self::Distance2Mul => 6,
            Self::Distance2Div => 7,
        }
    }

    #[must_use]
    pub const fn to_generator(self) -> NativeCellularReturnType {
        match self {
            Self::CellValue | Self::NoiseLookup => NativeCellularReturnType::CellValue,
            Self::Distance => NativeCellularReturnType::Distance,
            Self::Distance2 => NativeCellularReturnType::Distance2,
            Self::Distance2Add => NativeCellularReturnType::Distance2Add,
            Self::Distance2Sub => NativeCellularReturnType::Distance2Sub,
            Self::Distance2Mul => NativeCellularReturnType::Distance2Mul,
            Self::Distance2Div => NativeCellularReturnType::Distance2Div,
        }
    }
}

/// `FastNoise` gradient configuration.
///
/// O3DE reference: `Gems/FastNoise/Code/Source/FastNoiseGradientComponent.h:35`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect)]
#[az_rtti(
    name = "FastNoiseGem::FastNoiseGradientConfig",
    FAST_NOISE_GRADIENT_CONFIG_TYPE_ID,
    register
)]
pub struct FastNoiseGradientConfig {
    pub seed: i32,
    pub frequency: f32,
    pub interpolation: FastNoiseInterpolation,
    pub noise_type: FastNoiseType,
    pub octaves: i32,
    pub lacunarity: f32,
    pub gain: f32,
    pub fractal_type: FastNoiseFractalType,
    pub cellular_distance_function: FastNoiseCellularDistanceFunction,
    pub cellular_return_type: FastNoiseCellularReturnType,
    pub cellular_jitter: f32,
}

impl Default for FastNoiseGradientConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl FastNoiseGradientConfig {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            seed: DEFAULT_SEED,
            frequency: DEFAULT_FREQUENCY,
            interpolation: FastNoiseInterpolation::Quintic,
            noise_type: FastNoiseType::PerlinFractal,
            octaves: DEFAULT_OCTAVES,
            lacunarity: DEFAULT_LACUNARITY,
            gain: DEFAULT_GAIN,
            fractal_type: FastNoiseFractalType::Fbm,
            cellular_distance_function: FastNoiseCellularDistanceFunction::Euclidean,
            cellular_return_type: FastNoiseCellularReturnType::CellValue,
            cellular_jitter: DEFAULT_CELLULAR_JITTER,
        }
    }

    #[must_use]
    pub const fn normalized_seed(&self) -> u64 {
        // The branch already establishes `seed > 1`, so `unsigned_abs` is the
        // identity here and the widening to `u64` is exact.
        if self.seed > 1 {
            self.seed.unsigned_abs() as u64
        } else {
            1
        }
    }

    #[must_use]
    pub fn generator(&self) -> NativeFastNoise {
        let mut generator = NativeFastNoise::seeded(self.normalized_seed());
        generator.set_frequency(self.frequency);
        generator.set_interp(self.interpolation.to_generator());
        generator.set_noise_type(self.noise_type.to_generator());
        generator.set_fractal_octaves(self.octaves);
        generator.set_fractal_lacunarity(self.lacunarity);
        generator.set_fractal_gain(self.gain);
        generator.set_fractal_type(self.fractal_type.to_generator());
        generator.set_cellular_distance_function(self.cellular_distance_function.to_generator());
        generator.set_cellular_return_type(self.cellular_return_type.to_generator());
        generator.set_cellular_jitter(self.cellular_jitter);
        generator
    }

    #[must_use]
    pub fn sample_value(&self, position: Vec3) -> f32 {
        self.sample_value_with_generator(&self.generator(), position)
    }

    #[must_use]
    pub fn sample_value_with_generator(&self, generator: &NativeFastNoise, position: Vec3) -> f32 {
        ((generator.get_noise3d(position.x, position.y, position.z) + 1.0) * 0.5).clamp(0.0, 1.0)
    }
}

/// Runtime `FastNoise` gradient component.
///
/// O3DE reference: `Gems/FastNoise/Code/Source/FastNoiseGradientComponent.cpp:244`.
#[derive(Component, AzComponent, Debug, Clone, Copy, Default, PartialEq, Reflect, Prefab)]
#[az_component(
    name = "FastNoiseGem::FastNoiseGradientComponent",
    FAST_NOISE_GRADIENT_COMPONENT_TYPE_ID
)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.fast_noise.FastNoiseGradientComponent", version = 1)]
pub struct FastNoiseGradientComponent {
    pub configuration: FastNoiseGradientConfig,
}

impl FastNoiseGradientComponent {
    #[must_use]
    pub const fn new(configuration: FastNoiseGradientConfig) -> Self {
        Self { configuration }
    }

    #[must_use]
    pub fn sample_value(&self, position: Vec3) -> f32 {
        self.configuration.sample_value(position)
    }
}

/// Register `FastNoise` Gem components.
pub struct FastNoisePlugin;

impl Plugin for FastNoisePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FastNoiseInterpolation>()
            .register_type::<FastNoiseType>()
            .register_type::<FastNoiseFractalType>()
            .register_type::<FastNoiseCellularDistanceFunction>()
            .register_type::<FastNoiseCellularReturnType>()
            .register_type::<FastNoiseGradientConfig>()
            .register_type::<FastNoiseGradientComponent>();
    }
}

/// The AZ types this crate owns.
#[must_use]
pub const fn types() -> [az_core::AzTypeRegistration; 1] {
    [FastNoiseGradientConfig::REGISTRATION]
}

/// The component lowerings this crate owns.
#[must_use]
pub const fn lowerings() -> [az_core::ComponentLoweringRegistration; 1] {
    [FastNoiseGradientComponent::REGISTRATION]
}

/// The Bevy-native Prefab component types this crate owns.
#[must_use]
pub fn prefab_types() -> [PrefabType; 1] {
    [PrefabType::of::<FastNoiseGradientComponent>()]
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// The gradient component's AZ identity and its lowering travel with the gem
/// everywhere it is linked, which is why they are here and not in the
/// pipeline-only bundle below.
struct Package;

#[contribution(package)]
impl Contribution for Package {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_core::AzTypeRegistration>()
            .register_many(types());
        ctx.registrar::<az_core::ComponentLoweringRegistration>()
            .register_many(lowerings());
    }
}

/// Sealing is privacy: the generated `prefab_types_contribution` is the only
/// way in.
///
/// Reflected prefab types are read by the hosts that resolve prefab documents
/// without booting the runtime plugin graph — `project-host` through its type
/// registry, `asset-worker` through AZSCENE analysis — so they are the roles
/// this stanza names, and nothing here needs a Bevy world.
struct Prefabs;

#[contribution(prefab_types)]
impl Contribution for Prefabs {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<PrefabType>().register_many(prefab_types());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every float here is asserted against the very constant `new()` assigns to
    // it, so the values are bit-identical by construction; an epsilon would
    // weaken the assertion this test exists to make.
    #[allow(clippy::float_cmp)]
    #[test]
    fn fast_noise_config_defaults_match_source_defaults() {
        let config = FastNoiseGradientConfig::default();

        assert_eq!(config.seed, DEFAULT_SEED);
        assert_eq!(config.frequency, DEFAULT_FREQUENCY);
        assert_eq!(config.interpolation, FastNoiseInterpolation::Quintic);
        assert_eq!(config.noise_type, FastNoiseType::PerlinFractal);
        assert_eq!(config.octaves, DEFAULT_OCTAVES);
        assert_eq!(config.lacunarity, DEFAULT_LACUNARITY);
        assert_eq!(config.gain, DEFAULT_GAIN);
        assert_eq!(config.fractal_type, FastNoiseFractalType::Fbm);
        assert_eq!(
            config.cellular_distance_function,
            FastNoiseCellularDistanceFunction::Euclidean
        );
        assert_eq!(
            config.cellular_return_type,
            FastNoiseCellularReturnType::CellValue
        );
        assert_eq!(config.cellular_jitter, DEFAULT_CELLULAR_JITTER);
    }

    #[test]
    fn enum_native_values_match_fastnoise_source_order() {
        assert_eq!(FastNoiseType::CubicFractal.native(), 9);
        assert_eq!(
            FastNoiseType::from_native(3),
            Some(FastNoiseType::PerlinFractal)
        );
        assert_eq!(
            FastNoiseInterpolation::from_native(2),
            Some(FastNoiseInterpolation::Quintic)
        );
        assert_eq!(FastNoiseFractalType::RigidMulti.native(), 2);
        assert_eq!(
            FastNoiseCellularDistanceFunction::from_native(1),
            Some(FastNoiseCellularDistanceFunction::Manhattan)
        );
        assert_eq!(FastNoiseCellularReturnType::Distance2Div.native(), 7);
    }

    // Bit-identical output for identical input is exactly what this asserts;
    // an epsilon comparison would stop testing determinism.
    #[allow(clippy::float_cmp)]
    #[test]
    fn fast_noise_sampling_is_deterministic_and_normalized() {
        let config = FastNoiseGradientConfig {
            seed: 42,
            frequency: 0.25,
            noise_type: FastNoiseType::SimplexFractal,
            ..Default::default()
        };
        let position = Vec3::new(12.5, -3.25, 7.0);

        let first = config.sample_value(position);
        let second = config.sample_value(position);

        assert_eq!(first, second);
        assert!((0.0..=1.0).contains(&first));
    }
}
