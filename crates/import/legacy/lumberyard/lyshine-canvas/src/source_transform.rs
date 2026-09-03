//! Legacy `LyShine` UI canvas source classification.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiCanvasAssetSourceTransform;

impl LegacySourceTransform for UiCanvasAssetSourceTransform {
    type Error = UiCanvasAssetSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let source_path = input.source_path.to_string();
        if !is_legacy_ui_canvas_asset_source(&source_path) {
            return Err(UiCanvasAssetSourceTransformError::UnsupportedPath { path: source_path });
        }

        Ok(LegacySourceOutput::Unclassified {
            reason: format!(
                "legacy LyShine UI canvas {source_path} is recognized as UiCanvasFileObject input but is not imported as source yet; emit .uicanvas.ron only after a full reflected UI component field map is complete"
            ),
        })
    }
}

#[must_use]
pub fn is_legacy_ui_canvas_asset_source(source_path: &str) -> bool {
    let path = normalize_source_path(source_path);
    path.ends_with(".uicanvas") || path.ends_with(".dynamicuicanvas")
}

#[derive(Debug, thiserror::Error)]
pub enum UiCanvasAssetSourceTransformError {
    #[error("unsupported UI canvas source path {path}")]
    UnsupportedPath { path: String },
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    use super::*;

    #[test]
    fn classifies_only_legacy_ui_canvas_assets() {
        assert!(is_legacy_ui_canvas_asset_source(
            "LyShineUI/Main Menu/Landing.uicanvas"
        ));
        assert!(is_legacy_ui_canvas_asset_source(
            "LyShineUI/Main Menu/Landing.dynamicuicanvas"
        ));
        assert!(!is_legacy_ui_canvas_asset_source(
            "lyshineui/main menu/landing.uicanvas.ron"
        ));
        assert!(!is_legacy_ui_canvas_asset_source(
            "lyshineui/main menu/landing.uidb"
        ));
    }

    #[test]
    fn transform_leaves_legacy_canvas_known_pending_until_source_map_is_complete() {
        let output = UiCanvasAssetSourceTransform
            .transform(LegacySourceInput::new(
                "LyShineUI/Main Menu/Landing.dynamicuicanvas",
                b"legacy-objectstream-bytes",
            ))
            .unwrap();

        assert!(output.artifact().is_none());
        match output {
            LegacySourceOutput::Unclassified { reason } => {
                assert!(reason.contains("lyshineui/main menu/landing.dynamicuicanvas"));
                assert!(reason.contains("UiCanvasFileObject"));
                assert!(reason.contains("full reflected UI component field map"));
                assert!(reason.contains(".uicanvas.ron"));
            }
            other => panic!("expected unclassified UI canvas, got {other:?}"),
        }
    }
}
