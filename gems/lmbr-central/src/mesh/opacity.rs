//! Runtime `MeshComponentRequestBus::SetOpacity` behavior.

use bevy::asset::AssetId;
use bevy::ecs::entity_disabling::Disabled;
use bevy::prelude::*;

use super::StaticModelChildren;

/// Entity-addressed mesh opacity transition.
///
/// This is the Bevy boundary for the native
/// `LmbrCentral::MeshComponentRequestBus::SetOpacity` service. Callers target
/// the logical component entity; `LmbrCentral` owns descendant traversal and the
/// render-material implementation.
#[derive(EntityEvent, Debug, Clone, Copy, PartialEq)]
pub struct MeshOpacityRequest {
    pub entity: Entity,
    starting_opacity: f32,
    target_opacity: f32,
    duration_seconds: f32,
    apply_on_children: bool,
}

impl MeshOpacityRequest {
    #[must_use]
    pub const fn new(
        entity: Entity,
        starting_opacity: f32,
        target_opacity: f32,
        duration_seconds: f32,
        apply_on_children: bool,
    ) -> Self {
        Self {
            entity,
            starting_opacity,
            target_opacity,
            duration_seconds,
            apply_on_children,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub(super) struct MeshOpacityTransition {
    starting_opacity: f32,
    target_opacity: f32,
    duration_seconds: f32,
    elapsed_seconds: f32,
    apply_on_children: bool,
}

impl From<&MeshOpacityRequest> for MeshOpacityTransition {
    fn from(request: &MeshOpacityRequest) -> Self {
        Self {
            starting_opacity: request.starting_opacity,
            target_opacity: request.target_opacity,
            duration_seconds: request.duration_seconds,
            elapsed_seconds: 0.0,
            apply_on_children: request.apply_on_children,
        }
    }
}

impl MeshOpacityTransition {
    fn advance(&mut self, delta_seconds: f32) -> (f32, bool) {
        if self.duration_seconds <= 0.0 {
            return (self.target_opacity, true);
        }

        self.elapsed_seconds = (self.elapsed_seconds + delta_seconds).min(self.duration_seconds);
        let fraction = self.elapsed_seconds / self.duration_seconds;
        (
            (self.target_opacity - self.starting_opacity).mul_add(fraction, self.starting_opacity),
            self.elapsed_seconds >= self.duration_seconds,
        )
    }
}

/// Per-render-entity ownership of the material instance used for opacity.
///
/// `MeshMaterial3d` handles are not assumed to be unique. The first opacity
/// operation clones the source material, preserving entity-local request-bus
/// semantics and avoiding mutations to materials shared by unrelated meshes.
#[derive(Component, Debug, Clone)]
pub(super) struct MeshOpacityMaterialBinding {
    material: AssetId<StandardMaterial>,
    base_alpha: f32,
    base_alpha_mode: AlphaMode,
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`On` is an owned Bevy observer parameter; a reference stops this satisfying `IntoObserverSystem`"
)]
pub(super) fn begin_mesh_opacity_transition(event: On<MeshOpacityRequest>, mut commands: Commands) {
    commands
        .entity(event.entity)
        .insert(MeshOpacityTransition::from(&*event));
}

#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub(super) fn update_mesh_opacity_transitions(
    mut commands: Commands,
    time: Option<Res<Time>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut transitions: Query<(Entity, &mut MeshOpacityTransition), Allow<Disabled>>,
    children: Query<&Children, Allow<Disabled>>,
    static_model_children: Query<&StaticModelChildren, Allow<Disabled>>,
    mut rendered: Query<
        (
            &MeshMaterial3d<StandardMaterial>,
            Option<&mut MeshOpacityMaterialBinding>,
        ),
        Allow<Disabled>,
    >,
) {
    let (Some(time), Some(materials)) = (time.as_deref(), materials.as_deref_mut()) else {
        return;
    };

    for (entity, mut transition) in &mut transitions {
        let (opacity, complete) = transition.advance(time.delta_secs());
        for target in descendant_preorder(entity, transition.apply_on_children, &children) {
            apply_opacity_to_render_entity(
                target,
                opacity,
                &mut commands,
                materials,
                &mut rendered,
            );
            if let Ok(visuals) = static_model_children.get(target) {
                for visual in &visuals.0 {
                    apply_opacity_to_render_entity(
                        *visual,
                        opacity,
                        &mut commands,
                        materials,
                        &mut rendered,
                    );
                }
            }
        }
        if complete {
            commands.entity(entity).remove::<MeshOpacityTransition>();
        }
    }
}

fn apply_opacity_to_render_entity(
    entity: Entity,
    opacity: f32,
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    rendered: &mut Query<
        (
            &MeshMaterial3d<StandardMaterial>,
            Option<&mut MeshOpacityMaterialBinding>,
        ),
        Allow<Disabled>,
    >,
) {
    let Ok((material_handle, binding)) = rendered.get_mut(entity) else {
        return;
    };
    let material_id = material_handle.id();

    if let Some(binding) = binding
        && binding.material == material_id
    {
        let Some(mut material) = materials.get_mut(material_id) else {
            return;
        };
        set_material_opacity(
            &mut material,
            binding.base_alpha,
            binding.base_alpha_mode,
            opacity,
        );
        return;
    }

    let (base_alpha, base_alpha_mode, mut owned) = {
        let Some(source) = materials.get(material_id) else {
            return;
        };
        (source.base_color.alpha(), source.alpha_mode, source.clone())
    };
    set_material_opacity(&mut owned, base_alpha, base_alpha_mode, opacity);
    let owned = materials.add(owned);
    let owned_id = owned.id();
    commands.entity(entity).insert((
        MeshMaterial3d(owned),
        MeshOpacityMaterialBinding {
            material: owned_id,
            base_alpha,
            base_alpha_mode,
        },
    ));
}

fn set_material_opacity(
    material: &mut StandardMaterial,
    base_alpha: f32,
    base_alpha_mode: AlphaMode,
    opacity: f32,
) {
    let alpha = (base_alpha * opacity).clamp(0.0, 1.0);
    material.base_color = material.base_color.with_alpha(alpha);
    material.alpha_mode = if alpha < 1.0 {
        AlphaMode::Blend
    } else {
        base_alpha_mode
    };
}

fn descendant_preorder(
    root: Entity,
    recurse: bool,
    children: &Query<&Children, Allow<Disabled>>,
) -> Vec<Entity> {
    let mut result = vec![root];
    if !recurse {
        return result;
    }

    let mut stack = children
        .get(root)
        .map(|children| children.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    stack.reverse();
    while let Some(entity) = stack.pop() {
        result.push(entity);
        if let Ok(children) = children.get(entity) {
            let mut descendants = children.iter().collect::<Vec<_>>();
            descendants.reverse();
            stack.extend(descendants);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_interpolates_linearly_and_finishes_at_target() {
        let request = MeshOpacityRequest::new(Entity::PLACEHOLDER, 0.25, 0.75, 2.0, false);
        let mut transition = MeshOpacityTransition::from(&request);

        assert_eq!(transition.advance(0.5), (0.375, false));
        assert_eq!(transition.advance(1.5), (0.75, true));
        assert_eq!(transition.advance(1.0), (0.75, true));
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn zero_duration_applies_target_immediately() {
        let request = MeshOpacityRequest::new(Entity::PLACEHOLDER, 0.25, 0.75, 0.0, true);
        let mut transition = MeshOpacityTransition::from(&request);

        assert_eq!(transition.advance(0.0), (0.75, true));
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn material_opacity_preserves_base_alpha_and_mode() {
        let mut material = StandardMaterial {
            base_color: Color::WHITE.with_alpha(0.5),
            alpha_mode: AlphaMode::Mask(0.25),
            ..Default::default()
        };
        let base_mode = material.alpha_mode;

        set_material_opacity(&mut material, 0.5, base_mode, 0.4);
        assert_eq!(material.base_color.alpha(), 0.2);
        assert_eq!(material.alpha_mode, AlphaMode::Blend);

        set_material_opacity(&mut material, 0.5, base_mode, 2.0);
        assert_eq!(material.base_color.alpha(), 1.0);
        assert_eq!(material.alpha_mode, base_mode);
    }
}
