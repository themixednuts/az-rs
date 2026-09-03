use bevy::prelude::*;

use crate::{
    AudioControlId, AudioObstructionType, AudioRequest, RecordedAudioRequests,
    WwiseAudioControlsAsset, WwiseAudioControlsAssetLoader, WwiseBankHeader, WwiseBankId,
    WwiseBankSection, WwiseControlRegistry, WwiseHierarchyObject, WwiseHierarchyObjectKind,
    WwiseMediaAsset, WwiseMediaAssetLoader, WwiseMediaChunk, WwiseMediaChunkId, WwiseMediaEntry,
    WwiseMediaId, WwiseMediaInfo, WwiseNameId, WwiseObjectId, WwiseSectionId, WwiseSoundBank,
    WwiseSoundBankAsset, WwiseSoundBankAssetLoader, rebuild_wwise_control_registry_on_asset_events,
};

use super::request::record_audio_requests;

/// Minimal `AudioSystem` Gem plugin.
#[derive(Debug, Default)]
pub struct AudioSystemPlugin;

impl Plugin for AudioSystemPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<AudioControlId>()
            .register_type::<AudioObstructionType>()
            .register_type_data::<AudioObstructionType, az_core::ReflectAzTypeInfo>()
            .register_type::<WwiseBankId>()
            .register_type::<WwiseMediaId>()
            .register_type::<WwiseNameId>()
            .register_type::<WwiseObjectId>()
            .register_type::<WwiseSectionId>()
            .register_type::<WwiseMediaChunkId>()
            .register_type::<WwiseBankSection>()
            .register_type::<WwiseBankHeader>()
            .register_type::<WwiseMediaEntry>()
            .register_type::<WwiseMediaChunk>()
            .register_type::<WwiseMediaInfo>()
            .register_type::<WwiseHierarchyObjectKind>()
            .register_type::<WwiseHierarchyObject>()
            .register_type::<WwiseSoundBank>()
            .add_message::<AudioRequest>()
            .init_resource::<RecordedAudioRequests>()
            .init_resource::<WwiseControlRegistry>()
            .add_systems(Update, record_audio_requests);
    }
}

/// Register `AudioSystem` asset loaders.
#[derive(Debug, Default)]
pub struct AudioSystemAssetPlugin;

impl Plugin for AudioSystemAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<WwiseSoundBankAsset>()
            .init_asset_loader::<WwiseSoundBankAssetLoader>()
            .init_asset::<WwiseMediaAsset>()
            .init_asset_loader::<WwiseMediaAssetLoader>()
            .init_asset::<WwiseAudioControlsAsset>()
            .init_asset_loader::<WwiseAudioControlsAssetLoader>()
            .init_resource::<WwiseControlRegistry>()
            .add_systems(Update, rebuild_wwise_control_registry_on_asset_events);
    }
}
