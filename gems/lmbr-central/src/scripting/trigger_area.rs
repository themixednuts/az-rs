use az_core::component::{Component as AzComponent, ComponentId};
use az_core::crc::Crc32;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;
use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::TriggerAreaComponent` AZ component UUID.
pub const TRIGGER_AREA_COMPONENT_TYPE_ID: Uuid = uuid!("E3DF5790-F0AD-43AE-9FB2-0A37F873DECB");

/// Entity selection mode for trigger area activation.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Scripting/TriggerAreaComponent.h:140`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum TriggerAreaActivationEntityType {
    #[default]
    AllEntities = 0,
    SpecificEntities = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerAreaActivationEntityTypeError {
    value: i32,
}

impl TriggerAreaActivationEntityTypeError {
    #[inline]
    #[must_use]
    pub const fn value(self) -> i32 {
        self.value
    }
}

impl fmt::Display for TriggerAreaActivationEntityTypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown trigger area activation type {}",
            self.value
        )
    }
}

impl std::error::Error for TriggerAreaActivationEntityTypeError {}

impl TryFrom<i32> for TriggerAreaActivationEntityType {
    type Error = TriggerAreaActivationEntityTypeError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::AllEntities),
            1 => Ok(Self::SpecificEntities),
            _ => Err(TriggerAreaActivationEntityTypeError { value }),
        }
    }
}

impl From<TriggerAreaActivationEntityType> for i32 {
    fn from(value: TriggerAreaActivationEntityType) -> Self {
        value as Self
    }
}

/// `LmbrCentral` scripting tag identifier.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Scripting/TagComponentBus.h:22`.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Reflect,
    Serialize,
    Deserialize,
)]
#[reflect(Serialize, Deserialize)]
#[repr(transparent)]
pub struct TriggerAreaTag {
    pub crc32: u32,
}

/// Which native trigger filter already owns a tag that a request attempted to
/// add to the opposite filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerAreaTagFilter {
    Required,
    Excluded,
}

/// The shipping trigger rejects a tag when the opposite filter already
/// contains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("trigger-area tag {tag:?} is already {existing:?}")]
pub struct TriggerAreaTagConflict {
    pub tag: TriggerAreaTag,
    pub existing: TriggerAreaTagFilter,
}

/// Mutable filter surface corresponding to the four tag operations on
/// `TriggerAreaRequestsBus`.
///
/// The current implementation permits duplicate entries in the same list,
/// rejects a tag present in the opposite list, and removes the first matching
/// entry. Those details are visible in the request vtable implementations at
/// as reflected by the matching native request handlers.
pub trait TriggerAreaFilters {
    /// Adds one required-tag entry.
    ///
    /// # Errors
    ///
    /// Returns [`TriggerAreaTagConflict`] when `tag` is excluded.
    fn try_add_required_tag(
        &mut self,
        tag: impl Into<TriggerAreaTag>,
    ) -> Result<(), TriggerAreaTagConflict>;

    /// Removes the first required-tag entry matching `tag`.
    fn remove_required_tag(&mut self, tag: impl Borrow<TriggerAreaTag>) -> bool;

    /// Adds one excluded-tag entry.
    ///
    /// # Errors
    ///
    /// Returns [`TriggerAreaTagConflict`] when `tag` is required.
    fn try_add_excluded_tag(
        &mut self,
        tag: impl Into<TriggerAreaTag>,
    ) -> Result<(), TriggerAreaTagConflict>;

    /// Removes the first excluded-tag entry matching `tag`.
    fn remove_excluded_tag(&mut self, tag: impl Borrow<TriggerAreaTag>) -> bool;
}

/// One request directed at a trigger-area entity.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerAreaRequest {
    pub area: Entity,
    pub kind: TriggerAreaRequestKind,
}

/// Typed replacement for the reflected `TriggerAreaRequestsBus`
/// events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerAreaRequestKind {
    AddRequiredTag(TriggerAreaTag),
    RemoveRequiredTag(TriggerAreaTag),
    AddExcludedTag(TriggerAreaTag),
    RemoveExcludedTag(TriggerAreaTag),
    AddProximityTrigger,
    RemoveProximityTrigger,
}

/// Relevance callback used by the world/server orchestration layer to gate a
/// trigger whose authored relevance radius is positive.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerAreaRelevanceChanged {
    pub area: Entity,
    pub is_relevant: bool,
}

/// Global respawn notification. Shipping trigger areas with
/// `ResetTriggerOnRespawn` cycle an existing proximity trigger when this fires.
#[derive(Message, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TriggerAreasRespawned;

impl TriggerAreaTag {
    #[must_use]
    pub const fn from_raw_crc32(crc32: u32) -> Self {
        Self { crc32 }
    }

    #[must_use]
    pub const fn from_crc32(crc32: Crc32) -> Self {
        Self::from_raw_crc32(crc32.value())
    }

    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        Self::from_crc32(Crc32::from_str_lower(name))
    }

    #[must_use]
    pub const fn crc32(self) -> Crc32 {
        Crc32::from_u32(self.crc32)
    }
}

impl From<Crc32> for TriggerAreaTag {
    fn from(value: Crc32) -> Self {
        Self::from_crc32(value)
    }
}

impl From<TriggerAreaTag> for Crc32 {
    fn from(value: TriggerAreaTag) -> Self {
        value.crc32()
    }
}

/// Runtime trigger volume settings and filters.
#[derive(AzRtti, Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[az_rtti(
    name = "LmbrCentral::TriggerAreaComponent",
    TRIGGER_AREA_COMPONENT_TYPE_ID,
    az_core::component::Component,
    az_framework::NetBindable,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(
    tag = "azoth.lmbr_central.TriggerAreaComponent",
    version = 2,
    migration(from = 1, to = 2, migrate = TriggerAreaComponent::migrate_v1_to_v2)
)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the serde(rename) names ARE the ObjectStream wire field names of \
              LmbrCentral::TriggerAreaComponent - TriggerOnce, ResetTriggerOnRespawn, \
              StaticPosition, OnlyActiveForLocalPlayer; the type also carries a \
              versioned prefab tag with a v1->v2 migration, so the field set is a \
              committed on-disk shape"
)]
pub struct TriggerAreaComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "BaseClass2", default)]
    #[reflect(ignore)]
    pub net_bindable: az_framework::NetBindable,
    #[serde(rename = "TriggerOnce", default)]
    pub trigger_once: bool,
    #[serde(rename = "ActivatedBy", default)]
    pub activation_entity_type: TriggerAreaActivationEntityType,
    #[serde(rename = "SpecificInteractEntities", default)]
    pub specific_interact_entities: Vec<u64>,
    #[serde(rename = "ResetTriggerOnRespawn", default)]
    pub reset_trigger_on_respawn: bool,
    #[serde(rename = "StaticPosition", default)]
    pub static_position: bool,
    #[serde(rename = "OnlyActiveForLocalPlayer", default)]
    pub only_active_for_local_player: bool,
    #[serde(rename = "RelevanceRadius", default)]
    pub relevance_radius: f32,
    #[serde(rename = "RequiredTags", default)]
    pub required_tags: Vec<TriggerAreaTag>,
    #[serde(rename = "ExcludedTags", default)]
    pub excluded_tags: Vec<TriggerAreaTag>,
}

impl Default for TriggerAreaComponent {
    fn default() -> Self {
        Self {
            az_component: AzComponent {
                id: ComponentId::INVALID,
            },
            net_bindable: az_framework::NetBindable::default(),
            trigger_once: false,
            activation_entity_type: TriggerAreaActivationEntityType::AllEntities,
            specific_interact_entities: Vec::new(),
            reset_trigger_on_respawn: false,
            static_position: false,
            // The native constructor enables this flag.
            only_active_for_local_player: true,
            relevance_radius: 0.0,
            required_tags: Vec::new(),
            excluded_tags: Vec::new(),
        }
    }
}

impl TriggerAreaComponent {
    /// v2 only renumbered source identity; field layout is unchanged.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "the `#[prefab(migration(...))]` attribute above requires this exact \
                  fallible signature; returning the bare value would not compile"
    )]
    const fn migrate_v1_to_v2(
        value: az_prefab::ErasedPrefabValue,
    ) -> Result<az_prefab::ErasedPrefabValue, az_prefab::PrefabBuildError> {
        Ok(value)
    }
}

impl TriggerAreaFilters for TriggerAreaComponent {
    fn try_add_required_tag(
        &mut self,
        tag: impl Into<TriggerAreaTag>,
    ) -> Result<(), TriggerAreaTagConflict> {
        let tag = tag.into();
        if self.excluded_tags.contains(&tag) {
            return Err(TriggerAreaTagConflict {
                tag,
                existing: TriggerAreaTagFilter::Excluded,
            });
        }
        self.required_tags.push(tag);
        Ok(())
    }

    fn remove_required_tag(&mut self, tag: impl Borrow<TriggerAreaTag>) -> bool {
        let Some(index) = self
            .required_tags
            .iter()
            .position(|candidate| candidate == tag.borrow())
        else {
            return false;
        };
        self.required_tags.remove(index);
        true
    }

    fn try_add_excluded_tag(
        &mut self,
        tag: impl Into<TriggerAreaTag>,
    ) -> Result<(), TriggerAreaTagConflict> {
        let tag = tag.into();
        if self.required_tags.contains(&tag) {
            return Err(TriggerAreaTagConflict {
                tag,
                existing: TriggerAreaTagFilter::Required,
            });
        }
        self.excluded_tags.push(tag);
        Ok(())
    }

    fn remove_excluded_tag(&mut self, tag: impl Borrow<TriggerAreaTag>) -> bool {
        let Some(index) = self
            .excluded_tags
            .iter()
            .position(|candidate| candidate == tag.borrow())
        else {
            return false;
        };
        self.excluded_tags.remove(index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_area_activation_entity_type_maps_native_values() {
        assert_eq!(
            TriggerAreaActivationEntityType::try_from(0),
            Ok(TriggerAreaActivationEntityType::AllEntities)
        );
        assert_eq!(
            TriggerAreaActivationEntityType::try_from(1),
            Ok(TriggerAreaActivationEntityType::SpecificEntities)
        );
        assert_eq!(
            TriggerAreaActivationEntityType::try_from(2)
                .unwrap_err()
                .value(),
            2
        );
        assert_eq!(
            i32::from(TriggerAreaActivationEntityType::SpecificEntities),
            1
        );
    }

    #[test]
    fn trigger_area_tag_wraps_lmbr_central_crc32_tag() {
        let tag = TriggerAreaTag::from_name("Player");

        assert_eq!(tag.crc32(), Crc32::from_str_lower("player"));
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
    )]
    fn trigger_area_defaults_match_shipping_constructor() {
        let area = TriggerAreaComponent::default();

        assert!(area.only_active_for_local_player);
        assert!(!area.trigger_once);
        assert!(!area.reset_trigger_on_respawn);
        assert!(!area.static_position);
        assert_eq!(area.relevance_radius, 0.0);
    }

    #[test]
    fn trigger_area_filter_requests_preserve_native_list_semantics() {
        let required = TriggerAreaTag::from_name("Player");
        let excluded = TriggerAreaTag::from_name("Dead");
        let mut area = TriggerAreaComponent::default();

        area.try_add_required_tag(required).unwrap();
        area.try_add_required_tag(required).unwrap();
        area.try_add_excluded_tag(excluded).unwrap();
        assert_eq!(area.required_tags, vec![required, required]);
        assert_eq!(area.excluded_tags, vec![excluded]);
        assert_eq!(
            area.try_add_excluded_tag(required),
            Err(TriggerAreaTagConflict {
                tag: required,
                existing: TriggerAreaTagFilter::Required,
            })
        );
        assert!(area.remove_required_tag(required));
        assert_eq!(area.required_tags, vec![required]);
    }
}
