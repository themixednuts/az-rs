use image::{ColorType, ImageFormat};
use thiserror::Error;

use crate::DecodedImage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage16 {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedPng {
    Rgba8(DecodedImage),
    Rgba16(DecodedImage16),
}

#[derive(Debug, Error)]
pub enum PngCodecError {
    #[error("PNG codec: {0}")]
    Image(#[from] image::ImageError),
}

/// Read a PNG authoring source into tightly packed RGBA pixels.
///
/// # Errors
///
/// Returns [`PngCodecError`] if the PNG is malformed or uses an unsupported
/// image feature.
pub fn read_png(bytes: &[u8]) -> Result<DecodedPng, PngCodecError> {
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)?;
    let color = image.color();
    let (width, height) = (image.width(), image.height());
    if is_16_bit(color) {
        let rgba = image.into_rgba16().into_raw();
        Ok(DecodedPng::Rgba16(DecodedImage16 {
            width,
            height,
            rgba,
        }))
    } else {
        let rgba = image.into_rgba8().into_raw();
        Ok(DecodedPng::Rgba8(DecodedImage {
            width,
            height,
            rgba,
        }))
    }
}

const fn is_16_bit(color: ColorType) -> bool {
    matches!(
        color,
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16
    )
}
