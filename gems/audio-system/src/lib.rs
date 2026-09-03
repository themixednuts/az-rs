//! `AudioSystem` Gem runtime request surface.
//!
//! O3DE reference: `Gems/AudioSystem/Code/Source/AudioSystemGemSystemComponent.h`.

mod control;
mod plugin;
mod request;
mod wwise;

#[cfg(test)]
mod tests;

use az_gem_contract::{Contribution, GemContext, contribution};

pub use control::{AudioControlId, AudioObstructionType, INVALID_AUDIO_CONTROL_ID};
pub use plugin::{AudioSystemAssetPlugin, AudioSystemPlugin};
pub use request::{AudioRequest, RecordedAudioRequests};
pub use wwise::*;

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// The Wwise bank, media, and control-file readers are Bevy assets with Bevy
/// loaders, installed by [`AudioSystemAssetPlugin`] rather than composed: no
/// engine asset type, product format, or source schema names them, so this gem
/// owns no registry entry. An empty `register` is the honest shape of that, and
/// the compose-seam test holds it to empty so a registration added later has to
/// be declared here.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
