//! Steam platform gem.
//!
//! Projects enable this gem when they need Steam identity, auth tickets, overlay,
//! or install/library discovery. Title-specific app IDs and launch policy remain
//! project settings.

#![forbid(unsafe_code)]

use az_gem_contract::{Contribution, GemContext, contribution};
use serde::{Deserialize, Serialize};

pub use steam_locator::{
    AppManifest, SteamApp, candidate_libraryfolders_vdf_paths, candidate_steam_library_dirs,
    candidate_steam_roots, find_steam_app, parse_appmanifest_acf, parse_libraryfolders_vdf,
};

#[cfg(feature = "runtime")]
pub use bevy_steamworks;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamAppId(pub u32);

impl SteamAppId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamPlatformSettings {
    pub app_id: SteamAppId,
}

impl SteamPlatformSettings {
    #[must_use]
    pub const fn new(app_id: u32) -> Self {
        Self {
            app_id: SteamAppId::new(app_id),
        }
    }
}

/// Builds the Steamworks Bevy plugin for the app id in `settings`.
///
/// # Errors
///
/// Returns the [`bevy_steamworks::SteamAPIInitError`] raised by
/// `SteamworksPlugin::init_app`: `NoSteamClient` if the Steam client is not
/// running or this process cannot reach it, `VersionMismatch` if the installed
/// Steam client is older than the SDK this links against, and `FailedGeneric`
/// for any other init failure — most often the process not being recognized as
/// owning `app_id`.
#[cfg(feature = "runtime")]
pub fn steamworks_plugin(
    settings: &SteamPlatformSettings,
) -> Result<bevy_steamworks::SteamworksPlugin, bevy_steamworks::SteamAPIInitError> {
    bevy_steamworks::SteamworksPlugin::init_app(settings.app_id.0)
}

/// The gem's `client-runtime` contribution.
///
/// Steam's content is settings data, the install locator, and — behind the
/// `runtime` feature — a Steamworks Bevy plugin. None of those is a registry
/// entry, so the registration surface is genuinely empty: this gem owns no AZ
/// type, asset type, component lowering, or prefab type. The feature decides
/// what the crate links, never whether the declared contribution exists.
///
/// Sealing is privacy: the generated `client_runtime_contribution` is the only
/// way in.
struct Client;

#[contribution]
impl Contribution for Client {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
