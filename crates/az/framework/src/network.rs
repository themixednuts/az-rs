use az_derive::AzRtti;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// `AzFramework::NetBindable` RTTI UUID.
pub const NET_BINDABLE_TYPE_ID: Uuid = uuid!("80206665-D429-4703-B42E-94434F82F381");

/// Serialized network-binding base for AZ components.
#[derive(AzRtti, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "NetBindable", NET_BINDABLE_TYPE_ID, register)]
pub struct NetBindable {
    #[serde(rename = "m_isNetSyncEnabled", default)]
    pub is_net_sync_enabled: bool,
}

impl NetBindable {
    #[inline]
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        self.is_net_sync_enabled
    }
}
