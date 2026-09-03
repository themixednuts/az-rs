//! Footsteps Gem component data and Bevy integration.
//!
//! Lumberyard reference: `dev/Gems/Footsteps/Code/Source/FootstepComponent.h`.

use az_gem_contract::{Contribution, GemContext, contribution};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

/// Lumberyard `Footsteps::FootstepComponent` AZ component UUID.
pub const FOOTSTEP_COMPONENT_TYPE_ID: Uuid = uuid!("C5635496-0A46-4D2C-8CE3-D7F642E54FC5");

/// Component configuration for animation-driven footstep effects.
///
/// Lumberyard reference: `dev/Gems/Footsteps/Code/Source/FootstepComponent.h:25`.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct FootstepComponent {
    pub play_from_bone: bool,
    pub always_play: bool,
    pub raycast_collision_filter: Option<String>,
    pub override_footstep_character: Option<String>,
    #[entities]
    pub physics_entity_override: Option<Entity>,
    pub custom_fx_lib_character_tag: Option<String>,
}

impl Default for FootstepComponent {
    fn default() -> Self {
        Self {
            play_from_bone: true,
            always_play: false,
            raycast_collision_filter: None,
            override_footstep_character: None,
            physics_entity_override: None,
            custom_fx_lib_character_tag: None,
        }
    }
}

/// Register Footsteps Gem components.
pub struct FootstepsPlugin;

impl Plugin for FootstepsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FootstepComponent>();
    }
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// Footsteps owns one reflected component and nothing a host keeps in a
/// registry — no asset type, no build rule, no prefab type — so the whole of
/// what this contribution gives a host is the linkage the named call proves.
/// An empty `register` is the honest shape of that, and the compose-seam test
/// holds it to empty so a registration added later has to be declared here.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footstep_component_defaults_preserve_native_playback_behavior() {
        let component = FootstepComponent::default();

        assert!(component.play_from_bone);
        assert!(!component.always_play);
        assert!(component.raycast_collision_filter.is_none());
        assert!(component.override_footstep_character.is_none());
        assert!(component.physics_entity_override.is_none());
        assert!(component.custom_fx_lib_character_tag.is_none());
    }
}
