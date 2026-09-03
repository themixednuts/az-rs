//! Bevy `AssetLoader` for legacy `LyShine` `.font` descriptors.
//!
//! `LyShine` fonts use a two-tier asset:
//!
//! - `lyshineui/fonts/<name>.font` — a small XML descriptor
//!   pointing at the actual face file. Format:
//!
//!   ```xml
//!   <fontshader>
//!       <font path="NimbusSanNov-Reg.otf" fontsize="36" .../>
//!       <effectfile path="LyShineUI/Fonts/_SharedFontEffects.xml"/>
//!       <sizecache> ... </sizecache>
//!   </fontshader>
//!   ```
//!
//! - `lyshineui/fonts/<name>.ttf` / `.otf` — the binary font face.
//!
//! Bevy's stock `FontLoader` only claims `.ttf` / `.otf` and tries
//! to parse anything else as a binary font — that's why a direct
//! `asset_server.load::<Font>("lyshineui/fonts/nimbus_regular.font")`
//! fails with `An offset was out of bounds`. We register a
//! descriptor loader so the `.font` path resolves through the XML
//! to the underlying face file, then return a fully-decoded
//! `Font` asset.
//!
//! The descriptor's `path=` attribute is relative to the
//! descriptor's own directory (so `path="NimbusSanNov-Reg.otf"` in
//! `lyshineui/fonts/nimbus_regular.font` means
//! `lyshineui/fonts/NimbusSanNov-Reg.otf`).

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AssetPath, LoadContext};
use bevy::prelude::*;
use bevy::text::Font;
use quick_xml::events::Event;
use quick_xml::reader::Reader as XmlReader;
use thiserror::Error;

/// File extensions handled by [`LyShineFontDescriptorLoader`].
pub const LYSHINE_FONT_DESCRIPTOR_EXTENSIONS: &[&str] = &["font"];

/// Test whether a path refers to a `.font` descriptor (used by
/// callers that want to special-case descriptor paths before
/// handing them to the asset server).
#[must_use]
pub fn is_font_descriptor_path(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".font")
}

/// Bevy `AssetLoader` for `.font` XML descriptors.
#[derive(Default, TypePath)]
pub struct LyShineFontDescriptorLoader;

impl AssetLoader for LyShineFontDescriptorLoader {
    type Asset = Font;
    type Settings = ();
    type Error = LyShineFontDescriptorError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut xml_bytes = Vec::new();
        reader
            .read_to_end(&mut xml_bytes)
            .await
            .map_err(LyShineFontDescriptorError::Read)?;
        let face_filename = parse_face_filename(&xml_bytes)?;

        // Resolve the descriptor-relative `path=` to a full asset
        // path. `load_context.path().parent()` returns the parent
        // dir of the descriptor (e.g. `lyshineui/fonts`). The face
        // filename is sibling-relative.
        let descriptor_path = load_context.path().clone();
        let parent = descriptor_path
            .parent()
            .ok_or(LyShineFontDescriptorError::NoParentDirectory)?;
        let face_path = parent.resolve(&AssetPath::parse(&face_filename));

        let face_bytes = load_context
            .read_asset_bytes(face_path.clone())
            .await
            .map_err(|err| LyShineFontDescriptorError::ReadFace {
                face_path: face_path.to_string(),
                message: err.to_string(),
            })?;

        Ok(Font::from_bytes(face_bytes))
    }

    fn extensions(&self) -> &[&str] {
        LYSHINE_FONT_DESCRIPTOR_EXTENSIONS
    }
}

/// Errors produced by [`LyShineFontDescriptorLoader`].
#[derive(Debug, Error)]
pub enum LyShineFontDescriptorError {
    #[error("read .font descriptor bytes: {0}")]
    Read(#[source] std::io::Error),
    #[error("parse .font descriptor XML: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("descriptor has no `<font path=...>` attribute")]
    MissingFontPath,
    #[error("descriptor has no parent directory in its asset path")]
    NoParentDirectory,
    #[error("read referenced face file `{face_path}`: {message}")]
    ReadFace { face_path: String, message: String },
}

/// Parse `<fontshader><font path="..." ...></fontshader>` and
/// return the `path` attribute.
fn parse_face_filename(xml: &[u8]) -> Result<String, LyShineFontDescriptorError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Empty(e) | Event::Start(e)) => {
                if e.name().as_ref().eq_ignore_ascii_case(b"font") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref().eq_ignore_ascii_case(b"path") {
                            let value = attr
                                .normalized_value(quick_xml::XmlVersion::default())
                                .map_err(LyShineFontDescriptorError::Xml)?;
                            return Ok(value.into_owned());
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(err.into()),
            _ => {}
        }
    }
    Err(LyShineFontDescriptorError::MissingFontPath)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIMBUS_REGULAR: &[u8] = br#"
<fontshader>
    <font path="NimbusSanNov-Reg.otf" fontsize="36" widthslots="24" heightslots="16" sizebehavior="sizecache" />
    <effectfile path="LyShineUI/Fonts/_SharedFontEffects.xml"/>
    <sizecache>
        <fontcache size="12"/>
        <fontcache size="20"/>
    </sizecache>
</fontshader>
"#;

    const NO_FONT_TAG: &[u8] = br#"<fontshader><effectfile path="x.xml"/></fontshader>"#;

    #[test]
    fn parses_face_filename_from_real_descriptor() {
        let path = parse_face_filename(NIMBUS_REGULAR).unwrap();
        assert_eq!(path, "NimbusSanNov-Reg.otf");
    }

    #[test]
    fn missing_font_tag_reports_clear_error() {
        let err = parse_face_filename(NO_FONT_TAG).unwrap_err();
        assert!(matches!(err, LyShineFontDescriptorError::MissingFontPath));
    }

    #[test]
    fn case_insensitive_font_tag_match() {
        let xml = br#"<FontShader><FONT Path="Foo.ttf"/></FontShader>"#;
        let path = parse_face_filename(xml).unwrap();
        assert_eq!(path, "Foo.ttf");
    }

    #[test]
    fn is_font_descriptor_path_recognises_extension() {
        assert!(is_font_descriptor_path(
            "lyshineui/fonts/nimbus_regular.font"
        ));
        assert!(is_font_descriptor_path("FONTS/NIMBUS.FONT"));
        assert!(!is_font_descriptor_path("fonts/nimbus.otf"));
        assert!(!is_font_descriptor_path("fonts/nimbus"));
    }
}
