//! `GradientSignal` Gem runtime data.
//!
//! O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/GradientSampler.h`.

mod components;
mod noise;
mod plugin;
mod sampler;
mod sampling;
mod transform;
mod type_ids;
mod util;

use az_gem_contract::{Contribution, GemContext, contribution};
use az_prefab::PrefabType;

pub use components::*;
pub use noise::*;
pub use plugin::*;
pub use sampler::*;
pub use sampling::*;
pub use transform::*;
pub use type_ids::*;
pub use util::*;

/// The Bevy-native Prefab component types this crate owns.
#[must_use]
pub fn prefab_types() -> [PrefabType; 7] {
    [
        PrefabType::of::<ConstantGradientComponent>(),
        PrefabType::of::<ThresholdGradientComponent>(),
        PrefabType::of::<InvertGradientComponent>(),
        PrefabType::of::<LevelsGradientComponent>(),
        PrefabType::of::<RandomGradientComponent>(),
        PrefabType::of::<PerlinGradientComponent>(),
        PrefabType::of::<GradientTransformComponent>(),
    ]
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// Gradient sampling is Bevy components and the code that reads them; the
/// reflected half is the pipeline-only bundle below, and nothing else here is a
/// registry entry. An empty `register` is the honest shape of that, and the
/// compose-seam test holds it to empty so a registration added later has to be
/// declared here.
struct Package;

#[contribution(package)]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

/// Sealing is privacy: the generated `prefab_types_contribution` is the only
/// way in.
///
/// Reflected prefab types are read by the two hosts that resolve prefab
/// documents without booting the runtime plugin graph — `project-host` through
/// its type registry, `asset-worker` through AZSCENE analysis.
struct Prefabs;

#[contribution(prefab_types)]
impl Contribution for Prefabs {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<PrefabType>().register_many(prefab_types());
    }
}

#[cfg(test)]
mod tests;
