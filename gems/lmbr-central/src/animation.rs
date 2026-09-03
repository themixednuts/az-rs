//! Animation components for `LmbrCentral`.
//!
//! Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Animation`.

mod attachment;
mod events;
mod mannequin;
mod motion;
mod simple;

use bevy::prelude::*;

pub use attachment::{
    ATTACHMENT_COMPONENT_TYPE_ID, ATTACHMENT_CONFIGURATION_TYPE_ID, AttachmentComponent,
    AttachmentComponentAction, AttachmentComponentNotification, AttachmentComponentRequest,
    AttachmentConfiguration, AttachmentRuntime, AttachmentScaleSource, AttachmentTarget,
    compose_attachment_transform,
};
pub use events::{
    ANIMATION_EVENT_TYPE_ID, AnimationEvent, CharacterAnimationEvent,
    CharacterAnimationEventDatabase, CharacterAnimationEventSet, CharacterAnimationEventsPlugin,
};
pub use mannequin::{
    MANNEQUIN_ANIMATION_DATABASE_REFERENCE_TYPE_ID, MANNEQUIN_COMPONENT_TYPE_ID,
    MANNEQUIN_CONTROLLER_DEFINITION_REFERENCE_TYPE_ID, MANNEQUIN_SCOPE_COMPONENT_TYPE_ID,
    MannequinComponent, MannequinScopeComponent,
    SimpleAssetReferenceMannequinAnimationDatabaseAsset,
    SimpleAssetReferenceMannequinControllerDefinitionAsset,
};
pub use motion::{
    CHARACTER_ANIMATION_MANAGER_COMPONENT_TYPE_ID, CharacterAnimationManagerComponent,
    MOTION_PARAMETER_SMOOTHING_COMPONENT_TYPE_ID, MOTION_PARAMETER_SMOOTHING_SETTINGS_TYPE_ID,
    MotionParameterSmoothingComponent, MotionParameterSmoothingSettings,
};
pub use simple::{
    ANIMATED_LAYER_TYPE_ID, AnimatedLayer, AnimatedLayerResolveError,
    SIMPLE_ANIMATION_COMPONENT_TYPE_ID, SimpleAnimationComponent,
};

pub fn register_animation_components(app: &mut App) {
    attachment::register_attachment_runtime(app);
    motion::register_motion_runtime(app);
    app.register_type::<AttachmentScaleSource>()
        .register_type::<AttachmentConfiguration>()
        .register_type_data::<AttachmentConfiguration, az_core::ReflectAzTypeInfo>()
        .register_type::<AttachmentComponent>()
        .register_type_data::<AttachmentComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<AttachmentComponent, az_core::ReflectAzRtti>()
        .register_type::<MotionParameterSmoothingSettings>()
        .register_type_data::<MotionParameterSmoothingSettings, az_core::ReflectAzTypeInfo>()
        .register_type::<MotionParameterSmoothingComponent>()
        .register_type_data::<MotionParameterSmoothingComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<MotionParameterSmoothingComponent, az_core::ReflectAzRtti>()
        .register_type::<CharacterAnimationManagerComponent>()
        .register_type_data::<CharacterAnimationManagerComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CharacterAnimationManagerComponent, az_core::ReflectAzRtti>()
        .register_type::<az_framework::SimpleAssetReferenceBase>()
        .register_type_data::<az_framework::SimpleAssetReferenceBase, az_core::ReflectAzTypeInfo>()
        .register_type_data::<az_framework::SimpleAssetReferenceBase, az_core::ReflectAzRtti>()
        .register_type::<SimpleAssetReferenceMannequinAnimationDatabaseAsset>()
        .register_type_data::<SimpleAssetReferenceMannequinAnimationDatabaseAsset, az_core::ReflectAzTypeInfo>()
        .register_type_data::<SimpleAssetReferenceMannequinAnimationDatabaseAsset, az_core::ReflectAzRtti>()
        .register_type::<SimpleAssetReferenceMannequinControllerDefinitionAsset>()
        .register_type_data::<SimpleAssetReferenceMannequinControllerDefinitionAsset, az_core::ReflectAzTypeInfo>()
        .register_type_data::<SimpleAssetReferenceMannequinControllerDefinitionAsset, az_core::ReflectAzRtti>()
        .register_type::<MannequinComponent>()
        .register_type_data::<MannequinComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<MannequinComponent, az_core::ReflectAzRtti>()
        .register_type::<MannequinScopeComponent>()
        .register_type_data::<MannequinScopeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<MannequinScopeComponent, az_core::ReflectAzRtti>()
        .register_type::<AnimatedLayer>()
        .register_type_data::<AnimatedLayer, az_core::ReflectAzTypeInfo>()
        .register_type::<SimpleAnimationComponent>()
        .register_type_data::<SimpleAnimationComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<SimpleAnimationComponent, az_core::ReflectAzRtti>();
}
