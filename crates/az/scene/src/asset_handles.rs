//! Reflection processors for stable handles and product-local entities.

use std::{
    any::TypeId,
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

use az_asset::{AssetId as AzAssetId, ReflectAssetDependency, UntypedAssetRef};
use bevy::{
    asset::{
        EphemeralHandleBehavior, HandleDeserializeProcessor, HandleSerializeProcessor,
        LoadFromPath, ReflectHandle, UntypedHandle,
    },
    ecs::entity::Entity,
    reflect::{
        PartialReflect, ReflectRef, TypeRegistration, TypeRegistry,
        serde::{ReflectDeserializerProcessor, ReflectSerializerProcessor, TypedReflectSerializer},
    },
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::Error as _,
    ser::{Error as _, SerializeMap, SerializeSeq},
};

use crate::structural_serde;

pub struct SceneSerializeProcessor<'a> {
    entity_ids: &'a BTreeMap<Entity, u32>,
    dependencies: RefCell<BTreeSet<String>>,
    asset_dependencies: RefCell<BTreeSet<AzAssetId>>,
    handles: HandleSerializeProcessor,
}

impl<'a> SceneSerializeProcessor<'a> {
    pub(crate) const fn new(entity_ids: &'a BTreeMap<Entity, u32>) -> Self {
        Self {
            entity_ids,
            dependencies: RefCell::new(BTreeSet::new()),
            asset_dependencies: RefCell::new(BTreeSet::new()),
            handles: HandleSerializeProcessor {
                ephemeral_handle_behavior: EphemeralHandleBehavior::Error,
            },
        }
    }

    pub(crate) fn dependencies(&self) -> Vec<String> {
        self.dependencies.borrow().iter().cloned().collect()
    }

    pub(crate) fn asset_dependencies(&self) -> Vec<AzAssetId> {
        self.asset_dependencies.borrow().iter().copied().collect()
    }

    fn capture_asset_dependency(&self, value: &dyn PartialReflect, registry: &TypeRegistry) {
        let Some(value) = value.try_as_reflect() else {
            return;
        };
        let dependency_id = value
            .downcast_ref::<UntypedAssetRef>()
            .map(|reference| reference.asset_id)
            .filter(|asset_id| asset_id.is_valid())
            .or_else(|| {
                registry
                    .get(value.type_id())
                    .and_then(|registration| registration.data::<ReflectAssetDependency>())
                    .and_then(|dependency| dependency.dependency_id(value))
            });
        if let Some(dependency_id) = dependency_id {
            self.asset_dependencies.borrow_mut().insert(dependency_id);
        }
    }

    fn capture_handle(&self, value: &dyn PartialReflect, registry: &TypeRegistry) {
        let Some(value) = value.try_as_reflect() else {
            return;
        };
        let handle = value.downcast_ref::<UntypedHandle>().cloned().or_else(|| {
            registry
                .get(value.type_id())
                .and_then(|registration| registration.data::<ReflectHandle>())
                .and_then(|handle| handle.downcast_handle_untyped(value.as_any()))
        });
        if let Some(path) = handle.as_ref().and_then(UntypedHandle::path) {
            self.dependencies.borrow_mut().insert(path.to_string());
        }
    }

    fn canonical_bytes(
        &self,
        value: &dyn PartialReflect,
        registry: &TypeRegistry,
    ) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(&TypedReflectSerializer::with_processor(
            value, registry, self,
        ))
    }
}

impl ReflectSerializerProcessor for SceneSerializeProcessor<'_> {
    fn try_serialize<S>(
        &self,
        value: &dyn PartialReflect,
        registry: &TypeRegistry,
        serializer: S,
    ) -> Result<Result<S::Ok, S>, S::Error>
    where
        S: Serializer,
    {
        if let Some(entity) = value
            .try_as_reflect()
            .and_then(|value| value.downcast_ref::<Entity>())
        {
            let local = self.entity_ids.get(entity).ok_or_else(|| {
                S::Error::custom(format!("entity reference {entity} is outside this AZSCENE"))
            })?;
            return Ok(Ok(local.serialize(serializer)?));
        }

        self.capture_asset_dependency(value, registry);
        self.capture_handle(value, registry);

        match value.reflect_ref() {
            ReflectRef::Map(map) => {
                let mut entries = map
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            self.canonical_bytes(key, registry)
                                .map_err(S::Error::custom)?,
                            self.canonical_bytes(value, registry)
                                .map_err(S::Error::custom)?,
                            key,
                            value,
                        ))
                    })
                    .collect::<Result<Vec<_>, S::Error>>()?;
                entries
                    .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
                let mut state = serializer.serialize_map(Some(entries.len()))?;
                for (_, _, key, value) in entries {
                    state.serialize_entry(
                        &TypedReflectSerializer::with_processor(key, registry, self),
                        &TypedReflectSerializer::with_processor(value, registry, self),
                    )?;
                }
                return Ok(Ok(state.end()?));
            }
            ReflectRef::Set(set) => {
                let mut values = set
                    .iter()
                    .map(|value| {
                        Ok((
                            self.canonical_bytes(value, registry)
                                .map_err(S::Error::custom)?,
                            value,
                        ))
                    })
                    .collect::<Result<Vec<_>, S::Error>>()?;
                values.sort_by(|left, right| left.0.cmp(&right.0));
                let mut state = serializer.serialize_seq(Some(values.len()))?;
                for (_, value) in values {
                    state.serialize_element(&TypedReflectSerializer::with_processor(
                        value, registry, self,
                    ))?;
                }
                return Ok(Ok(state.end()?));
            }
            _ => {}
        }

        let serializer = match self.handles.try_serialize(value, registry, serializer) {
            Ok(Ok(value)) => return Ok(Ok(value)),
            Err(error) => return Err(error),
            Ok(Err(serializer)) => serializer,
        };

        structural_serde::try_serialize(value, registry, self, serializer)
    }
}

pub struct SceneDeserializeProcessor<'a> {
    entity_count: u32,
    handles: HandleDeserializeProcessor<'a>,
}

impl<'a> SceneDeserializeProcessor<'a> {
    pub(crate) fn new(entity_count: u32, load_from_path: &'a mut dyn LoadFromPath) -> Self {
        Self {
            entity_count,
            handles: HandleDeserializeProcessor { load_from_path },
        }
    }
}

impl ReflectDeserializerProcessor for SceneDeserializeProcessor<'_> {
    fn try_deserialize<'de, D>(
        &mut self,
        registration: &TypeRegistration,
        registry: &TypeRegistry,
        deserializer: D,
    ) -> Result<Result<Box<dyn PartialReflect>, D>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if registration.type_id() == TypeId::of::<Entity>() {
            let local = u32::deserialize(deserializer)?;
            if local >= self.entity_count {
                return Err(D::Error::custom(format!(
                    "local entity reference {local} is outside entity count {}",
                    self.entity_count
                )));
            }
            let entity = Entity::from_raw_u32(local)
                .ok_or_else(|| D::Error::custom("local entity identity is not representable"))?;
            return Ok(Ok(Box::new(entity)));
        }
        let deserializer = match self
            .handles
            .try_deserialize(registration, registry, deserializer)
        {
            Ok(Ok(value)) => return Ok(Ok(value)),
            Err(error) => return Err(error),
            Ok(Err(deserializer)) => deserializer,
        };
        structural_serde::try_deserialize(registration, registry, self, deserializer)
    }
}

pub struct TrackingLoadFromPath<'a> {
    inner: &'a mut dyn LoadFromPath,
    dependencies: BTreeSet<String>,
}

impl<'a> TrackingLoadFromPath<'a> {
    pub(crate) fn new(inner: &'a mut dyn LoadFromPath) -> Self {
        Self {
            inner,
            dependencies: BTreeSet::new(),
        }
    }

    pub(crate) fn dependencies(&self) -> Vec<String> {
        self.dependencies.iter().cloned().collect()
    }
}

impl LoadFromPath for TrackingLoadFromPath<'_> {
    fn load_from_path_erased(
        &mut self,
        type_id: TypeId,
        path: bevy::asset::AssetPath<'static>,
    ) -> UntypedHandle {
        self.dependencies.insert(path.to_string());
        self.inner.load_from_path_erased(type_id, path)
    }
}
