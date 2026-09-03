use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use crate::ParseError;
use thiserror::Error;

pub const SKY_LIGHT_LUT_MAGIC: &[u8; 4] = b"SKYL";
pub const SKY_LIGHT_LUT_VERSION: u16 = 2;
pub const SKY_LIGHT_LUT_TABLE_SET: u16 = 1;
pub const SKY_LIGHT_LUT_HEADER_SIZE: usize = 8;
pub const OPTICAL_DEPTH_ALTITUDE_SAMPLES: usize = 32;
pub const OPTICAL_DEPTH_VIEW_SAMPLES: usize = 256;
pub const OPTICAL_DEPTH_SAMPLE_COUNT: usize =
    OPTICAL_DEPTH_ALTITUDE_SAMPLES * OPTICAL_DEPTH_VIEW_SAMPLES;
pub const OPTICAL_DEPTH_SAMPLE_SIZE: usize = 8;
pub const OPTICAL_DEPTH_TABLE_SIZE: usize = OPTICAL_DEPTH_SAMPLE_COUNT * OPTICAL_DEPTH_SAMPLE_SIZE;
pub const TRANSMITTANCE_SAMPLE_COUNT: usize = 32;
pub const TRANSMITTANCE_SAMPLE_SIZE: usize = 12;
pub const TRANSMITTANCE_TABLE_SIZE: usize = TRANSMITTANCE_SAMPLE_COUNT * TRANSMITTANCE_SAMPLE_SIZE;
pub const SKY_LIGHT_LUT_SIZE: usize =
    SKY_LIGHT_LUT_HEADER_SIZE + OPTICAL_DEPTH_TABLE_SIZE + TRANSMITTANCE_TABLE_SIZE;

/// `engineassets/sky/optical.lut`.
///
/// Follows Lumberyard's `dev/Code/CryEngine/RenderDll/Common/RendElements/CRESky.cpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyLightOpticalLut<'a> {
    version: u16,
    table_set: u16,
    optical_depth: &'a [u8],
    transmittance: &'a [u8],
}

impl<'a> SkyLightOpticalLut<'a> {
    /// Parse a sky-light optical lookup table.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] if `bytes` is shorter than the
    /// header or than the table payload the header declares,
    /// [`ParseError::InvalidMagic`] if the first four bytes are not `SKYL`,
    /// [`ParseError::UnsupportedVersion`] for a version other than
    /// [`SKY_LIGHT_LUT_VERSION`], and [`ParseError::UnsupportedSkyLightLutTableSet`] for
    /// a table-set count this reader does not handle.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let header = bytes
            .get(..SKY_LIGHT_LUT_HEADER_SIZE)
            .ok_or(ParseError::UnexpectedEof {
                offset: 0,
                needed: SKY_LIGHT_LUT_HEADER_SIZE,
                actual: bytes.len(),
            })?;
        let magic = &header[..4];
        if magic != SKY_LIGHT_LUT_MAGIC {
            return Err(ParseError::InvalidMagic {
                asset: "sky light LUT",
                expected: SKY_LIGHT_LUT_MAGIC,
                found: magic.to_vec(),
            });
        }

        let version = u16::from_le_bytes([header[4], header[5]]);
        if version != SKY_LIGHT_LUT_VERSION {
            return Err(ParseError::UnsupportedVersion {
                asset: "sky light LUT",
                expected: SKY_LIGHT_LUT_VERSION,
                found: version,
            });
        }

        let table_set = u16::from_le_bytes([header[6], header[7]]);
        if table_set != SKY_LIGHT_LUT_TABLE_SET {
            return Err(ParseError::UnsupportedSkyLightLutTableSet {
                expected: SKY_LIGHT_LUT_TABLE_SET,
                found: table_set,
            });
        }

        if bytes.len() != SKY_LIGHT_LUT_SIZE {
            return Err(ParseError::InvalidSkyLightLutSize {
                expected: SKY_LIGHT_LUT_SIZE,
                actual: bytes.len(),
            });
        }

        let optical_start = SKY_LIGHT_LUT_HEADER_SIZE;
        let transmittance_start = optical_start + OPTICAL_DEPTH_TABLE_SIZE;
        Ok(Self {
            version,
            table_set,
            optical_depth: &bytes[optical_start..transmittance_start],
            transmittance: &bytes[transmittance_start..],
        })
    }

    #[inline]
    #[must_use]
    pub const fn version(self) -> u16 {
        self.version
    }

    #[inline]
    #[must_use]
    pub const fn table_set(self) -> u16 {
        self.table_set
    }

    #[inline]
    #[must_use]
    pub const fn optical_depth_bytes(self) -> &'a [u8] {
        self.optical_depth
    }

    #[inline]
    #[must_use]
    pub const fn transmittance_bytes(self) -> &'a [u8] {
        self.transmittance
    }

    #[inline]
    #[must_use]
    pub const fn optical_depth_len(self) -> usize {
        OPTICAL_DEPTH_SAMPLE_COUNT
    }

    #[inline]
    #[must_use]
    pub const fn transmittance_len(self) -> usize {
        TRANSMITTANCE_SAMPLE_COUNT
    }

    #[inline]
    #[must_use]
    pub const fn optical_depth_samples(self) -> OpticalDepthSamples<'a> {
        OpticalDepthSamples {
            bytes: self.optical_depth,
            index: 0,
        }
    }

    #[inline]
    #[must_use]
    pub const fn transmittance_samples(self) -> TransmittanceSamples<'a> {
        TransmittanceSamples {
            bytes: self.transmittance,
            index: 0,
        }
    }

    #[must_use]
    pub fn optical_depth_at(
        self,
        altitude_index: usize,
        view_index: usize,
    ) -> Option<OpticalDepthSample> {
        if altitude_index >= OPTICAL_DEPTH_ALTITUDE_SAMPLES
            || view_index >= OPTICAL_DEPTH_VIEW_SAMPLES
        {
            return None;
        }
        let index = altitude_index * OPTICAL_DEPTH_VIEW_SAMPLES + view_index;
        Some(read_optical_depth_sample(
            self.optical_depth,
            index * OPTICAL_DEPTH_SAMPLE_SIZE,
        ))
    }

    #[inline]
    #[must_use]
    pub fn summary(self) -> SkyLightOpticalLutSummary {
        SkyLightOpticalLutSummary {
            version: self.version(),
            table_set: self.table_set(),
            optical_depth_grid: (OPTICAL_DEPTH_ALTITUDE_SAMPLES, OPTICAL_DEPTH_VIEW_SAMPLES),
            optical_depth_samples: self.optical_depth_len(),
            transmittance_samples: self.transmittance_len(),
            first_optical_depth: self.optical_depth_at(0, 0),
            last_transmittance: self.transmittance_samples().last(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyLightOpticalLutSummary {
    pub version: u16,
    pub table_set: u16,
    pub optical_depth_grid: (usize, usize),
    pub optical_depth_samples: usize,
    pub transmittance_samples: usize,
    pub first_optical_depth: Option<OpticalDepthSample>,
    pub last_transmittance: Option<TransmittanceSample>,
}

impl fmt::Display for SkyLightOpticalLutSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  version:              {}", self.version)?;
        writeln!(f, "  table set:            {}", self.table_set)?;
        writeln!(
            f,
            "  optical depth grid:   {}x{}",
            self.optical_depth_grid.0, self.optical_depth_grid.1
        )?;
        writeln!(f, "  optical depth samples: {}", self.optical_depth_samples)?;
        write!(f, "  transmittance samples: {}", self.transmittance_samples)?;
        if let Some(first) = self.first_optical_depth {
            write!(
                f,
                "\n  first optical depth:  rayleigh={} mie={}",
                first.rayleigh, first.mie
            )?;
        }
        if let Some(last) = self.last_transmittance {
            write!(
                f,
                "\n  last transmittance:   height={} rayleigh={} mie={}",
                last.height, last.rayleigh, last.mie
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkyLightOpticalLutInspectionReport<'a> {
    pub path: &'a Path,
    pub summary: SkyLightOpticalLutSummary,
}

impl fmt::Display for SkyLightOpticalLutInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.path.display())?;
        write!(f, "{}", self.summary)
    }
}

/// Summarises a sky-light optical LUT's dimensions and table extents.
///
/// # Errors
///
/// Returns any error [`SkyLightOpticalLut::parse`] returns —
/// [`ParseError::UnexpectedEof`] for a truncated file,
/// [`ParseError::InvalidMagic`] for a signature other than `SKYL`,
/// [`ParseError::UnsupportedVersion`], or
/// [`ParseError::UnsupportedSkyLightLutTableSet`].
pub fn summarize_sky_light_lut(bytes: &[u8]) -> Result<SkyLightOpticalLutSummary, ParseError> {
    SkyLightOpticalLut::parse(bytes).map(SkyLightOpticalLut::summary)
}

/// Summarises a sky-light optical LUT and pairs it with `path` for display.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_sky_light_lut`] returns —
/// [`ParseError::UnexpectedEof`], [`ParseError::InvalidMagic`],
/// [`ParseError::UnsupportedVersion`] or
/// [`ParseError::UnsupportedSkyLightLutTableSet`].
pub fn inspect_sky_light_lut<'a>(
    path: &'a Path,
    bytes: &[u8],
) -> Result<SkyLightOpticalLutInspectionReport<'a>, ParseError> {
    summarize_sky_light_lut(bytes)
        .map(|summary| SkyLightOpticalLutInspectionReport { path, summary })
}

#[derive(Debug, Error)]
pub enum SkyLightLutInspectionError {
    #[error("read sky light LUT {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse sky light LUT {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Reads a sky-light optical LUT from disk and summarises it.
///
/// # Errors
///
/// Returns [`SkyLightLutInspectionError::Read`] if `path` cannot be read
/// (missing file, permissions), or [`SkyLightLutInspectionError::Parse`]
/// wrapping the [`ParseError`] from a malformed LUT. Both variants carry the
/// offending path.
pub fn inspect_sky_light_lut_path(
    path: &Path,
) -> Result<SkyLightOpticalLutInspectionReport<'_>, SkyLightLutInspectionError> {
    let bytes = std::fs::read(path).map_err(|source| SkyLightLutInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_sky_light_lut(path, &bytes).map_err(|source| SkyLightLutInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpticalDepthSample {
    pub rayleigh: f32,
    pub mie: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransmittanceSample {
    pub height: f32,
    pub rayleigh: f32,
    pub mie: f32,
}

// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct OpticalDepthSamples<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Iterator for OpticalDepthSamples<'_> {
    type Item = OpticalDepthSample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index == OPTICAL_DEPTH_SAMPLE_COUNT {
            return None;
        }
        let offset = self.index * OPTICAL_DEPTH_SAMPLE_SIZE;
        self.index += 1;
        Some(read_optical_depth_sample(self.bytes, offset))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = OPTICAL_DEPTH_SAMPLE_COUNT - self.index;
        (len, Some(len))
    }
}

impl ExactSizeIterator for OpticalDepthSamples<'_> {}

// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct TransmittanceSamples<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Iterator for TransmittanceSamples<'_> {
    type Item = TransmittanceSample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index == TRANSMITTANCE_SAMPLE_COUNT {
            return None;
        }
        let offset = self.index * TRANSMITTANCE_SAMPLE_SIZE;
        self.index += 1;
        Some(TransmittanceSample {
            height: read_f32_le(self.bytes, offset),
            rayleigh: read_f32_le(self.bytes, offset + 4),
            mie: read_f32_le(self.bytes, offset + 8),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = TRANSMITTANCE_SAMPLE_COUNT - self.index;
        (len, Some(len))
    }
}

impl ExactSizeIterator for TransmittanceSamples<'_> {}

fn read_optical_depth_sample(bytes: &[u8], offset: usize) -> OpticalDepthSample {
    OpticalDepthSample {
        rayleigh: read_f32_le(bytes, offset),
        mie: read_f32_le(bytes, offset + 4),
    }
}

fn read_f32_le(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated sky-light LUT sample width"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sky_light_lut_layout() {
        let mut bytes = vec![0; SKY_LIGHT_LUT_SIZE];
        bytes[..4].copy_from_slice(SKY_LIGHT_LUT_MAGIC);
        bytes[4..6].copy_from_slice(&SKY_LIGHT_LUT_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&SKY_LIGHT_LUT_TABLE_SET.to_le_bytes());
        bytes[8..12].copy_from_slice(&1.25f32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2.5f32.to_le_bytes());
        let transmittance_offset = SKY_LIGHT_LUT_HEADER_SIZE + OPTICAL_DEPTH_TABLE_SIZE;
        bytes[transmittance_offset..transmittance_offset + 4]
            .copy_from_slice(&10.0f32.to_le_bytes());
        bytes[transmittance_offset + 4..transmittance_offset + 8]
            .copy_from_slice(&0.75f32.to_le_bytes());
        bytes[transmittance_offset + 8..transmittance_offset + 12]
            .copy_from_slice(&0.5f32.to_le_bytes());

        let lut = SkyLightOpticalLut::parse(&bytes).unwrap();

        assert_eq!(lut.version(), SKY_LIGHT_LUT_VERSION);
        assert_eq!(lut.table_set(), SKY_LIGHT_LUT_TABLE_SET);
        assert_eq!(lut.optical_depth_len(), 8192);
        assert_eq!(lut.transmittance_len(), 32);
        assert_eq!(
            lut.optical_depth_at(0, 0),
            Some(OpticalDepthSample {
                rayleigh: 1.25,
                mie: 2.5,
            })
        );
        assert_eq!(
            lut.transmittance_samples().next(),
            Some(TransmittanceSample {
                height: 10.0,
                rayleigh: 0.75,
                mie: 0.5,
            })
        );
        assert_eq!(
            lut.summary(),
            SkyLightOpticalLutSummary {
                version: SKY_LIGHT_LUT_VERSION,
                table_set: SKY_LIGHT_LUT_TABLE_SET,
                optical_depth_grid: (OPTICAL_DEPTH_ALTITUDE_SAMPLES, OPTICAL_DEPTH_VIEW_SAMPLES),
                optical_depth_samples: OPTICAL_DEPTH_SAMPLE_COUNT,
                transmittance_samples: TRANSMITTANCE_SAMPLE_COUNT,
                first_optical_depth: Some(OpticalDepthSample {
                    rayleigh: 1.25,
                    mie: 2.5,
                }),
                last_transmittance: Some(TransmittanceSample {
                    height: 0.0,
                    rayleigh: 0.0,
                    mie: 0.0,
                }),
            }
        );
        assert_eq!(
            lut.summary().to_string(),
            "  version:              2\n  table set:            1\n  optical depth grid:   32x256\n  optical depth samples: 8192\n  transmittance samples: 32\n  first optical depth:  rayleigh=1.25 mie=2.5\n  last transmittance:   height=0 rayleigh=0 mie=0"
        );
        assert_eq!(
            inspect_sky_light_lut(Path::new("engineassets/sky/optical.lut"), &bytes)
                .unwrap()
                .to_string(),
            "engineassets/sky/optical.lut\n  version:              2\n  table set:            1\n  optical depth grid:   32x256\n  optical depth samples: 8192\n  transmittance samples: 32\n  first optical depth:  rayleigh=1.25 mie=2.5\n  last transmittance:   height=0 rayleigh=0 mie=0"
        );
    }

    #[test]
    fn rejects_bad_lut_size() {
        let mut bytes = vec![0; SKY_LIGHT_LUT_SIZE - 1];
        bytes[..4].copy_from_slice(SKY_LIGHT_LUT_MAGIC);
        bytes[4..6].copy_from_slice(&SKY_LIGHT_LUT_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&SKY_LIGHT_LUT_TABLE_SET.to_le_bytes());

        let err = SkyLightOpticalLut::parse(&bytes).unwrap_err();

        assert!(matches!(err, ParseError::InvalidSkyLightLutSize { .. }));
    }
}
