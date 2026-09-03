//! Shared texture for GPU communication between runtime viewport streams and GPUI.
//!
//! This module provides a thread-safe way to publish viewport frame
//! readiness metadata from a runtime-host viewport producer to GPUI.

use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Errors that can occur when working with shared viewport textures
#[derive(Debug, Error)]
pub enum TextureError {
    /// The texture mutex was poisoned (panic occurred while locked)
    #[error("Texture mutex poisoned")]
    MutexPoisoned,

    /// Invalid texture dimensions provided
    #[error("Invalid dimensions: {0}x{1}")]
    InvalidDimensions(u32, u32),
}

/// Shared viewport frame slot between a runtime viewport producer and GPUI.
///
/// Runtime-host viewport streaming should populate this slot through a
/// process boundary. The editor UI only consumes the latest frame state.
///
/// This implementation provides:
/// - Thread-safe texture sharing via Arc<Mutex<...>>
/// - Resize support with dimension tracking
/// - Generation tracking to detect texture updates
/// - Error handling for robustness
#[derive(Clone)]
pub struct SharedViewportTexture {
    inner: Arc<Mutex<TextureState>>,
}

impl gpui::Global for SharedViewportTexture {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportFrameSource {
    pub kind: String,
    pub locator: String,
    pub platform: String,
    pub byte_length: u64,
    pub pixel_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportFrameState {
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub source: Option<ViewportFrameSource>,
}

struct TextureState {
    has_frame: bool,
    width: u32,
    height: u32,
    generation: u64,
    source: Option<ViewportFrameSource>,
}

impl SharedViewportTexture {
    /// Create a new shared texture with the given dimensions
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(TextureState {
                has_frame: false,
                width,
                height,
                generation: 0,
                source: None,
            })),
        }
    }

    /// Publish a viewport frame's dimensions atomically.
    ///
    /// # Errors
    ///
    /// Returns [`TextureError::InvalidDimensions`] if `width` or `height` is zero,
    /// or [`TextureError::MutexPoisoned`] if the internal lock was poisoned.
    pub fn publish_frame(&self, width: u32, height: u32) -> Result<(), TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::InvalidDimensions(width, height));
        }

        let mut state = self.inner.lock().map_err(|_| TextureError::MutexPoisoned)?;

        state.has_frame = true;
        state.width = width;
        state.height = height;
        state.generation += 1;
        state.source = None;
        drop(state);

        Ok(())
    }

    /// Publish a viewport frame source and allocate the next editor-local
    /// revision for consumers that need to detect an update.
    ///
    /// # Errors
    ///
    /// Returns [`TextureError::InvalidDimensions`] if the frame width or height is
    /// zero, or [`TextureError::MutexPoisoned`] if the internal lock was poisoned.
    pub fn publish_frame_source(
        &self,
        width: u32,
        height: u32,
        source: ViewportFrameSource,
    ) -> Result<u64, TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::InvalidDimensions(width, height));
        }

        let mut state = self.inner.lock().map_err(|_| TextureError::MutexPoisoned)?;

        state.has_frame = true;
        state.width = width;
        state.height = height;
        state.generation = state.generation.saturating_add(1);
        state.source = Some(source);
        Ok(state.generation)
    }

    /// Get the latest published viewport frame metadata.
    #[must_use]
    pub fn get(&self) -> Option<ViewportFrameState> {
        let state = self.inner.lock().ok()?;
        state.has_frame.then_some(ViewportFrameState {
            width: state.width,
            height: state.height,
            generation: state.generation,
            source: state.source.clone(),
        })
    }

    /// Get current dimensions
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        self.inner.lock().map_or((0, 0), |s| (s.width, s.height))
    }

    /// Check if texture is available
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.inner.lock().is_ok_and(|s| s.has_frame)
    }

    /// Resize viewport
    ///
    /// Updates the dimensions and clears the current texture,
    /// forcing the producer to publish a fresh frame at the new size.
    ///
    /// # Errors
    ///
    /// Returns [`TextureError::InvalidDimensions`] if `width` or `height` is zero,
    /// or [`TextureError::MutexPoisoned`] if the internal lock was poisoned.
    pub fn resize(&self, width: u32, height: u32) -> Result<(), TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::InvalidDimensions(width, height));
        }

        let mut state = self.inner.lock().map_err(|_| TextureError::MutexPoisoned)?;

        state.width = width;
        state.height = height;
        // Clear texture to force a producer refresh.
        state.has_frame = false;
        state.source = None;
        drop(state);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_viewport_texture_tracks_frame_generation() {
        let texture = SharedViewportTexture::new(320, 200);
        assert_eq!(texture.get(), None);

        texture.publish_frame(640, 480).unwrap();
        assert_eq!(
            texture.get(),
            Some(ViewportFrameState {
                width: 640,
                height: 480,
                generation: 1,
                source: None,
            })
        );

        texture.resize(800, 600).unwrap();
        assert_eq!(texture.dimensions(), (800, 600));
        assert_eq!(texture.get(), None);

        let source = ViewportFrameSource {
            kind: "gpuSurface".to_string(),
            locator: "dxgi://adapter/0/texture/99".to_string(),
            platform: "windows".to_string(),
            byte_length: 0,
            pixel_format: "bgra8Unorm".to_string(),
        };
        assert_eq!(
            texture
                .publish_frame_source(1024, 768, source.clone())
                .unwrap(),
            2
        );
        assert_eq!(
            texture.get(),
            Some(ViewportFrameState {
                width: 1024,
                height: 768,
                generation: 2,
                source: Some(source),
            })
        );
    }
}
