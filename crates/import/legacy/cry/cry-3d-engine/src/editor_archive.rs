use std::borrow::Cow;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::ParseError;
use crate::read::{Cursor, Endian};
use crate::terrain::CompiledTerrain;

/// Sandbox `CXmlArchive` payload.
///
/// Follows Lumberyard's `dev/Code/Sandbox/Editor/Util/XmlArchive.cpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorArchive<'a> {
    xml: ArchiveString<'a>,
    named_blocks: Vec<NamedBlock<'a>>,
}

impl<'a> EditorArchive<'a> {
    /// Parse a `CXmlArchive` payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the MFC-style string header, XML string, or
    /// named data block table is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let mut cursor = Cursor::new(bytes);
        let xml = read_archive_string(&mut cursor, "CXmlArchive.xml")?;
        validate_xml(xml.as_str(), "CXmlArchive.xml")?;
        let declared_block_count = cursor.read_i32(Endian::Little)?;
        let block_count =
            usize::try_from(declared_block_count).map_err(|_| ParseError::InvalidCount {
                field: "CNamedData block count",
                count: declared_block_count,
            })?;

        let mut named_blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let name = read_archive_string(&mut cursor, "CNamedData block name")?;
            let size_flags = cursor.read_u32(Endian::Little)?;
            let original_size = cursor.read_u32(Endian::Little)?;
            let flags = cursor.read_u32(Endian::Little)?;
            let size = size_flags & !(1 << 31);
            let compressed = size_flags & (1 << 31) != 0;
            let data = cursor.read_bytes(size as usize)?;
            named_blocks.push(NamedBlock {
                name,
                data,
                original_size,
                flags,
                compressed,
            });
        }
        if cursor.remaining() != 0 {
            return Err(ParseError::ChunkSizeMismatch {
                declared: bytes.len(),
                actual: cursor.position(),
            });
        }

        Ok(Self { xml, named_blocks })
    }

    #[inline]
    #[must_use]
    pub const fn xml(&self) -> &ArchiveString<'a> {
        &self.xml
    }

    #[inline]
    #[must_use]
    pub fn named_blocks(&self) -> &[NamedBlock<'a>] {
        &self.named_blocks
    }

    #[must_use]
    pub fn block(&self, name: &str) -> Option<&NamedBlock<'a>> {
        self.named_blocks
            .iter()
            .find(|block| block.name.as_str().eq_ignore_ascii_case(name))
    }
}

/// MFC-style archived string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveString<'a> {
    value: Cow<'a, str>,
}

impl<'a> ArchiveString<'a> {
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }

    #[inline]
    #[must_use]
    pub fn as_cow(&self) -> Cow<'a, str> {
        self.value.clone()
    }
}

/// One `CNamedData` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedBlock<'a> {
    name: ArchiveString<'a>,
    data: &'a [u8],
    original_size: u32,
    flags: u32,
    compressed: bool,
}

impl<'a> NamedBlock<'a> {
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &ArchiveString<'a> {
        &self.name
    }

    #[inline]
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    #[inline]
    #[must_use]
    pub const fn original_size(&self) -> u32 {
        self.original_size
    }

    #[inline]
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    #[inline]
    #[must_use]
    pub const fn is_compressed(&self) -> bool {
        self.compressed
    }
}

/// `LevelData/Heightmap.dat`.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorHeightmap<'a> {
    archive: EditorArchive<'a>,
    attributes: HeightmapAttributes,
    terrain: CompiledTerrain<'a>,
}

impl<'a> EditorHeightmap<'a> {
    /// Parse a heightmap editor archive.
    ///
    /// # Errors
    ///
    /// Returns an error when the archive, XML attributes, or nested compiled
    /// terrain block is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let archive = EditorArchive::parse(bytes)?;
        let attributes = read_heightmap_attributes(archive.xml.as_str())?;
        let terrain_block =
            archive
                .block("TerrainCompiledData")
                .ok_or(ParseError::MissingNamedBlock {
                    name: "TerrainCompiledData",
                })?;
        let terrain = CompiledTerrain::parse(terrain_block.data())?;
        validate_heightmap_blocks(&archive, attributes)?;
        Ok(Self {
            archive,
            attributes,
            terrain,
        })
    }

    #[inline]
    #[must_use]
    pub const fn archive(&self) -> &EditorArchive<'a> {
        &self.archive
    }

    #[inline]
    #[must_use]
    pub const fn attributes(&self) -> HeightmapAttributes {
        self.attributes
    }

    #[inline]
    #[must_use]
    pub const fn terrain(&self) -> &CompiledTerrain<'a> {
        &self.terrain
    }
}

/// `LevelData/VegetationMap.dat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorVegetationMap<'a> {
    archive: EditorArchive<'a>,
    version: i32,
}

impl<'a> EditorVegetationMap<'a> {
    /// Parse a vegetation-map editor archive.
    ///
    /// # Errors
    ///
    /// Returns an error when the archive or XML root is invalid.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let archive = EditorArchive::parse(bytes)?;
        let version = read_vegetation_map_version(archive.xml.as_str())?;
        Ok(Self { archive, version })
    }

    #[inline]
    #[must_use]
    pub const fn archive(&self) -> &EditorArchive<'a> {
        &self.archive
    }

    #[inline]
    #[must_use]
    pub const fn version(&self) -> i32 {
        self.version
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeightmapAttributes {
    pub width: u32,
    pub height: u32,
    pub water_level: f32,
    pub unit_size: u32,
    pub texture_size: u32,
    pub max_height: f32,
}

fn read_archive_string<'a>(
    cursor: &mut Cursor<'a>,
    field: &'static str,
) -> Result<ArchiveString<'a>, ParseError> {
    let mut char_size = 1usize;
    let mut len = u64::from(cursor.read_u8()?);
    if len == 0xff {
        let raw = cursor.read_bytes(2)?;
        let mut len16 = u16::from_le_bytes([raw[0], raw[1]]);
        if len16 == 0xfffe {
            char_size = 2;
            len = u64::from(cursor.read_u8()?);
            if len == 0xff {
                let raw = cursor.read_bytes(2)?;
                len16 = u16::from_le_bytes([raw[0], raw[1]]);
            }
        }
        if len == 0xff {
            if len16 < 0xffff {
                len = u64::from(len16);
            } else {
                len = u64::from(cursor.read_u32(Endian::Little)?);
                if len == 0xffff_ffff {
                    let raw = cursor.read_bytes(8)?;
                    len = u64::from_le_bytes([
                        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
                    ]);
                }
            }
        }
    }

    let byte_len = usize::try_from(len)
        .ok()
        .and_then(|len| len.checked_mul(char_size))
        .ok_or(ParseError::IntegerOverflow)?;
    let bytes = cursor.read_bytes(byte_len)?;
    if char_size == 1 {
        let text =
            std::str::from_utf8(bytes).map_err(|source| ParseError::Utf8 { field, source })?;
        Ok(ArchiveString {
            value: Cow::Borrowed(text),
        })
    } else {
        let mut words = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            words.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let text =
            String::from_utf16(&words).map_err(|source| ParseError::Utf16 { field, source })?;
        Ok(ArchiveString {
            value: Cow::Owned(text),
        })
    }
}

fn validate_xml(xml: &str, field: &'static str) -> Result<(), ParseError> {
    let mut reader = Reader::from_str(xml);
    loop {
        if reader
            .read_event()
            .map_err(|source| ParseError::Xml { field, source })?
            == Event::Eof
        {
            return Ok(());
        }
    }
}

fn read_heightmap_attributes(xml: &str) -> Result<HeightmapAttributes, ParseError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(|source| ParseError::Xml {
            field: "Heightmap XML",
            source,
        })? {
            Event::Start(event) if event.name().as_ref() == b"Heightmap" => {
                return parse_heightmap_start(&reader, &event);
            }
            Event::Eof => {
                return Err(ParseError::InvalidMagic {
                    asset: "Heightmap.dat XML",
                    expected: b"Heightmap",
                    found: Vec::new(),
                });
            }
            _ => {}
        }
    }
}

fn read_vegetation_map_version(xml: &str) -> Result<i32, ParseError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(|source| ParseError::Xml {
            field: "VegetationMap XML",
            source,
        })? {
            Event::Start(event) if event.name().as_ref() == b"VegetationMap" => {
                let value = required_attr(&reader, &event, b"Version", "Version")?;
                return value.parse().map_err(|_| ParseError::XmlAttribute {
                    field: "VegetationMap XML",
                    name: "Version",
                });
            }
            Event::Eof => {
                return Err(ParseError::InvalidMagic {
                    asset: "VegetationMap.dat XML",
                    expected: b"VegetationMap",
                    found: Vec::new(),
                });
            }
            _ => {}
        }
    }
}

fn parse_heightmap_start(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<HeightmapAttributes, ParseError> {
    Ok(HeightmapAttributes {
        width: parse_attr(reader, event, b"Width", "Width")?,
        height: parse_attr(reader, event, b"Height", "Height")?,
        water_level: parse_attr(reader, event, b"WaterLevel", "WaterLevel")?,
        unit_size: parse_attr(reader, event, b"UnitSize", "UnitSize")?,
        texture_size: parse_attr(reader, event, b"TextureSize", "TextureSize")?,
        max_height: parse_attr(reader, event, b"MaxHeight", "MaxHeight")?,
    })
}

fn validate_heightmap_blocks(
    archive: &EditorArchive<'_>,
    attributes: HeightmapAttributes,
) -> Result<(), ParseError> {
    let pixels = (attributes.width as usize)
        .checked_mul(attributes.height as usize)
        .ok_or(ParseError::IntegerOverflow)?;
    if let Some(block) = archive.block("HeightmapDataW") {
        let expected = pixels
            .checked_mul(size_of::<u16>())
            .ok_or(ParseError::IntegerOverflow)?;
        if block.data().len() != expected {
            return Err(ParseError::ChunkSizeMismatch {
                declared: expected,
                actual: block.data().len(),
            });
        }
    }
    if let Some(block) = archive.block("WeightmapData") {
        let expected = pixels.checked_mul(6).ok_or(ParseError::IntegerOverflow)?;
        if block.data().len() != expected {
            return Err(ParseError::ChunkSizeMismatch {
                declared: expected,
                actual: block.data().len(),
            });
        }
    }
    Ok(())
}

fn parse_attr<T: std::str::FromStr>(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<T, ParseError> {
    let value = required_attr(reader, event, key, name)?;
    value.parse().map_err(|_| ParseError::XmlAttribute {
        field: "Heightmap XML",
        name,
    })
}

fn required_attr(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<String, ParseError> {
    for attr in event.attributes() {
        let attr = attr.map_err(|source| ParseError::Xml {
            field: "XML attribute",
            source: source.into(),
        })?;
        if attr.key.as_ref() == key {
            return attr
                .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())
                .map(std::borrow::Cow::into_owned)
                .map_err(|source| ParseError::Xml {
                    field: "XML attribute",
                    source,
                });
        }
    }
    Err(ParseError::XmlAttribute {
        field: "XML attribute",
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_archive_string_short_utf8() {
        let bytes = [3, b'a', b'b', b'c'];
        let mut cursor = Cursor::new(&bytes);
        let value = read_archive_string(&mut cursor, "test").unwrap();

        assert_eq!(value.as_str(), "abc");
        assert_eq!(cursor.position(), bytes.len());
    }
}
