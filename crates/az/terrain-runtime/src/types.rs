use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainCoord {
    pub x: i32,
    pub y: i32,
}

impl TerrainCoord {
    /// Parse the native `r_<signed-x>_<signed-y>` terrain-tile identity.
    #[must_use]
    pub fn from_region_segment(segment: &str) -> Option<Self> {
        let coordinates = segment.strip_prefix("r_")?;
        let mut parts = coordinates.split('_');
        let x = parts.next()?.parse().ok()?;
        let y = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some(Self { x, y })
    }

    /// Resolve the terrain tile segment from an asset-catalog path.
    #[must_use]
    pub fn from_region_path(path: &str) -> Option<Self> {
        let mut parts = path.split(['/', '\\']).filter(|part| !part.is_empty());
        while let Some(part) = parts.next() {
            if part.eq_ignore_ascii_case("regions") {
                return Self::from_region_segment(parts.next()?);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainBounds {
    pub min: Vec2,
    pub max: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainHeightRange {
    pub min: f32,
    pub max: f32,
}

impl TerrainHeightRange {
    /// Decode one normalized `u16` sample into terrain world units.
    #[must_use]
    pub fn decode_sample(self, sample: u16) -> f32 {
        f32::from(sample).mul_add((self.max - self.min) / f32::from(u16::MAX), self.min)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainResolution {
    pub height_spacing: f32,
    pub surface_spacing: f32,
}
