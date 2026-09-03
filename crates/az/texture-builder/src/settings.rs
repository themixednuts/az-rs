use az_filesystem::normalize_source_path;
pub use az_texture_source::{
    TextureAuthoringFormat, TextureColorSpace, TextureCompressionFormat, TextureCompressionIntent,
    TextureDimension, TextureImageOrder, TextureMipSettings, TextureNormalConvention,
    TextureNormalSemantics, TextureOrmSemantics, TextureRole, TextureSourceSettings,
    TextureSourceShape,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TextureSettingsError {
    #[error("texture settings are not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("parse texture settings RON: {0}")]
    Ron(String),
}

#[derive(Debug, Error)]
pub enum TextureSettingsWriteError {
    #[error("serialize texture settings RON: {0}")]
    Ron(String),
}

#[must_use]
pub fn texture_settings_source_path(authoring_source_path: &str) -> String {
    let normalized = normalize_source_path(authoring_source_path);
    normalized
        .strip_suffix(".png")
        .or_else(|| normalized.strip_suffix(".exr"))
        .map_or_else(
            || format!("{normalized}.texture.ron"),
            |stem| format!("{stem}.texture.ron"),
        )
}

/// # Errors
///
/// Returns [`TextureSettingsWriteError::Ron`] if the settings cannot be
/// serialized to RON.
pub fn write_texture_settings(
    settings: &TextureSourceSettings,
) -> Result<Vec<u8>, TextureSettingsWriteError> {
    settings
        .to_ron_bytes()
        .map_err(|err| TextureSettingsWriteError::Ron(err.to_string()))
}

/// # Errors
///
/// Returns [`TextureSettingsError`] if `bytes` is not valid UTF-8 or is not
/// well-formed RON for a texture settings document.
pub fn read_texture_settings(bytes: &[u8]) -> Result<TextureSourceSettings, TextureSettingsError> {
    let text = std::str::from_utf8(bytes)?;
    ron::from_str(text).map_err(|err| TextureSettingsError::Ron(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_path_tracks_png_and_exr_sources() {
        assert_eq!(
            texture_settings_source_path("textures/objects/sword_d.png"),
            "textures/objects/sword_d.texture.ron"
        );
        assert_eq!(
            texture_settings_source_path("Textures/Objects/SkyProbe.exr"),
            "textures/objects/skyprobe.texture.ron"
        );
    }

    #[test]
    fn settings_round_trip_as_ron() {
        let settings = TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Srgb,
            role: TextureRole::Albedo,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: None,
            shape: None,
        };
        let bytes = write_texture_settings(&settings).unwrap();
        assert_eq!(read_texture_settings(&bytes).unwrap(), settings);
    }
}
