//! Untyped asset reference value used by reflected AZ assets.

use az_core::AssetType;

use crate::{AssetId, ReflectAssetDependency};
use bevy_app::{App, Plugin};
use bevy_reflect::{Reflect, ReflectDeserialize, ReflectSerialize};

/// Reflected untyped asset reference with optional source hint.
#[derive(Debug, Clone, PartialEq, Eq, Default, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(opaque, Serialize, Deserialize, AssetDependency)]
pub struct UntypedAssetRef {
    pub asset_id: AssetId,
    pub asset_type: AssetType,
    pub hint: Option<String>,
}

impl UntypedAssetRef {
    #[inline]
    #[must_use]
    pub fn new(asset_id: AssetId, asset_type: AssetType, hint: Option<impl Into<String>>) -> Self {
        Self {
            asset_id,
            asset_type,
            hint: hint.map(Into::into),
        }
    }

    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            asset_id: AssetId::nil(),
            asset_type: AssetType::nil(),
            hint: None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_id(asset_id: AssetId, asset_type: AssetType) -> Self {
        Self {
            asset_id,
            asset_type,
            hint: None,
        }
    }

    #[inline]
    #[must_use]
    pub fn from_hint(hint: impl Into<String>) -> Self {
        Self {
            hint: Some(hint.into()),
            ..Self::empty()
        }
    }

    #[inline]
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    #[inline]
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint
            .as_deref()
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
    }

    #[inline]
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        self.asset_id.is_nil()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.is_nil() && self.asset_type == AssetType::nil() && self.hint().is_none()
    }
}

pub struct UntypedAssetRefPlugin;

impl Plugin for UntypedAssetRefPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<UntypedAssetRef>()
            .register_type::<Vec<UntypedAssetRef>>();
    }

    fn is_unique(&self) -> bool {
        false
    }
}
