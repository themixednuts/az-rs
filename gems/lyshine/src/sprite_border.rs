//! `LyShine` sprite-border sidecar handling.
//!
//! `LyShine` can pair a `.dds` UI sprite with a `.sprite` XML sidecar that holds
//! the 9-slice border definitions:
//!
//! ```xml
//! <Sprite versionNumber="2">
//!   <Sprite m_left="0.00052083336" m_right="0.99947917"
//!           m_top="0.0" m_bottom="1.0"/>
//! </Sprite>
//! ```
//!
//! The four values are normalized UV coordinates — `m_left` /
//! `m_top` are the **fractions from the top/left edges** at which
//! the centre slice begins, and `m_right` / `m_bottom` are the
//! **fractions from the top/left edges** at which it ends.
//!
//! O3DE derives the pixel borders fed into the 9-slice renderer like this
//! (O3DE reference: `Gems/LyShine/Code/Source/UiImageComponent.cpp:1318`):
//!
//! ```cpp
//! ISprite::Borders cellUvBorders(sprite->GetCellUvBorders(cellIndex));
//! float leftBorder   = cellUvBorders.m_left;
//! float rightBorder  = 1.0f - cellUvBorders.m_right;
//! float topBorder    = cellUvBorders.m_top;
//! float bottomBorder = 1.0f - cellUvBorders.m_bottom;
//! // …multiply by texture size to get pixel borders…
//! ```
//!
//! Bevy's `TextureSlicer::border` expects [`BorderRect`] in
//! **texture pixels** measured inward from each edge. So our
//! conversion is:
//!
//! ```text
//! border.left   = m_left            * texture_width
//! border.right  = (1.0 - m_right)   * texture_width
//! border.top    = m_top             * texture_height
//! border.bottom = (1.0 - m_bottom)  * texture_height
//! ```
//!
//! See [`pixel_border_rect`] for the implementation.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::sprite::BorderRect;
use lyshine::{SpriteBorders, SpriteItem, visit_sprite};
use thiserror::Error;

/// Bevy `Asset` carrying the parsed 9-slice borders for one
/// `.sprite` sidecar.
///
/// `borders` is the **raw** Lumberyard `m_left/m_right/m_top/m_bottom`
/// tuple (normalized 0..1 UV space). Call [`pixel_border_rect`] to
/// convert to a Bevy [`BorderRect`] in pixels once the texture
/// dimensions are known.
#[derive(Asset, TypePath, Debug, Clone, Copy, PartialEq)]
pub struct LyShineSpriteAsset {
    pub borders: SpriteBorders,
}

impl LyShineSpriteAsset {
    /// Build an asset with no slicing — `m_left=0, m_right=1,
    /// m_top=0, m_bottom=1`. Used as the fallback when a sprite
    /// has no companion `.sprite` file (treats the entire texture
    /// as the centre slice with zero borders).
    #[must_use]
    pub const fn default_borders() -> Self {
        Self {
            borders: SpriteBorders::DEFAULT,
        }
    }
}

/// File extensions handled by [`LyShineSpriteAssetLoader`].
pub const LYSHINE_SPRITE_EXTENSIONS: &[&str] = &["sprite"];

/// Bevy `AssetLoader` for `LyShine` `.sprite` sidecars.
#[derive(Default, TypePath)]
pub struct LyShineSpriteAssetLoader;

impl AssetLoader for LyShineSpriteAssetLoader {
    type Asset = LyShineSpriteAsset;
    type Settings = ();
    type Error = LyShineSpriteAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(LyShineSpriteAssetError::Read)?;

        // The XML may carry both a root-level `<Sprite>` and per-cell
        // borders. For 9-slice rendering of a non-spritesheet image
        // (every UI panel chrome) only the root borders matter.
        // Capture the *first* `Borders` event whose context is
        // `SpriteContext::Root` and return it.
        let mut root_borders: Option<SpriteBorders> = None;
        visit_sprite(&bytes, |item| {
            if let SpriteItem::Borders(b) = item
                && b.context == lyshine::SpriteContext::Root
                && root_borders.is_none()
            {
                root_borders = Some(b.borders);
            }
            Ok(())
        })
        .map_err(|err| LyShineSpriteAssetError::Parse(err.to_string()))?;

        Ok(LyShineSpriteAsset {
            // Sprites without a `<Borders>` element are treated as
            // "no slicing" (default borders cover the whole texture).
            borders: root_borders.unwrap_or(SpriteBorders::DEFAULT),
        })
    }

    fn extensions(&self) -> &[&str] {
        LYSHINE_SPRITE_EXTENSIONS
    }
}

/// Errors produced by [`LyShineSpriteAssetLoader`].
#[derive(Debug, Error)]
pub enum LyShineSpriteAssetError {
    #[error("read .sprite bytes: {0}")]
    Read(#[source] std::io::Error),
    #[error("parse .sprite XML: {0}")]
    Parse(String),
}

/// Convert Lumberyard `SpriteBorders` (normalized UV) into Bevy's
/// `BorderRect` (texture pixels measured inward from each edge).
///
/// Mirrors `UiImageComponent::RenderSlicedSprite`
/// (O3DE reference: `Gems/LyShine/Code/Source/UiImageComponent.cpp:1318-1336`):
///
/// ```text
/// leftBorder   = m_left
/// rightBorder  = 1.0 - m_right
/// topBorder    = m_top
/// bottomBorder = 1.0 - m_bottom
/// pixel_border = uv_border * texture_size_along_axis
/// ```
///
/// Bevy 0.18's `BorderRect` uses two `Vec2`s rather than four
/// scalar `left/right/top/bottom` fields:
///
/// - `min_inset.x` ⇄ left
/// - `min_inset.y` ⇄ top
/// - `max_inset.x` ⇄ right
/// - `max_inset.y` ⇄ bottom
///
/// If `texture_size` is zero on either axis the corresponding
/// border collapses to zero (no slicing on that axis).
#[must_use]
pub fn pixel_border_rect(borders: SpriteBorders, texture_size: Vec2) -> BorderRect {
    let width = texture_size.x.max(0.0);
    let height = texture_size.y.max(0.0);
    BorderRect {
        min_inset: Vec2::new(borders.left * width, borders.top * height),
        max_inset: Vec2::new(
            (1.0 - borders.right).max(0.0) * width,
            (1.0 - borders.bottom).max(0.0) * height,
        ),
    }
}

/// Derive the `.sprite` sidecar asset path for a `.dds` sprite path.
///
/// The conventional pair shares a directory and stem: `foo.dds` and
/// `foo.sprite`. The path normalization (lowercase + slash flip)
/// matches what `AssetServer::load` does internally.
#[must_use]
pub fn sprite_sidecar_path(dds_path: &str) -> Option<String> {
    let normalized = dds_path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let stem = lower.strip_suffix(".dds")?;
    Some(format!("{stem}.sprite"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Representative Lumberyard `.sprite` border values.
    const NAVBARBG_SPRITE: &[u8] = br#"<Sprite versionNumber="2">
 <Sprite m_left="0.00052083336" m_right="0.99947917" m_bottom="1"/>
</Sprite>"#;

    // Sprite with all four borders set (more typical 9-slice panel).
    const PANEL_SPRITE: &[u8] = br#"<Sprite versionNumber="2">
 <Sprite m_left="0.25" m_right="0.75" m_top="0.125" m_bottom="0.875"/>
</Sprite>"#;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "every border in PANEL_SPRITE is a dyadic fraction (0.25/0.75/0.125/0.875) that parses to an exact f32, so an epsilon would only weaken the assertion"
    )]
    fn parses_root_borders_with_all_four_attributes_set() {
        // Sanity check the visitor's behaviour when every
        // `m_left/m_right/m_top/m_bottom` attribute is present — the
        // real-world navbar sprite skips `m_top` so we can't cover
        // that path with `NAVBARBG_SPRITE` alone.
        let mut got: Option<SpriteBorders> = None;
        visit_sprite(PANEL_SPRITE, |item| {
            if let SpriteItem::Borders(b) = item
                && b.context == lyshine::SpriteContext::Root
                && got.is_none()
            {
                got = Some(b.borders);
            }
            Ok(())
        })
        .unwrap();
        let borders = got.expect("root borders");
        assert_eq!(borders.left, 0.25);
        assert_eq!(borders.right, 0.75);
        assert_eq!(borders.top, 0.125);
        assert_eq!(borders.bottom, 0.875);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the two exact assertions cover an absent attribute (default 0.0) and a literal 1.0, both bit-exact; the inexact borders above already use epsilons"
    )]
    fn parses_root_borders_from_real_navbar_sprite() {
        // We can't run the AssetLoader directly without a Bevy
        // load context, so test the underlying visitor through the
        // same code path the loader uses.
        let mut got: Option<SpriteBorders> = None;
        visit_sprite(NAVBARBG_SPRITE, |item| {
            if let SpriteItem::Borders(b) = item
                && b.context == lyshine::SpriteContext::Root
                && got.is_none()
            {
                got = Some(b.borders);
            }
            Ok(())
        })
        .unwrap();
        let borders = got.expect("root borders");
        assert!((borders.left - 0.000_520_833_36).abs() < 1e-7);
        assert!((borders.right - 0.999_479_2).abs() < 1e-5);
        // `m_top` absent → defaults to 0.0.
        assert_eq!(borders.top, 0.0);
        assert_eq!(borders.bottom, 1.0);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "dyadic borders times whole-pixel sizes give exact IEEE-754 products (50.0 and 12.5), so this pins the formula rather than an approximation"
    )]
    fn pixel_border_matches_lumberyard_formula_for_panel() {
        // m_left=0.25, m_right=0.75, m_top=0.125, m_bottom=0.875
        // Texture: 200×100
        //   left   = 0.25 * 200 = 50
        //   right  = (1.0 - 0.75) * 200 = 50
        //   top    = 0.125 * 100 = 12.5
        //   bottom = (1.0 - 0.875) * 100 = 12.5
        let borders = SpriteBorders::new(0.25, 0.75, 0.125, 0.875);
        let rect = pixel_border_rect(borders, Vec2::new(200.0, 100.0));
        assert_eq!(rect.min_inset.x, 50.0);
        assert_eq!(rect.max_inset.x, 50.0);
        assert_eq!(rect.min_inset.y, 12.5);
        assert_eq!(rect.max_inset.y, 12.5);
    }

    #[test]
    fn pixel_border_zero_texture_size_collapses() {
        let borders = SpriteBorders::new(0.5, 0.5, 0.5, 0.5);
        let rect = pixel_border_rect(borders, Vec2::ZERO);
        assert_eq!(rect.min_inset, Vec2::ZERO);
        assert_eq!(rect.max_inset, Vec2::ZERO);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the vertical borders are 0.0 * height and (1.0 - 1.0) * height, exactly zero; the inexact horizontal pair above already uses epsilons"
    )]
    fn pixel_border_navbar_sprite_yields_tiny_horizontal_borders() {
        // navbarbg.sprite at 1920×188 → ~1px each side, 0px top/bottom.
        let borders = SpriteBorders::new(0.000_520_833_36, 0.999_479_2, 0.0, 1.0);
        let rect = pixel_border_rect(borders, Vec2::new(1920.0, 188.0));
        assert!(
            (rect.min_inset.x - 1.0).abs() < 0.01,
            "got {}",
            rect.min_inset.x
        );
        assert!(
            (rect.max_inset.x - 1.0).abs() < 0.01,
            "got {}",
            rect.max_inset.x
        );
        assert_eq!(rect.min_inset.y, 0.0);
        assert_eq!(rect.max_inset.y, 0.0);
    }

    #[test]
    fn sprite_sidecar_path_swaps_extension() {
        assert_eq!(
            sprite_sidecar_path("textures/lyshineui/images/panel.dds").as_deref(),
            Some("textures/lyshineui/images/panel.sprite")
        );
        // Mixed case + backslashes get normalized.
        assert_eq!(
            sprite_sidecar_path("Textures\\LyShineUI\\Foo.DDS").as_deref(),
            Some("textures/lyshineui/foo.sprite")
        );
        // Non-DDS paths get None.
        assert_eq!(sprite_sidecar_path("textures/foo.ttf"), None);
    }
}
