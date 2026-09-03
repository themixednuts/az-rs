//! Reflected `LmbrCentral` Mannequin component data.

use az_core::component::{Component as AzComponent, EntityId};
use az_derive::{AzComponent, AzRtti};
use az_framework::SimpleAssetReferenceBase;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const MANNEQUIN_ANIMATION_DATABASE_REFERENCE_TYPE_ID: Uuid =
    uuid!("171F78A0-CAEB-4665-8439-F954EDCF29AE");
pub const MANNEQUIN_CONTROLLER_DEFINITION_REFERENCE_TYPE_ID: Uuid =
    uuid!("17A70E99-A282-4FFE-9921-3EC4884A2F70");
pub const MANNEQUIN_COMPONENT_TYPE_ID: Uuid = uuid!("83E4AC4C-2184-49D1-AAD0-A0687EEE1405");
pub const MANNEQUIN_SCOPE_COMPONENT_TYPE_ID: Uuid = uuid!("AB4FDB4A-D742-4EF8-B36E-9A1775FA6FA5");

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
    name = "AzFramework::SimpleAssetReference<LmbrCentral::MannequinAnimationDatabaseAsset>",
    MANNEQUIN_ANIMATION_DATABASE_REFERENCE_TYPE_ID,
    az_framework::SimpleAssetReferenceBase,
    register
)]
#[reflect(Default)]
pub struct SimpleAssetReferenceMannequinAnimationDatabaseAsset {
    #[serde(rename = "BaseClass1", default)]
    pub simple_asset_reference_base: SimpleAssetReferenceBase,
}

impl SimpleAssetReferenceMannequinAnimationDatabaseAsset {
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            simple_asset_reference_base: SimpleAssetReferenceBase::new(path),
        }
    }

    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.simple_asset_reference_base.path()
    }
}

impl AsRef<SimpleAssetReferenceBase> for SimpleAssetReferenceMannequinAnimationDatabaseAsset {
    fn as_ref(&self) -> &SimpleAssetReferenceBase {
        &self.simple_asset_reference_base
    }
}

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
    name = "AzFramework::SimpleAssetReference<LmbrCentral::MannequinControllerDefinitionAsset>",
    MANNEQUIN_CONTROLLER_DEFINITION_REFERENCE_TYPE_ID,
    az_framework::SimpleAssetReferenceBase,
    register
)]
#[reflect(Default)]
pub struct SimpleAssetReferenceMannequinControllerDefinitionAsset {
    #[serde(rename = "BaseClass1", default)]
    pub simple_asset_reference_base: SimpleAssetReferenceBase,
}

impl SimpleAssetReferenceMannequinControllerDefinitionAsset {
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            simple_asset_reference_base: SimpleAssetReferenceBase::new(path),
        }
    }

    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.simple_asset_reference_base.path()
    }
}

impl AsRef<SimpleAssetReferenceBase> for SimpleAssetReferenceMannequinControllerDefinitionAsset {
    fn as_ref(&self) -> &SimpleAssetReferenceBase {
        &self.simple_asset_reference_base
    }
}

/// Creates and owns an action controller for the referenced definition.
///
/// The current component queues `initial_fragment` at priority one as a
/// persistent request after controller activation.
#[derive(
    Component,
    AzComponent,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_component(name = "MannequinComponent", MANNEQUIN_COMPONENT_TYPE_ID)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.MannequinComponent", version = 1)]
pub struct MannequinComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Controller Definition", default)]
    pub controller_definition: SimpleAssetReferenceMannequinControllerDefinitionAsset,
    #[serde(rename = "Initial Fragment", default)]
    pub initial_fragment: String,
}

impl MannequinComponent {
    #[must_use]
    pub fn initial_fragment(&self) -> Option<&str> {
        let fragment = self.initial_fragment.trim();
        (!fragment.is_empty()).then_some(fragment)
    }
}

/// Binds an animation database and entity to a named action-controller scope.
#[derive(
    Component,
    AzComponent,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_component(name = "MannequinScopeComponent", MANNEQUIN_SCOPE_COMPONENT_TYPE_ID)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.MannequinScopeComponent", version = 1)]
pub struct MannequinScopeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Animation Database", default)]
    pub animation_database: SimpleAssetReferenceMannequinAnimationDatabaseAsset,
    #[serde(rename = "Context Name", default)]
    pub context_name: String,
    #[serde(rename = "Target Entity", default)]
    pub target_entity: EntityId,
}

impl MannequinScopeComponent {
    #[must_use]
    pub fn context_name(&self) -> Option<&str> {
        let context = self.context_name.trim();
        (!context.is_empty()).then_some(context)
    }
}
