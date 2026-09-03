use std::{num::ParseFloatError, num::ParseIntError, str};

use az_asset_builder::normalize_source_path;
use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event, attributes::AttrError},
};
use serde::{Deserialize, Serialize};

use crate::{XmlAssetKind, xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::{XmlAttribute, to_ron_bytes};

/// Lossless authoring projection of Cry/Lumberyard `levelinfo.xml`.
///
/// This intentionally retains metadata that is not needed by the runtime
/// level descriptor. Project builders decide which fields become hot runtime
/// products; the compatibility transform does not discard upstream data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelInfoSource {
    pub source_path: String,
    pub sandbox_version: String,
    pub name: String,
    pub heightmap_size: Option<u32>,
    pub terrain: Option<LevelTerrainInfoSource>,
    pub missions: Vec<LevelMissionSource>,
    pub comments: Vec<String>,
}

impl LevelInfoSource {
    /// Parse a legacy `levelinfo.xml` payload.
    ///
    /// # Errors
    ///
    /// Returns [`LevelInfoParseError::UnsupportedPath`] when `source_path` is
    /// not a `levelinfo.xml` asset, [`LevelInfoParseError::InvalidUtf8`] when
    /// `bytes` are not UTF-8, and any parser error the XML reader reports for
    /// malformed markup or an unparsable attribute value.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, LevelInfoParseError> {
        let normalized_source_path = normalize_source_path(source_path);
        if !matches!(
            XmlAssetKind::from_path(&normalized_source_path),
            XmlAssetKind::LevelInfo
        ) {
            return Err(LevelInfoParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let xml = str::from_utf8(bytes)?;
        LevelInfoParser::new(normalized_source_path).parse(xml)
    }

    /// Serialize this source projection to pretty RON bytes.
    ///
    /// # Errors
    ///
    /// Returns any [`ron::Error`] the RON serializer reports for this value.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        to_ron_bytes(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LevelTerrainInfoSource {
    pub heightmap_size: u32,
    pub unit_size: u32,
    pub sector_size: u32,
    pub sectors_table_size: u32,
    pub heightmap_z_ratio: f32,
    pub ocean_water_level: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelMissionSource {
    pub name: String,
    pub description: String,
}

struct LevelInfoParser {
    source_path: String,
    xml_version: XmlVersion,
    source: Option<LevelInfoSource>,
    open_elements: Vec<String>,
    root_closed: bool,
    comments: Vec<String>,
}

impl LevelInfoParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            xml_version: XmlVersion::Implicit1_0,
            source: None,
            open_elements: Vec::new(),
            root_closed: false,
            comments: Vec::new(),
        }
    }

    fn parse(mut self, xml: &str) -> Result<LevelInfoSource, LevelInfoParseError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        loop {
            match reader.read_event()? {
                Event::Start(event) => self.start_element(&reader, &event, false)?,
                Event::Empty(event) => self.start_element(&reader, &event, true)?,
                Event::End(event) => {
                    let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                    self.end_element(&name)?;
                }
                Event::Text(event) => {
                    let text = xml_text_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(LevelInfoParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(LevelInfoParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::Comment(event) => {
                    let comment = String::from_utf8_lossy(event.as_ref()).trim().to_string();
                    if !comment.is_empty() {
                        self.comments.push(comment);
                    }
                }
                Event::GeneralRef(event) => {
                    let text = xml_general_reference_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(LevelInfoParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::Decl(event) => self.xml_version = event.xml_version()?,
                Event::PI(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if let Some(element) = self.open_elements.last() {
            return Err(LevelInfoParseError::UnclosedElement {
                element: element.clone(),
            });
        }
        if !self.root_closed {
            return Err(LevelInfoParseError::MissingElement {
                element: "LevelInfo",
            });
        }
        let mut source = self.source.ok_or(LevelInfoParseError::MissingElement {
            element: "LevelInfo",
        })?;
        source.comments = self.comments;
        Ok(source)
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        empty: bool,
    ) -> Result<(), LevelInfoParseError> {
        let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
        let attributes = attributes(reader, event, self.xml_version)?;
        let parent_path = self.open_elements.join("/");
        let parent = self.open_elements.last().cloned();

        match (parent_path.as_str(), name.as_str()) {
            ("", "LevelInfo") if self.source.is_none() && !self.root_closed => {
                self.source = Some(parse_level_info(self.source_path.clone(), attributes)?);
            }
            ("LevelInfo", "TerrainInfo") => {
                let terrain = parse_terrain_info(attributes)?;
                let source = self.source_mut()?;
                if source.terrain.replace(terrain).is_some() {
                    return Err(LevelInfoParseError::DuplicateElement {
                        element: "TerrainInfo",
                    });
                }
            }
            ("LevelInfo", "Missions") => require_no_attributes("Missions", attributes)?,
            ("LevelInfo/Missions", "Mission") => {
                let mission = parse_mission(attributes)?;
                self.source_mut()?.missions.push(mission);
            }
            _ => {
                return Err(LevelInfoParseError::UnexpectedElement {
                    element: name,
                    parent,
                });
            }
        }

        if empty {
            if name == "LevelInfo" {
                self.root_closed = true;
            }
        } else {
            self.open_elements.push(name);
        }
        Ok(())
    }

    fn end_element(&mut self, name: &str) -> Result<(), LevelInfoParseError> {
        let open = self
            .open_elements
            .pop()
            .ok_or_else(|| LevelInfoParseError::UnexpectedEnd {
                element: name.to_string(),
            })?;
        if open != name {
            return Err(LevelInfoParseError::MismatchedEnd {
                expected: open,
                actual: name.to_string(),
            });
        }
        if name == "LevelInfo" {
            self.root_closed = true;
        }
        Ok(())
    }

    fn source_mut(&mut self) -> Result<&mut LevelInfoSource, LevelInfoParseError> {
        if self.root_closed {
            return Err(LevelInfoParseError::ElementAfterRoot);
        }
        self.source
            .as_mut()
            .ok_or(LevelInfoParseError::MissingElement {
                element: "LevelInfo",
            })
    }
}

fn parse_level_info(
    source_path: String,
    attributes: Vec<XmlAttribute>,
) -> Result<LevelInfoSource, LevelInfoParseError> {
    let mut sandbox_version = None;
    let mut name = None;
    let mut heightmap_size = None;

    for attribute in attributes {
        match attribute.name.as_str() {
            "SandboxVersion" => set_once(
                &mut sandbox_version,
                attribute.value,
                "LevelInfo",
                "SandboxVersion",
            )?,
            "Name" => set_once(&mut name, attribute.value, "LevelInfo", "Name")?,
            "HeightmapSize" => set_once(
                &mut heightmap_size,
                parse_u32("LevelInfo", "HeightmapSize", &attribute.value)?,
                "LevelInfo",
                "HeightmapSize",
            )?,
            _ => {
                return Err(LevelInfoParseError::UnknownAttribute {
                    element: "LevelInfo",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(LevelInfoSource {
        source_path,
        sandbox_version: sandbox_version.ok_or(LevelInfoParseError::MissingAttribute {
            element: "LevelInfo",
            attribute: "SandboxVersion",
        })?,
        name: name.ok_or(LevelInfoParseError::MissingAttribute {
            element: "LevelInfo",
            attribute: "Name",
        })?,
        heightmap_size,
        terrain: None,
        missions: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_terrain_info(
    attributes: Vec<XmlAttribute>,
) -> Result<LevelTerrainInfoSource, LevelInfoParseError> {
    let mut heightmap_size = None;
    let mut unit_size = None;
    let mut sector_size = None;
    let mut sectors_table_size = None;
    let mut heightmap_z_ratio = None;
    let mut ocean_water_level = None;

    for attribute in attributes {
        match attribute.name.as_str() {
            "HeightmapSize" => set_once(
                &mut heightmap_size,
                parse_u32("TerrainInfo", "HeightmapSize", &attribute.value)?,
                "TerrainInfo",
                "HeightmapSize",
            )?,
            "UnitSize" => set_once(
                &mut unit_size,
                parse_u32("TerrainInfo", "UnitSize", &attribute.value)?,
                "TerrainInfo",
                "UnitSize",
            )?,
            "SectorSize" => set_once(
                &mut sector_size,
                parse_u32("TerrainInfo", "SectorSize", &attribute.value)?,
                "TerrainInfo",
                "SectorSize",
            )?,
            "SectorsTableSize" => set_once(
                &mut sectors_table_size,
                parse_u32("TerrainInfo", "SectorsTableSize", &attribute.value)?,
                "TerrainInfo",
                "SectorsTableSize",
            )?,
            "HeightmapZRatio" => set_once(
                &mut heightmap_z_ratio,
                parse_f32("TerrainInfo", "HeightmapZRatio", &attribute.value)?,
                "TerrainInfo",
                "HeightmapZRatio",
            )?,
            "OceanWaterLevel" => set_once(
                &mut ocean_water_level,
                parse_f32("TerrainInfo", "OceanWaterLevel", &attribute.value)?,
                "TerrainInfo",
                "OceanWaterLevel",
            )?,
            _ => {
                return Err(LevelInfoParseError::UnknownAttribute {
                    element: "TerrainInfo",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(LevelTerrainInfoSource {
        heightmap_size: required(heightmap_size, "TerrainInfo", "HeightmapSize")?,
        unit_size: required(unit_size, "TerrainInfo", "UnitSize")?,
        sector_size: required(sector_size, "TerrainInfo", "SectorSize")?,
        sectors_table_size: required(sectors_table_size, "TerrainInfo", "SectorsTableSize")?,
        heightmap_z_ratio: required(heightmap_z_ratio, "TerrainInfo", "HeightmapZRatio")?,
        ocean_water_level: required(ocean_water_level, "TerrainInfo", "OceanWaterLevel")?,
    })
}

fn parse_mission(attributes: Vec<XmlAttribute>) -> Result<LevelMissionSource, LevelInfoParseError> {
    let mut name = None;
    let mut description = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "Name" => set_once(&mut name, attribute.value, "Mission", "Name")?,
            "Description" => set_once(&mut description, attribute.value, "Mission", "Description")?,
            _ => {
                return Err(LevelInfoParseError::UnknownAttribute {
                    element: "Mission",
                    attribute: attribute.name,
                });
            }
        }
    }
    Ok(LevelMissionSource {
        name: required(name, "Mission", "Name")?,
        description: required(description, "Mission", "Description")?,
    })
}

fn require_no_attributes(
    element: &'static str,
    attributes: Vec<XmlAttribute>,
) -> Result<(), LevelInfoParseError> {
    if let Some(attribute) = attributes.into_iter().next() {
        return Err(LevelInfoParseError::UnknownAttribute {
            element,
            attribute: attribute.name,
        });
    }
    Ok(())
}

fn set_once<T>(
    target: &mut Option<T>,
    value: T,
    element: &'static str,
    attribute: &'static str,
) -> Result<(), LevelInfoParseError> {
    if target.replace(value).is_some() {
        return Err(LevelInfoParseError::DuplicateAttribute { element, attribute });
    }
    Ok(())
}

fn required<T>(
    value: Option<T>,
    element: &'static str,
    attribute: &'static str,
) -> Result<T, LevelInfoParseError> {
    value.ok_or(LevelInfoParseError::MissingAttribute { element, attribute })
}

fn parse_u32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<u32, LevelInfoParseError> {
    value
        .parse()
        .map_err(|source| LevelInfoParseError::InvalidInteger {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}

fn parse_f32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<f32, LevelInfoParseError> {
    value
        .parse()
        .map_err(|source| LevelInfoParseError::InvalidFloat {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    xml_version: XmlVersion,
) -> Result<Vec<XmlAttribute>, LevelInfoParseError> {
    event
        .attributes()
        .with_checks(true)
        .map(|attribute| {
            let attribute = attribute?;
            Ok(XmlAttribute {
                name: String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                value: attribute
                    .decoded_and_normalized_value(xml_version, reader.decoder())?
                    .into_owned(),
            })
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum LevelInfoParseError {
    #[error("unsupported level-info path {path}")]
    UnsupportedPath { path: String },
    #[error("level-info XML is not UTF-8: {0}")]
    InvalidUtf8(#[from] str::Utf8Error),
    #[error("parse level-info XML: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("parse level-info XML attribute: {0}")]
    Attribute(#[from] AttrError),
    #[error("unexpected <{element}> under {parent:?} in level-info XML")]
    UnexpectedElement {
        element: String,
        parent: Option<String>,
    },
    #[error("unexpected </{element}> in level-info XML")]
    UnexpectedEnd { element: String },
    #[error("expected </{expected}> but found </{actual}> in level-info XML")]
    MismatchedEnd { expected: String, actual: String },
    #[error("unexpected non-whitespace text in level-info XML: {text:?}")]
    UnexpectedText { text: String },
    #[error("element <{element}> appears more than once in level-info XML")]
    DuplicateElement { element: &'static str },
    #[error("attribute {attribute} appears more than once on <{element}>")]
    DuplicateAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("unknown attribute {attribute} on <{element}> in level-info XML")]
    UnknownAttribute {
        element: &'static str,
        attribute: String,
    },
    #[error("missing attribute {attribute} on <{element}> in level-info XML")]
    MissingAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("invalid integer {value:?} in <{element}> attribute {attribute}: {source}")]
    InvalidInteger {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseIntError,
    },
    #[error("invalid float {value:?} in <{element}> attribute {attribute}: {source}")]
    InvalidFloat {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseFloatError,
    },
    #[error("missing <{element}> in level-info XML")]
    MissingElement { element: &'static str },
    #[error("unclosed <{element}> in level-info XML")]
    UnclosedElement { element: String },
    #[error("element found after </LevelInfo>")]
    ElementAfterRoot,
}
