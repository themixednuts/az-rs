//! Engine texture format selection helpers.
//!
//! The asset pipeline uses these when a legacy source references a
//! texture source path (`.tif`, `.dds`, `.sprite`, etc.) and the
//! generated product needs to point at the engine texture asset on
//! disk.

use serde::{Deserialize, Serialize};

use az_filesystem::{normalize_source_path, texture_source_asset_path};

/// Texture output format selected by an asset builder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineTextureFormat {
    /// Preserve source DDS files for desktop builds.
    #[default]
    Dds,
    /// Transcode textures to KTX2/Basis Universal.
    Ktx2,
}

/// Map a legacy texture source reference to the engine texture product
/// path selected by a texture format.
#[must_use]
pub fn engine_texture_asset_path(source_path: &str, texture_format: EngineTextureFormat) -> String {
    let dds_path = texture_source_asset_path(source_path);
    match texture_format {
        EngineTextureFormat::Dds => dds_path,
        EngineTextureFormat::Ktx2 => replace_extension(&dds_path, "ktx2"),
    }
}

/// Map a same-stem sidecar such as `.sprite` or `.texatlasidx` to
/// the backing texture product path selected by a texture format.
#[must_use]
pub fn same_stem_texture_asset_path(
    source_path: &str,
    sidecar_extension: &str,
    texture_format: EngineTextureFormat,
) -> String {
    let source_path = normalize_source_path(source_path);
    let texture_source_path = source_path
        .strip_suffix(sidecar_extension)
        .map_or_else(|| source_path.clone(), |base| format!("{base}.dds"));
    engine_texture_asset_path(&texture_source_path, texture_format)
}

fn replace_extension(path: &str, extension: &str) -> String {
    path.rsplit_once('.').map_or_else(
        || format!("{path}.{extension}"),
        |(base, _)| format!("{base}.{extension}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_paths_follow_selected_output_format() {
        assert_eq!(
            engine_texture_asset_path("Objects/Foo/Diff.tif", EngineTextureFormat::Dds),
            "objects/foo/diff.dds"
        );
        assert_eq!(
            engine_texture_asset_path("Objects/Foo/Diff.tif", EngineTextureFormat::Ktx2),
            "objects/foo/diff.ktx2"
        );
        assert_eq!(
            same_stem_texture_asset_path(
                "LyShineUI/Images/Common/GradientBox.sprite",
                ".sprite",
                EngineTextureFormat::Ktx2,
            ),
            "lyshineui/images/common/gradientbox.ktx2"
        );
    }
}
