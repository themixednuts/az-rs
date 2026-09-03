use std::io::Cursor;

use exr::prelude::{ReadChannels, ReadLayers, Vec2, read};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedFloatImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
}

#[derive(Debug, Error)]
pub enum ExrCodecError {
    #[error("RGBA8 image contains {actual} bytes, expected {expected}")]
    RgbaSize { expected: u64, actual: usize },
    #[error("RGBA32F image contains {actual} samples, expected {expected}")]
    Rgba32FloatSize { expected: u64, actual: usize },
    #[error("image dimension `{field}` {value} exceeds u32")]
    DimensionTooLarge { field: &'static str, value: usize },
    #[error("image dimensions are too large")]
    SizeOverflow,
    #[error("OpenEXR codec: {0}")]
    Exr(#[from] exr::error::Error),
}

/// Read the first RGBA `OpenEXR` layer into tightly packed RGBA8 pixels.
///
/// # Errors
///
/// Returns [`ExrCodecError`] if `OpenEXR` parsing fails, image dimensions exceed
/// `u32`, or dimensions overflow the RGBA8 buffer size.
pub fn read_rgba8_exr(bytes: &[u8]) -> Result<DecodedImage, ExrCodecError> {
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(
            |resolution, _channels| Rgba8Buffer::new(resolution),
            |pixels, position, rgba: (f32, f32, f32, f32)| {
                pixels.set(position, rgba.into());
            },
        )
        .first_valid_layer()
        .all_attributes()
        .non_parallel()
        .from_buffered(Cursor::new(bytes))?;

    let layer = image.layer_data;
    let pixels = layer.channel_data.pixels;
    let width = u32::try_from(pixels.width).map_err(|_| ExrCodecError::DimensionTooLarge {
        field: "width",
        value: pixels.width,
    })?;
    let height = u32::try_from(pixels.height).map_err(|_| ExrCodecError::DimensionTooLarge {
        field: "height",
        value: pixels.height,
    })?;
    let expected = usize::try_from(expected_rgba_len(width, height)?)
        .map_err(|_| ExrCodecError::SizeOverflow)?;
    if pixels.rgba.len() != expected {
        return Err(ExrCodecError::RgbaSize {
            expected: expected as u64,
            actual: pixels.rgba.len(),
        });
    }

    Ok(DecodedImage {
        width,
        height,
        rgba: pixels.rgba,
    })
}

/// Read the first RGBA `OpenEXR` layer into tightly packed RGBA32F pixels.
///
/// # Errors
///
/// Returns [`ExrCodecError`] if `OpenEXR` parsing fails, image dimensions exceed
/// `u32`, or dimensions overflow the RGBA32F buffer size.
pub fn read_rgba32f_exr(bytes: &[u8]) -> Result<DecodedFloatImage, ExrCodecError> {
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(
            |resolution, _channels| Rgba32fBuffer::new(resolution),
            |pixels, position, rgba: (f32, f32, f32, f32)| {
                pixels.set(position, rgba.into());
            },
        )
        .first_valid_layer()
        .all_attributes()
        .non_parallel()
        .from_buffered(Cursor::new(bytes))?;

    let layer = image.layer_data;
    let pixels = layer.channel_data.pixels;
    let width = u32::try_from(pixels.width).map_err(|_| ExrCodecError::DimensionTooLarge {
        field: "width",
        value: pixels.width,
    })?;
    let height = u32::try_from(pixels.height).map_err(|_| ExrCodecError::DimensionTooLarge {
        field: "height",
        value: pixels.height,
    })?;
    let expected = usize::try_from(expected_rgba_len(width, height)?)
        .map_err(|_| ExrCodecError::SizeOverflow)?;
    if pixels.rgba.len() != expected {
        return Err(ExrCodecError::Rgba32FloatSize {
            expected: expected as u64,
            actual: pixels.rgba.len(),
        });
    }

    Ok(DecodedFloatImage {
        width,
        height,
        rgba: pixels.rgba,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct Rgba8Buffer {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Rgba8Buffer {
    fn new(size: Vec2<usize>) -> Self {
        let width = size.width();
        let height = size.height();
        let len = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(0);
        Self {
            width,
            height,
            rgba: vec![0; len],
        }
    }

    fn set(&mut self, position: Vec2<usize>, rgba: [f32; 4]) {
        let offset = position
            .y()
            .checked_mul(self.width)
            .and_then(|row| row.checked_add(position.x()))
            .and_then(|pixel| pixel.checked_mul(4));
        let Some(offset) = offset else {
            return;
        };
        if let Some(pixel) = self.rgba.get_mut(offset..offset + 4) {
            pixel[0] = sample_to_byte(rgba[0]);
            pixel[1] = sample_to_byte(rgba[1]);
            pixel[2] = sample_to_byte(rgba[2]);
            pixel[3] = sample_to_byte(rgba[3]);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Rgba32fBuffer {
    width: usize,
    height: usize,
    rgba: Vec<f32>,
}

impl Rgba32fBuffer {
    fn new(size: Vec2<usize>) -> Self {
        let width = size.width();
        let height = size.height();
        let len = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(0);
        Self {
            width,
            height,
            rgba: vec![0.0; len],
        }
    }

    fn set(&mut self, position: Vec2<usize>, rgba: [f32; 4]) {
        let offset = position
            .y()
            .checked_mul(self.width)
            .and_then(|row| row.checked_add(position.x()))
            .and_then(|pixel| pixel.checked_mul(4));
        let Some(offset) = offset else {
            return;
        };
        if let Some(pixel) = self.rgba.get_mut(offset..offset + 4) {
            pixel.copy_from_slice(&rgba);
        }
    }
}

fn expected_rgba_len(width: u32, height: u32) -> Result<u64, ExrCodecError> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ExrCodecError::SizeOverflow)
}

fn sample_to_byte(sample: f32) -> u8 {
    if sample.is_nan() {
        return 0;
    }
    // Clamped to 0.0..=1.0 then scaled, so the rounded value is always in
    // 0..=255 and the cast is exact; float-to-int `as` also saturates.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = (sample.clamp(0.0, 1.0) * 255.0).round() as u8;
    byte
}
