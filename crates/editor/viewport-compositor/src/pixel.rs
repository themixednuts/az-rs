//! Frame pixel primitives shared by the capture classifier and the PNG
//! comparison. Both read BGRA because that is the color format WGC delivers.

use serde::Serialize;

/// Lime corner markers GPUI paints at the independently current layout rect.
pub const LAYOUT_CORNER: [u8; 4] = [0x39, 0xff, 0x14, 0xff];

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Rect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Rect {
    pub const fn contains(self, x: u32, y: u32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Extents matter only to the capture classifier. `compare` never measures the
/// diagnostic rectangle, it only excludes the pixels inside it.
#[cfg(windows)]
impl Rect {
    pub const fn width(self) -> u32 {
        self.right.saturating_sub(self.left)
    }

    pub const fn height(self) -> u32 {
        self.bottom.saturating_sub(self.top)
    }
}

pub fn color_bounds_bgra(
    data: &[u8],
    width: u32,
    height: u32,
    rgba: [u8; 4],
    tolerance: u8,
) -> Option<Rect> {
    let mut bounds = Rect {
        left: width,
        top: height,
        right: 0,
        bottom: 0,
    };
    let mut found = 0_u32;
    for y in 0..height {
        for x in 0..width {
            let Some(pixel) = bgra_pixel(data, width, x, y) else {
                continue;
            };
            let actual = [pixel[2], pixel[1], pixel[0], pixel[3]];
            if color_near(actual, rgba, tolerance) {
                bounds.left = bounds.left.min(x);
                bounds.top = bounds.top.min(y);
                bounds.right = bounds.right.max(x + 1);
                bounds.bottom = bounds.bottom.max(y + 1);
                found += 1;
            }
        }
    }
    (found >= 8).then_some(bounds)
}

pub fn color_near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) -> bool {
    actual
        .into_iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= tolerance)
}

pub fn bgra_pixel(data: &[u8], width: u32, x: u32, y: u32) -> Option<[u8; 4]> {
    let index = usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4).ok()?;
    let pixel = data.get(index..index + 4)?;
    Some([pixel[0], pixel[1], pixel[2], pixel[3]])
}

#[cfg(test)]
mod tests {
    use super::color_near;

    #[test]
    fn color_tolerance_is_per_channel() {
        assert!(color_near([10, 20, 30, 255], [12, 18, 31, 255], 2));
        assert!(!color_near([10, 20, 30, 255], [13, 20, 30, 255], 2));
    }
}
