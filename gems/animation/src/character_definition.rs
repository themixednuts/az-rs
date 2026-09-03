use az_animation::character::definition::{
    CHARACTER_DEFINITION_PRODUCT_EXTENSION, CharacterDefinitionAsset,
    CharacterDefinitionCodecError, read_character_definition,
};
use bevy::{
    asset::{AssetLoader, AsyncReadExt, LoadContext, io::Reader},
    reflect::TypePath,
};

#[derive(Default, TypePath)]
pub struct CharacterDefinitionAssetLoader;

#[derive(Debug, thiserror::Error)]
pub enum CharacterDefinitionLoadError {
    #[error("read character-definition product: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse character-definition product: {0}")]
    Parse(#[from] CharacterDefinitionCodecError),
}

impl AssetLoader for CharacterDefinitionAssetLoader {
    type Asset = CharacterDefinitionAsset;
    type Settings = ();
    type Error = CharacterDefinitionLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(read_character_definition(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[CHARACTER_DEFINITION_PRODUCT_EXTENSION]
    }
}
