//! `LyShine` Gem UI boot data and Bevy integration.

mod boot;
mod canvas;
mod canvas_binary;
mod font_descriptor;
mod lua;
mod render;
pub mod script;
mod sprite_border;
mod ui_spawner;

use az_gem_contract::contribution;

pub use boot::*;
pub use canvas::*;
pub use font_descriptor::{
    LYSHINE_FONT_DESCRIPTOR_EXTENSIONS, LyShineFontDescriptorError, LyShineFontDescriptorLoader,
    is_font_descriptor_path,
};
pub use lua::*;
pub use render::*;
pub use script::{LYSHINE_LUA_BUSES, LyShineScriptingPlugin, add_lyshine_bus_init};
pub use sprite_border::{
    LYSHINE_SPRITE_EXTENSIONS, LyShineSpriteAsset, LyShineSpriteAssetError,
    LyShineSpriteAssetLoader, pixel_border_rect, sprite_sidecar_path,
};
pub use ui_spawner::*;

/// The AZ types this crate owns.
#[must_use]
pub const fn types() -> [az_core::AzTypeRegistration; 1] {
    [UiSpawnerComponent::REGISTRATION]
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// The gem's second contribution, `builders`, lives in the bundle crate beside
/// this one: `lyshine-canvas` depends on this crate, so a block here could not
/// name it.
struct Package;

#[contribution]
impl az_gem_contract::Contribution for Package {
    fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_core::AzTypeRegistration>()
            .register_many(types());
    }
}
