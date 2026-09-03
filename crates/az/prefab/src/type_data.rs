//! Prefab-specific data stored on normal Bevy type registrations.

use std::{any::Any, panic::AssertUnwindSafe};

use bevy_ecs::{
    reflect::{ReflectComponent, ReflectFromWorld},
    template::{SceneEntityReferences, TemplateContext},
    world::EntityWorldMut,
};
use bevy_reflect::{
    FromReflect, PartialReflect, Reflect, TypeInfo, TypeRegistration, TypeRegistry, Typed,
    std_traits::ReflectDefault,
};
use thiserror::Error;

/// An old source tag and the source version encoded under that tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefabTagAlias {
    pub tag: &'static str,
    pub source_version: u32,
}

/// A sparse reflected payload paired with the type information it represents.
pub struct ErasedPrefabValue {
    pub value: Box<dyn PartialReflect>,
    pub type_info: &'static TypeInfo,
}

impl ErasedPrefabValue {
    /// Creates a payload only when the reflected value carries represented type
    /// information, as all Prefab source values must.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabBuildError::MissingRepresentedTypeInfo`] when `value`
    /// has no represented `TypeInfo`, which is the case for bare dynamic values.
    pub fn try_new(value: Box<dyn PartialReflect>) -> Result<Self, PrefabBuildError> {
        let type_info = value
            .get_represented_type_info()
            .ok_or(PrefabBuildError::MissingRepresentedTypeInfo)?;
        Ok(Self { value, type_info })
    }
}

pub type ErasedPrefabMigrationFn =
    fn(ErasedPrefabValue) -> Result<ErasedPrefabValue, PrefabBuildError>;

#[derive(Clone, Copy)]
pub struct PrefabMigrationStep {
    pub from_version: u32,
    pub to_version: u32,
    pub migrate: ErasedPrefabMigrationFn,
}

impl std::fmt::Debug for PrefabMigrationStep {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrefabMigrationStep")
            .field("from_version", &self.from_version)
            .field("to_version", &self.to_version)
            .finish_non_exhaustive()
    }
}

/// Construction capabilities required by this registration.
#[derive(Debug, Clone, Copy)]
pub enum PrefabConstruction {
    ReflectDefaultOrFromWorld,
    Template {
        template_type_info: fn() -> &'static TypeInfo,
    },
}

/// Controls whether a constructed Prefab component is emitted into the
/// cooked runtime scene product.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrefabProductPolicy {
    /// Include the component in the cooked runtime scene product.
    #[default]
    Runtime,
    /// Retain the component for source decode, composition, validation, and
    /// construction, but omit it from the cooked runtime scene product.
    SourceOnly,
}

pub type ErasedPrefabConstructFn = for<'a, 'w> fn(
    &TypeRegistration,
    &dyn PartialReflect,
    &'a mut EntityWorldMut<'w>,
    &'a mut SceneEntityReferences,
) -> Result<ErasedPrefabValue, PrefabBuildError>;

pub type ErasedPrefabInsertFn = for<'w> fn(
    &TypeRegistration,
    &dyn PartialReflect,
    &mut EntityWorldMut<'w>,
    &TypeRegistry,
) -> Result<(), PrefabBuildError>;

/// The complete Prefab-specific extension on a Bevy type registration.
#[derive(Clone, Copy)]
pub struct PrefabTypeData {
    pub tag: &'static str,
    pub source_version: u32,
    pub aliases: &'static [PrefabTagAlias],
    pub migrations: &'static [PrefabMigrationStep],
    pub construction: PrefabConstruction,
    pub product_policy: PrefabProductPolicy,
    pub construct: ErasedPrefabConstructFn,
    pub insert: ErasedPrefabInsertFn,
}

impl std::fmt::Debug for PrefabTypeData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrefabTypeData")
            .field("tag", &self.tag)
            .field("source_version", &self.source_version)
            .field("aliases", &self.aliases)
            .field("migrations", &self.migrations)
            .field("construction", &self.construction)
            .field("product_policy", &self.product_policy)
            .finish_non_exhaustive()
    }
}

/// Structured construction and materialization failures.
#[derive(Debug, Error)]
pub enum PrefabBuildError {
    #[error("reflected Prefab payload has no represented TypeInfo")]
    MissingRepresentedTypeInfo,
    #[error("Prefab payload represents `{actual}`, expected `{expected}`")]
    TypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("Prefab type `{type_path}` has neither ReflectDefault nor ReflectFromWorld")]
    MissingConstructionTypeData { type_path: &'static str },
    #[error("Prefab type `{type_path}` is missing ReflectComponent")]
    MissingReflectComponent { type_path: &'static str },
    #[error("failed to apply sparse Prefab patch to `{type_path}`: {message}")]
    ApplyFailed {
        type_path: &'static str,
        message: String,
    },
    #[error("failed to reconstruct template `{type_path}` from its sparse value")]
    TemplateFromReflectFailed { type_path: &'static str },
    #[error("Prefab construction failed for `{type_path}`: {message}")]
    ConstructionFailed {
        type_path: &'static str,
        message: String,
    },
    #[error("Prefab insertion failed for `{type_path}`: {message}")]
    InsertFailed {
        type_path: &'static str,
        message: String,
    },
}

/// Builds an ordinary component from reflected Default/FromWorld data and
/// applies the sparse source patch.
///
/// # Errors
///
/// Returns [`PrefabBuildError::MissingRepresentedTypeInfo`] or
/// [`PrefabBuildError::TypeMismatch`] when `sparse` does not represent
/// `registration`'s type, [`PrefabBuildError::MissingConstructionTypeData`]
/// when the registration has neither `ReflectDefault` nor `ReflectFromWorld`,
/// and [`PrefabBuildError::ApplyFailed`] when the sparse patch does not apply
/// to the constructed value.
pub fn construct_reflected(
    registration: &TypeRegistration,
    sparse: &dyn PartialReflect,
    entity: &mut EntityWorldMut<'_>,
    _references: &mut SceneEntityReferences,
) -> Result<ErasedPrefabValue, PrefabBuildError> {
    require_represented_type(registration.type_info(), sparse)?;
    let mut value = if let Some(default) = registration.data::<ReflectDefault>() {
        default.default()
    } else if let Some(from_world) = registration.data::<ReflectFromWorld>() {
        entity.world_scope(|world| from_world.from_world(world))
    } else {
        return Err(PrefabBuildError::MissingConstructionTypeData {
            type_path: registration.type_info().type_path(),
        });
    };
    value
        .as_partial_reflect_mut()
        .try_apply(sparse)
        .map_err(|error| PrefabBuildError::ApplyFailed {
            type_path: registration.type_info().type_path(),
            message: error.to_string(),
        })?;
    ErasedPrefabValue::try_new(value.into_partial_reflect())
}

/// Reconstructs a typed Bevy template and invokes the explicitly registered
/// construction adapter.
///
/// # Errors
///
/// Returns [`PrefabBuildError::MissingRepresentedTypeInfo`] or
/// [`PrefabBuildError::TypeMismatch`] when `sparse` does not represent
/// `Template`, [`PrefabBuildError::TemplateFromReflectFailed`] when
/// `Template::from_reflect` rejects the sparse value, and
/// any [`PrefabBuildError`] returned by `adapter`.
pub fn construct_from_template<Template, Output>(
    _registration: &TypeRegistration,
    sparse: &dyn PartialReflect,
    entity: &mut EntityWorldMut<'_>,
    references: &mut SceneEntityReferences,
    adapter: for<'a, 'w> fn(
        &Template,
        &mut TemplateContext<'a, 'w>,
    ) -> Result<Output, PrefabBuildError>,
) -> Result<ErasedPrefabValue, PrefabBuildError>
where
    Template: FromReflect + Typed,
    Output: Reflect,
{
    require_represented_type(Template::type_info(), sparse)?;
    let template = Template::from_reflect(sparse).ok_or_else(|| {
        PrefabBuildError::TemplateFromReflectFailed {
            type_path: Template::type_info().type_path(),
        }
    })?;
    let mut context = TemplateContext::new(entity, references);
    let output = adapter(&template, &mut context)?;
    ErasedPrefabValue::try_new(Box::new(output).into_partial_reflect())
}

/// Inserts a constructed reflected component through Bevy's `ReflectComponent`.
///
/// # Errors
///
/// Returns [`PrefabBuildError::MissingRepresentedTypeInfo`] or
/// [`PrefabBuildError::TypeMismatch`] when `value` does not represent
/// `registration`'s type, [`PrefabBuildError::MissingReflectComponent`] when
/// the registration carries no `ReflectComponent`, and
/// [`PrefabBuildError::InsertFailed`] when the reflected insertion itself
/// panics — that panic is caught rather than propagated.
pub fn insert_reflected_component(
    registration: &TypeRegistration,
    value: &dyn PartialReflect,
    entity: &mut EntityWorldMut<'_>,
    registry: &TypeRegistry,
) -> Result<(), PrefabBuildError> {
    require_represented_type(registration.type_info(), value)?;
    let reflect_component = registration.data::<ReflectComponent>().ok_or_else(|| {
        PrefabBuildError::MissingReflectComponent {
            type_path: registration.type_info().type_path(),
        }
    })?;
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        reflect_component.insert(entity, value, registry);
    }))
    .map_err(|payload| PrefabBuildError::InsertFailed {
        type_path: registration.type_info().type_path(),
        message: panic_message(&*payload),
    })
}

fn require_represented_type(
    expected: &'static TypeInfo,
    value: &dyn PartialReflect,
) -> Result<(), PrefabBuildError> {
    let actual = value
        .get_represented_type_info()
        .ok_or(PrefabBuildError::MissingRepresentedTypeInfo)?;
    if actual.type_id() != expected.type_id() {
        return Err(PrefabBuildError::TypeMismatch {
            expected: expected.type_path(),
            actual: actual.type_path(),
        });
    }
    Ok(())
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    "unknown ReflectComponent panic".to_owned()
}

#[cfg(test)]
mod tests {
    use bevy_ecs::{component::Component, reflect::AppTypeRegistry, world::World};
    use bevy_reflect::{
        FromType, GetTypeRegistration, Reflect, TypeRegistry, structs::DynamicStruct,
    };

    use super::*;

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct DirectComponent {
        amount: f32,
        enabled: bool,
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "this asserts the sparse patch wrote the exact authored value; an epsilon would let a corrupted value pass"
    )]
    fn default_construction_applies_sparse_patch_and_inserts_component() {
        let mut registry = TypeRegistry::default();
        registry.register::<DirectComponent>();
        let registration = registry
            .get(std::any::TypeId::of::<DirectComponent>())
            .expect("component registration");
        let mut sparse = DynamicStruct::default();
        sparse.set_represented_type(Some(DirectComponent::type_info()));
        sparse.insert("amount", 4.5_f32);

        let mut world = World::new();
        world.insert_resource(AppTypeRegistry::default());
        let entity = world.spawn_empty().id();
        let mut references = SceneEntityReferences::default();
        let built = {
            let mut entity = world.entity_mut(entity);
            construct_reflected(registration, &sparse, &mut entity, &mut references)
                .expect("construct component")
        };
        {
            let mut entity = world.entity_mut(entity);
            insert_reflected_component(registration, built.value.as_ref(), &mut entity, &registry)
                .expect("insert component");
        }

        let component = world
            .get::<DirectComponent>(entity)
            .expect("inserted component");
        assert_eq!(component.amount, 4.5);
        assert!(!component.enabled);
    }

    #[test]
    fn prefab_type_data_is_cloneable_bevy_type_data() {
        let data = PrefabTypeData {
            tag: "Direct",
            source_version: 1,
            aliases: &[],
            migrations: &[],
            construction: PrefabConstruction::ReflectDefaultOrFromWorld,
            product_policy: PrefabProductPolicy::Runtime,
            construct: construct_reflected,
            insert: insert_reflected_component,
        };
        let mut registration = DirectComponent::get_type_registration();
        registration.insert(data);
        assert_eq!(
            registration
                .data::<PrefabTypeData>()
                .expect("Prefab TypeData")
                .tag,
            "Direct"
        );
        let _ = <ReflectDefault as FromType<DirectComponent>>::from_type();
    }
}
