//! Legacy `AzFramework::SimpleAssetReference` serialized base data.

use az_derive::AzRtti;
use bevy::prelude::Reflect;
use bevy::reflect::std_traits::ReflectDefault;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID: Uuid = uuid!("E16CA6C5-5C78-4AD9-8E9B-F8C1FB4D1DB8");

#[derive(
    AzRtti,
    Debug,
    Clone,
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
#[az_rtti(
    name = "SimpleAssetReferenceBase",
    SIMPLE_ASSET_REFERENCE_BASE_TYPE_ID,
    register
)]
#[reflect(Default)]
pub struct SimpleAssetReferenceBase {
    #[serde(rename = "AssetPath", default)]
    pub asset_path: String,
}

impl SimpleAssetReferenceBase {
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            asset_path: path.into(),
        }
    }

    #[must_use]
    pub fn path(&self) -> Option<&str> {
        let path = self.asset_path.trim();
        (!path.is_empty()).then_some(path)
    }
}

impl AsRef<str> for SimpleAssetReferenceBase {
    fn as_ref(&self) -> &str {
        &self.asset_path
    }
}

impl From<String> for SimpleAssetReferenceBase {
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

impl From<&str> for SimpleAssetReferenceBase {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}
