//! Generic `SlayerScript` source ownership and compiler routing.
//!
//! The types in this module are the source contracts reflected directly by the
//! native `SlayerScript` system component. Project source subclasses may compose
//! them, but still compile to project-owned typed operations before runtime.
//! `ObjectStream` decoding and subtype selection remain offline concerns.

use az_core::crc::Crc32;
use az_derive::{AzRtti, AzTypeInfo};
use bevy::prelude::Reflect;
use gridmate::Marshaler;
use serde::{Deserialize, Serialize};

/// Generic `SlayerScript` source base marker (native source version 1).
///
/// Its empty reflected projection does not imply runtime no-op behavior.
#[derive(
    AzRtti, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect,
)]
#[az_rtti("3CAD57DB-9179-456F-904F-1B3D68FAD90E")]
pub struct SlayerScriptData {}

impl SlayerScriptData {
    /// Native `SerializeContext` version.
    pub const VERSION: u32 = 1;
}

impl SlayerScriptSource for SlayerScriptData {
    fn source_type_id(&self) -> uuid::Uuid {
        <Self as az_core::AzTypeInfo>::TYPE_ID
    }
}

/// Marker for a typed offline source payload accepted by the generic container.
///
/// Projects implement this on an explicit closed source enum, then lower that
/// enum into their typed program product. The marker is not a dynamic registry,
/// reflection executor, or runtime property bag.
pub trait SlayerScriptSource: std::fmt::Debug + Send + Sync {
    /// Returns the payload's native source UUID for offline compiler routing.
    #[must_use]
    fn source_type_id(&self) -> uuid::Uuid;
}

/// A named, owned `SlayerScript` source payload (native source version 1).
#[derive(AzRtti, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[reflect(where)]
#[az_rtti("C9CCB4FB-44B8-4BE8-BCA8-4909C1C22B82")]
pub struct SlayerScriptDataContainer<S = SlayerScriptData>
where
    S: SlayerScriptSource,
{
    /// Authored script name (native serialized field `m_scriptName`).
    #[serde(rename = "m_scriptName", default)]
    pub script_name: String,
    /// Owned concrete source payload (native serialized field `m_scriptData`).
    #[serde(rename = "m_scriptData", default)]
    pub script_data: Option<S>,
}

impl<S> SlayerScriptDataContainer<S>
where
    S: SlayerScriptSource,
{
    /// Native `SerializeContext` version.
    pub const VERSION: u32 = 1;
    /// Native serialized script-name field.
    pub const SCRIPT_NAME_FIELD: &'static str = "m_scriptName";
    /// Native serialized owned-payload field.
    pub const SCRIPT_DATA_FIELD: &'static str = "m_scriptData";

    /// Creates a named container around one concrete owned source payload.
    #[must_use]
    pub fn new(script_name: impl Into<String>, script_data: S) -> Self {
        Self {
            script_name: script_name.into(),
            script_data: Some(script_data),
        }
    }
}

impl<S> Default for SlayerScriptDataContainer<S>
where
    S: SlayerScriptSource,
{
    fn default() -> Self {
        Self {
            script_name: String::new(),
            script_data: None,
        }
    }
}

/// Editable entity selector used by generic `SlayerScript` source events.
#[derive(
    AzRtti, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Reflect,
)]
#[az_rtti(name = "SlayerScriptEditCrc", "1611905C-C51F-4FF2-ADC1-A553763826BA")]
pub struct SlayerScriptEditCrc {
    /// Native `m_string` authoring field.
    #[serde(rename = "m_string", default)]
    pub string: String,
    /// Native `m_crc` cooked selector field.
    #[serde(rename = "m_crc", default)]
    pub crc: u32,
    /// Native editor/debug row index.
    #[serde(rename = "m_debugIndex", default = "default_debug_index")]
    pub debug_index: u32,
}

impl SlayerScriptEditCrc {
    /// Native `SerializeContext` version.
    pub const VERSION: u32 = 2;

    #[must_use]
    pub fn new(value: impl Into<String>, crc: Crc32) -> Self {
        Self {
            string: value.into(),
            crc: crc.value(),
            debug_index: default_debug_index(),
        }
    }

    /// Returns the non-empty editable selector text.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        let value = self.string.trim();
        (!value.is_empty()).then_some(value)
    }

    /// Returns the cooked selector CRC without interpreting its domain.
    #[must_use]
    pub const fn crc32(&self) -> Crc32 {
        Crc32::from_u32(self.crc)
    }
}

impl Default for SlayerScriptEditCrc {
    fn default() -> Self {
        Self {
            string: String::new(),
            crc: 0,
            debug_index: default_debug_index(),
        }
    }
}

/// Closed entity-event selector reflected by the generic `SlayerScript` system.
#[derive(
    AzTypeInfo,
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Marshaler,
    Serialize,
    Deserialize,
    Reflect,
)]
#[repr(i32)]
#[az_type_info(name = "EntityEvents", "FA36FE64-38FC-4A2D-B59B-D72571C5EF29")]
#[serde(try_from = "i32", into = "i32")]
pub enum EntityEvents {
    #[default]
    Activate = 0,
    Deactivate = 1,
    Trigger = 2,
    Untrigger = 3,
}

impl From<EntityEvents> for i32 {
    fn from(value: EntityEvents) -> Self {
        value as Self
    }
}

impl TryFrom<i32> for EntityEvents {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Activate),
            1 => Ok(Self::Deactivate),
            2 => Ok(Self::Trigger),
            3 => Ok(Self::Untrigger),
            value => Err(value),
        }
    }
}

/// Generic entity activation/trigger source event.
#[derive(AzRtti, Debug, Default, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[az_rtti(name = "EntityEvent", "82897793-BE85-48CD-BAC9-A1AD3B487133")]
pub struct EntityEvent {
    #[serde(rename = "eventType", default)]
    pub event_type: EntityEvents,
    #[serde(rename = "applyOnChildren", default)]
    pub apply_on_children: bool,
    #[serde(rename = "entityNames", default)]
    pub entity_names: Vec<SlayerScriptEditCrc>,
}

impl EntityEvent {
    /// Native `SerializeContext` version.
    pub const VERSION: u32 = 2;
}

/// Generic mesh-opacity source event.
#[derive(AzRtti, Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[az_rtti(name = "OpacityEvent", "045EA278-0F20-4625-8C4D-97E3B2483439")]
pub struct OpacityEvent {
    #[serde(rename = "startingOpacity", default = "default_one")]
    pub starting_opacity: f32,
    #[serde(rename = "targetOpacity", default)]
    pub target_opacity: f32,
    #[serde(rename = "opacityTransitionDuration", default = "default_one")]
    pub opacity_transition_duration: f32,
    #[serde(rename = "applyOnChildren", default)]
    pub apply_on_children: bool,
    #[serde(rename = "entityNames", default)]
    pub entity_names: Vec<SlayerScriptEditCrc>,
}

impl OpacityEvent {
    /// Native `SerializeContext` version.
    pub const VERSION: u32 = 2;
}

impl Default for OpacityEvent {
    fn default() -> Self {
        Self {
            starting_opacity: 1.0,
            target_opacity: 0.0,
            opacity_transition_duration: 1.0,
            apply_on_children: false,
            entity_names: Vec::new(),
        }
    }
}

/// Generic entity-rotation source event.
#[derive(AzRtti, Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
#[az_rtti(name = "RotationEvent", "83232264-3642-440C-9147-C6CE1C77706F")]
pub struct RotationEvent {
    #[serde(rename = "targetRotationPercent", default)]
    pub target_rotation_percent: f32,
    #[serde(rename = "rotationDuration", default)]
    pub rotation_duration: f32,
    #[serde(rename = "initializeRotationToZeroPercent", default = "default_true")]
    pub initialize_rotation_to_zero_percent: bool,
    #[serde(rename = "entityNames", default)]
    pub entity_names: Vec<SlayerScriptEditCrc>,
}

impl RotationEvent {
    /// Native `SerializeContext` version.
    pub const VERSION: u32 = 2;
}

impl Default for RotationEvent {
    fn default() -> Self {
        Self {
            target_rotation_percent: 0.0,
            rotation_duration: 0.0,
            initialize_rotation_to_zero_percent: true,
            entity_names: Vec::new(),
        }
    }
}

const fn default_debug_index() -> u32 {
    1000
}

const fn default_one() -> f32 {
    1.0
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use az_core::AzTypeInfo;
    use uuid::uuid;

    use super::*;

    #[test]
    fn shared_source_types_keep_native_ids() {
        assert_eq!(
            SlayerScriptEditCrc::TYPE_ID,
            uuid!("1611905C-C51F-4FF2-ADC1-A553763826BA")
        );
        assert_eq!(
            EntityEvents::TYPE_ID,
            uuid!("FA36FE64-38FC-4A2D-B59B-D72571C5EF29")
        );
        assert_eq!(
            EntityEvent::TYPE_ID,
            uuid!("82897793-BE85-48CD-BAC9-A1AD3B487133")
        );
        assert_eq!(
            OpacityEvent::TYPE_ID,
            uuid!("045EA278-0F20-4625-8C4D-97E3B2483439")
        );
        assert_eq!(
            RotationEvent::TYPE_ID,
            uuid!("83232264-3642-440C-9147-C6CE1C77706F")
        );
    }

    // Each default is the exact literal the native constructor stores; an
    // epsilon comparison would stop pinning that.
    #[allow(clippy::float_cmp)]
    #[test]
    fn shared_source_defaults_match_native_construction() {
        assert_eq!(SlayerScriptEditCrc::default().debug_index, 1000);
        assert_eq!(EntityEvent::default().event_type, EntityEvents::Activate);

        let opacity = OpacityEvent::default();
        assert_eq!(opacity.starting_opacity, 1.0);
        assert_eq!(opacity.target_opacity, 0.0);
        assert_eq!(opacity.opacity_transition_duration, 1.0);

        let rotation = RotationEvent::default();
        assert_eq!(rotation.target_rotation_percent, 0.0);
        assert_eq!(rotation.rotation_duration, 0.0);
        assert!(rotation.initialize_rotation_to_zero_percent);
    }

    #[test]
    fn entity_event_discriminants_are_closed() {
        assert_eq!(i32::from(EntityEvents::Untrigger), 3);
        assert_eq!(EntityEvents::try_from(0), Ok(EntityEvents::Activate));
        assert_eq!(EntityEvents::try_from(4), Err(4));
    }
}
