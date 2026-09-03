//! Entity-backed gradient sampling.

use az_gem_fast_noise::FastNoiseGradientComponent;
use az_gem_lmbr_central::ShapeLocalBoundsQuery;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::{
    ConstantGradientComponent, GradientSampleParams, GradientSampler, GradientTransformComponent,
    InvertGradientComponent, LevelsGradientComponent, PerlinGradientComponent,
    RandomGradientComponent, ThresholdGradientComponent, TransformType,
};

const MAX_GRADIENT_SOURCE_DEPTH: usize = 32;

/// Samples a gradient sampler against available gradient source entities.
///
/// O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/GradientSampler.h:92`.
pub trait GradientLookup {
    fn sample_gradient(&self, sampler: &GradientSampler, params: GradientSampleParams) -> f32;
}

/// Empty gradient lookup used when no source query is available.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGradientSources;

impl GradientLookup for NoGradientSources {
    fn sample_gradient(&self, sampler: &GradientSampler, _params: GradientSampleParams) -> f32 {
        sampler.apply_embedded_operations(0.0)
    }
}

/// Bevy query set for `GradientSignal` source components.
///
/// O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/Ebuses/GradientRequestBus.h:54`.
#[derive(SystemParam)]
pub struct GradientSourceQuery<'w, 's> {
    constants: Query<'w, 's, &'static ConstantGradientComponent>,
    thresholds: Query<'w, 's, &'static ThresholdGradientComponent>,
    inverts: Query<'w, 's, &'static InvertGradientComponent>,
    levels: Query<'w, 's, &'static LevelsGradientComponent>,
    perlins: Query<'w, 's, &'static PerlinGradientComponent>,
    randoms: Query<'w, 's, &'static RandomGradientComponent>,
    fast_noises: Query<'w, 's, &'static FastNoiseGradientComponent>,
    gradient_transforms: Query<'w, 's, &'static GradientTransformComponent>,
    shape_bounds: ShapeLocalBoundsQuery<'w, 's>,
    transforms: Query<'w, 's, (Option<&'static Transform>, Option<&'static GlobalTransform>)>,
}

impl GradientLookup for GradientSourceQuery<'_, '_> {
    fn sample_gradient(&self, sampler: &GradientSampler, params: GradientSampleParams) -> f32 {
        let mut stack = GradientSourceStack::default();
        self.sample_sampler(sampler, params, &mut stack)
    }
}

impl GradientSourceQuery<'_, '_> {
    fn sample_sampler(
        &self,
        sampler: &GradientSampler,
        params: GradientSampleParams,
        stack: &mut GradientSourceStack,
    ) -> f32 {
        let sampled_value = match sampler.gradient {
            Some(entity) => {
                let params = sampler.transform_params(params);
                match self.sample_source(entity, params, stack) {
                    GradientSourceSample::Value(value) => value,
                    GradientSourceSample::Missing => 0.0,
                    GradientSourceSample::Blocked => return 0.0,
                }
            }
            None => 0.0,
        };

        sampler.apply_embedded_operations(sampled_value)
    }

    fn sample_source(
        &self,
        entity: Entity,
        params: GradientSampleParams,
        stack: &mut GradientSourceStack,
    ) -> GradientSourceSample {
        if !stack.push(entity) {
            return GradientSourceSample::Blocked;
        }

        let value = self.source_value(entity, params, stack);
        stack.pop(entity);
        value
    }

    fn source_value(
        &self,
        entity: Entity,
        params: GradientSampleParams,
        stack: &mut GradientSourceStack,
    ) -> GradientSourceSample {
        if let Ok(component) = self.constants.get(entity) {
            return GradientSourceSample::Value(component.sample_value());
        }
        if let Ok(component) = self.thresholds.get(entity) {
            let sample = self.sample_sampler(&component.configuration.gradient, params, stack);
            return GradientSourceSample::Value(component.configuration.apply_threshold(sample));
        }
        if let Ok(component) = self.inverts.get(entity) {
            let sample = self.sample_sampler(&component.configuration.gradient, params, stack);
            return GradientSourceSample::Value(component.configuration.apply_invert(sample));
        }
        if let Ok(component) = self.levels.get(entity) {
            let sample = self.sample_sampler(&component.configuration.gradient, params, stack);
            return GradientSourceSample::Value(component.configuration.apply_levels(sample));
        }
        if let Ok(component) = self.perlins.get(entity) {
            let params = match self.transform_source_params(entity, params, false) {
                GradientSourceTransform::Params(params) => params,
                GradientSourceTransform::Rejected => return GradientSourceSample::Value(0.0),
            };
            return GradientSourceSample::Value(component.sample_value(params));
        }
        if let Ok(component) = self.randoms.get(entity) {
            let params = match self.transform_source_params(entity, params, false) {
                GradientSourceTransform::Params(params) => params,
                GradientSourceTransform::Rejected => return GradientSourceSample::Value(0.0),
            };
            return GradientSourceSample::Value(component.sample_value(params));
        }
        if let Ok(component) = self.fast_noises.get(entity) {
            let params = match self.transform_source_params(entity, params, false) {
                GradientSourceTransform::Params(params) => params,
                GradientSourceTransform::Rejected => return GradientSourceSample::Value(0.0),
            };
            return GradientSourceSample::Value(component.sample_value(params.position));
        }

        GradientSourceSample::Missing
    }

    fn transform_source_params(
        &self,
        entity: Entity,
        params: GradientSampleParams,
        should_normalize_output: bool,
    ) -> GradientSourceTransform {
        let Ok(gradient_transform) = self.gradient_transforms.get(entity) else {
            return GradientSourceTransform::Params(params);
        };

        let shape_entity = gradient_transform.configuration.shape_entity(entity);
        let transform = self.gradient_transform(entity, shape_entity, gradient_transform);
        let bounds = gradient_transform
            .configuration
            .local_bounds_from_shape(self.shape_bounds.local_bounds(shape_entity));
        let transformed = gradient_transform.transform_position_to_uvw_in_bounds(
            params.position,
            transform,
            bounds,
            should_normalize_output,
        );
        if transformed.rejected {
            GradientSourceTransform::Rejected
        } else {
            GradientSourceTransform::Params(GradientSampleParams {
                position: transformed.uvw,
            })
        }
    }

    fn gradient_transform(
        &self,
        entity: Entity,
        shape_entity: Entity,
        gradient_transform: &GradientTransformComponent,
    ) -> Transform {
        let config = &gradient_transform.configuration;
        let owner = self.local_and_world_transform(entity);
        let reference = self.local_and_world_transform(shape_entity);
        let selected = match config.transform_type {
            TransformType::WorldOrigin | TransformType::Relative => Transform::IDENTITY,
            TransformType::LocalThisEntity => owner.local,
            TransformType::WorldThisEntity => owner.world,
            TransformType::LocalReferenceEntity => reference.local,
            TransformType::WorldReferenceEntity => reference.world,
        };

        config.transform_from_bevy(selected, owner.world, reference.world)
    }

    fn local_and_world_transform(&self, entity: Entity) -> LocalAndWorldTransform {
        let Ok((local, global)) = self.transforms.get(entity) else {
            return LocalAndWorldTransform::default();
        };
        let local = local.copied().unwrap_or_default();
        let world = global.map_or(local, GlobalTransform::compute_transform);

        LocalAndWorldTransform { local, world }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GradientSourceSample {
    Value(f32),
    Missing,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GradientSourceTransform {
    Params(GradientSampleParams),
    Rejected,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct LocalAndWorldTransform {
    local: Transform,
    world: Transform,
}

#[derive(Debug, Clone, Copy)]
struct GradientSourceStack {
    entities: [Option<Entity>; MAX_GRADIENT_SOURCE_DEPTH],
    len: usize,
}

impl Default for GradientSourceStack {
    fn default() -> Self {
        Self {
            entities: [None; MAX_GRADIENT_SOURCE_DEPTH],
            len: 0,
        }
    }
}

impl GradientSourceStack {
    fn push(&mut self, entity: Entity) -> bool {
        if self.entities[..self.len].contains(&Some(entity))
            || self.len == MAX_GRADIENT_SOURCE_DEPTH
        {
            return false;
        }

        self.entities[self.len] = Some(entity);
        self.len += 1;
        true
    }

    fn pop(&mut self, entity: Entity) {
        debug_assert!(self.len > 0);
        debug_assert_eq!(self.entities[self.len - 1], Some(entity));
        self.len -= 1;
        self.entities[self.len] = None;
    }
}
