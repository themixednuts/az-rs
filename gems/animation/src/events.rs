use az_animation::events::{
    AnimationEventDatabaseAsset, AnimationEventDatabaseCodecError, read_animation_event_database,
};
use bevy::{
    asset::{AssetLoader, AsyncReadExt, LoadContext, io::Reader},
    reflect::TypePath,
};

pub const ANIMATION_EVENT_DATABASE_PRODUCT_KEY: &str = "animation.events.bin";

#[derive(Default, TypePath)]
pub struct AnimationEventDatabaseAssetLoader;

#[derive(Debug, thiserror::Error)]
pub enum AnimationEventDatabaseLoadError {
    #[error("read animation-event database: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse animation-event database: {0}")]
    Parse(#[from] AnimationEventDatabaseCodecError),
}

impl AssetLoader for AnimationEventDatabaseAssetLoader {
    type Asset = AnimationEventDatabaseAsset;
    type Settings = ();
    type Error = AnimationEventDatabaseLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(read_animation_event_database(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[ANIMATION_EVENT_DATABASE_PRODUCT_KEY]
    }
}
