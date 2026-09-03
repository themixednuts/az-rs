use std::{
    mem,
    num::{ParseFloatError, ParseIntError},
    str,
};

use az_asset_builder::normalize_source_path;
use quick_xml::{
    Reader,
    events::{BytesStart, Event, attributes::AttrError},
};
use serde::{Deserialize, Serialize};

use crate::{XmlAssetKind, xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::{XmlAttribute, XmlSourceTransformError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDayProfileSource {
    pub source_path: String,
    pub time: f32,
    pub start_time: f32,
    pub end_time: f32,
    pub animation_speed: f32,
    pub variables: Vec<TimeOfDayVariableSource>,
    pub comments: Vec<String>,
}

impl TimeOfDayProfileSource {
    /// Parse a legacy time-of-day profile payload.
    ///
    /// # Errors
    ///
    /// Returns [`XmlSourceTransformError::UnsupportedPath`] when `source_path`
    /// is not a time-of-day asset, and the wrapped
    /// [`TimeOfDayParseError`] for non-UTF-8 bytes, malformed markup, or an
    /// unparsable attribute value.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, XmlSourceTransformError> {
        let normalized_source_path = normalize_source_path(source_path);
        if !matches!(
            XmlAssetKind::from_path(&normalized_source_path),
            XmlAssetKind::TimeOfDay
        ) {
            return Err(XmlSourceTransformError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let xml = str::from_utf8(bytes).map_err(TimeOfDayParseError::InvalidUtf8)?;
        Ok(TimeOfDayParser::new(normalized_source_path).parse(xml)?)
    }

    /// Serialize this source projection to pretty RON bytes.
    ///
    /// # Errors
    ///
    /// Returns any [`ron::Error`] the RON serializer reports for this value.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        super::to_ron_bytes(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDayVariableSource {
    pub name: String,
    pub value: TimeOfDayValueSource,
    pub spline: TimeOfDaySplineSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimeOfDayValueSource {
    Float(TimeOfDayFloatValueSource),
    Color(TimeOfDayColorValueSource),
}

impl TimeOfDayValueSource {
    const fn kind(self) -> TimeOfDayValueKind {
        match self {
            Self::Float(_) => TimeOfDayValueKind::Float,
            Self::Color(_) => TimeOfDayValueKind::Color,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDayFloatValueSource {
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDayColorValueSource {
    pub value: ColorRgbSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorRgbSource {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDaySplineSource {
    pub keys: Vec<TimeOfDaySplineKeySource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeOfDaySplineKeySource {
    pub time: f32,
    pub value: TimeOfDayValueSource,
    pub flags: SplineKeyFlagsSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplineKeyFlagsSource {
    pub in_tangent: SplineTangentSource,
    pub out_tangent: SplineTangentSource,
    pub unified: bool,
    pub selected_dimensions: u8,
    pub unknown_bits: u32,
}

impl SplineKeyFlagsSource {
    const IN_MASK: u32 = 0x07;
    const OUT_MASK: u32 = 0x07 << 3;
    const UNIFIED_MASK: u32 = 0x01 << 6;
    const SELECTED_DIMENSIONS_MASK: u32 = 0x0f << 16;
    const KNOWN_MASK: u32 =
        Self::IN_MASK | Self::OUT_MASK | Self::UNIFIED_MASK | Self::SELECTED_DIMENSIONS_MASK;

    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            in_tangent: SplineTangentSource::from_raw((raw & Self::IN_MASK) as u8),
            out_tangent: SplineTangentSource::from_raw(((raw & Self::OUT_MASK) >> 3) as u8),
            unified: raw & Self::UNIFIED_MASK != 0,
            selected_dimensions: ((raw & Self::SELECTED_DIMENSIONS_MASK) >> 16) as u8,
            unknown_bits: raw & !Self::KNOWN_MASK,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplineTangentSource {
    None,
    Custom,
    Zero,
    Step,
    Linear,
    Bezier,
    Unknown(SplineTangentUnknownSource),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplineTangentUnknownSource {
    pub raw: u8,
}

impl SplineTangentSource {
    const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::None,
            1 => Self::Custom,
            2 => Self::Zero,
            3 => Self::Step,
            4 => Self::Linear,
            5 => Self::Bezier,
            _ => Self::Unknown(SplineTangentUnknownSource { raw }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeOfDayValueKind {
    Float,
    Color,
}

struct CurrentVariable {
    name: String,
    value: TimeOfDayValueSource,
    spline: Option<TimeOfDaySplineSource>,
}

struct TimeOfDayParser {
    source_path: String,
    source: Option<TimeOfDayProfileSource>,
    current_variable: Option<CurrentVariable>,
    spline_open: bool,
    root_closed: bool,
    pending_comments: Vec<String>,
}

impl TimeOfDayParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            source: None,
            current_variable: None,
            spline_open: false,
            root_closed: false,
            pending_comments: Vec::new(),
        }
    }

    fn parse(mut self, xml: &str) -> Result<TimeOfDayProfileSource, TimeOfDayParseError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        loop {
            match reader.read_event()? {
                Event::Start(event) => self.start_element(&reader, &event)?,
                Event::Empty(event) => self.empty_element(&reader, &event)?,
                Event::End(event) => {
                    let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                    self.end_element(&name)?;
                }
                Event::Text(event) => {
                    let text = xml_text_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(TimeOfDayParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(TimeOfDayParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::Comment(event) => {
                    let comment = String::from_utf8_lossy(event.as_ref()).trim().to_string();
                    if !comment.is_empty() {
                        self.add_comment(comment)?;
                    }
                }
                Event::GeneralRef(event) => {
                    let text = xml_general_reference_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(TimeOfDayParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if self.spline_open {
            return Err(TimeOfDayParseError::UnclosedElement {
                element: "Spline".to_string(),
            });
        }
        if self.current_variable.is_some() {
            return Err(TimeOfDayParseError::UnclosedElement {
                element: "Variable".to_string(),
            });
        }

        self.source.ok_or(TimeOfDayParseError::MissingElement {
            element: "TimeOfDay",
        })
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), TimeOfDayParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;
        match name.as_str() {
            "TimeOfDay" => self.start_time_of_day(attributes),
            "Variable" => self.start_variable(attributes),
            "Spline" => self.start_spline(attributes),
            _ => Err(TimeOfDayParseError::UnknownElement { element: name }),
        }
    }

    fn empty_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), TimeOfDayParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;
        match name.as_str() {
            "TimeOfDay" => {
                self.start_time_of_day(attributes)?;
                self.end_element("TimeOfDay")
            }
            "Variable" => {
                self.start_variable(attributes)?;
                self.end_element("Variable")
            }
            "Spline" => self.store_spline(attributes),
            _ => Err(TimeOfDayParseError::UnknownElement { element: name }),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), TimeOfDayParseError> {
        match name {
            "TimeOfDay" => {
                if self.spline_open {
                    return Err(TimeOfDayParseError::UnclosedElement {
                        element: "Spline".to_string(),
                    });
                }
                if self.current_variable.is_some() {
                    return Err(TimeOfDayParseError::UnclosedElement {
                        element: "Variable".to_string(),
                    });
                }
                self.root_closed = true;
                Ok(())
            }
            "Variable" => self.end_variable(),
            "Spline" => {
                if !self.spline_open {
                    return Err(TimeOfDayParseError::ElementInWrongParent {
                        element: "Spline",
                        parent: "Variable",
                    });
                }
                self.spline_open = false;
                Ok(())
            }
            _ => Err(TimeOfDayParseError::UnknownElement {
                element: name.to_string(),
            }),
        }
    }

    fn start_time_of_day(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), TimeOfDayParseError> {
        if self.source.is_some() {
            return Err(TimeOfDayParseError::DuplicateElement {
                element: "TimeOfDay",
            });
        }

        let mut time = None;
        let mut start_time = None;
        let mut end_time = None;
        let mut animation_speed = None;

        for attribute in attributes {
            match attribute.name.as_str() {
                "Time" => time = Some(parse_f32("TimeOfDay", "Time", &attribute.value)?),
                "TimeStart" => {
                    start_time = Some(parse_f32("TimeOfDay", "TimeStart", &attribute.value)?);
                }
                "TimeEnd" => {
                    end_time = Some(parse_f32("TimeOfDay", "TimeEnd", &attribute.value)?);
                }
                "TimeAnimSpeed" => {
                    animation_speed =
                        Some(parse_f32("TimeOfDay", "TimeAnimSpeed", &attribute.value)?);
                }
                _ => {
                    return Err(TimeOfDayParseError::UnknownAttribute {
                        element: "TimeOfDay",
                        attribute: attribute.name,
                    });
                }
            }
        }

        self.source = Some(TimeOfDayProfileSource {
            source_path: self.source_path.clone(),
            time: time.ok_or(TimeOfDayParseError::MissingAttribute {
                element: "TimeOfDay",
                attribute: "Time",
            })?,
            start_time: start_time.ok_or(TimeOfDayParseError::MissingAttribute {
                element: "TimeOfDay",
                attribute: "TimeStart",
            })?,
            end_time: end_time.ok_or(TimeOfDayParseError::MissingAttribute {
                element: "TimeOfDay",
                attribute: "TimeEnd",
            })?,
            animation_speed: animation_speed.ok_or(TimeOfDayParseError::MissingAttribute {
                element: "TimeOfDay",
                attribute: "TimeAnimSpeed",
            })?,
            variables: Vec::new(),
            comments: mem::take(&mut self.pending_comments),
        });
        Ok(())
    }

    fn start_variable(&mut self, attributes: Vec<XmlAttribute>) -> Result<(), TimeOfDayParseError> {
        self.source_mut()?;
        if self.current_variable.is_some() {
            return Err(TimeOfDayParseError::NestedElement {
                element: "Variable",
            });
        }
        self.current_variable = Some(parse_variable(attributes)?);
        Ok(())
    }

    fn end_variable(&mut self) -> Result<(), TimeOfDayParseError> {
        if self.spline_open {
            return Err(TimeOfDayParseError::UnclosedElement {
                element: "Spline".to_string(),
            });
        }

        let variable =
            self.current_variable
                .take()
                .ok_or(TimeOfDayParseError::ElementInWrongParent {
                    element: "Variable",
                    parent: "TimeOfDay",
                })?;
        let spline = variable
            .spline
            .ok_or_else(|| TimeOfDayParseError::MissingSpline {
                name: variable.name.clone(),
            })?;
        self.source_mut()?.variables.push(TimeOfDayVariableSource {
            name: variable.name,
            value: variable.value,
            spline,
        });
        Ok(())
    }

    fn start_spline(&mut self, attributes: Vec<XmlAttribute>) -> Result<(), TimeOfDayParseError> {
        self.store_spline(attributes)?;
        self.spline_open = true;
        Ok(())
    }

    fn store_spline(&mut self, attributes: Vec<XmlAttribute>) -> Result<(), TimeOfDayParseError> {
        let variable =
            self.current_variable
                .as_mut()
                .ok_or(TimeOfDayParseError::ElementInWrongParent {
                    element: "Spline",
                    parent: "Variable",
                })?;
        if variable.spline.is_some() {
            return Err(TimeOfDayParseError::DuplicateElement { element: "Spline" });
        }
        variable.spline = Some(parse_spline(attributes, variable.value)?);
        Ok(())
    }

    fn add_comment(&mut self, comment: String) -> Result<(), TimeOfDayParseError> {
        if self.root_closed {
            return Err(TimeOfDayParseError::ElementAfterRoot);
        }
        if let Some(source) = self.source.as_mut() {
            source.comments.push(comment);
        } else {
            self.pending_comments.push(comment);
        }
        Ok(())
    }

    fn source_mut(&mut self) -> Result<&mut TimeOfDayProfileSource, TimeOfDayParseError> {
        if self.root_closed {
            return Err(TimeOfDayParseError::ElementAfterRoot);
        }
        self.source
            .as_mut()
            .ok_or(TimeOfDayParseError::MissingElement {
                element: "TimeOfDay",
            })
    }
}

fn parse_variable(attributes: Vec<XmlAttribute>) -> Result<CurrentVariable, TimeOfDayParseError> {
    let mut name = None;
    let mut value = None;

    for attribute in attributes {
        match attribute.name.as_str() {
            "Name" => name = Some(attribute.value),
            "Value" => set_value(
                &mut value,
                TimeOfDayValueSource::Float(TimeOfDayFloatValueSource {
                    value: parse_f32("Variable", "Value", &attribute.value)?,
                }),
            )?,
            "Color" => set_value(
                &mut value,
                TimeOfDayValueSource::Color(TimeOfDayColorValueSource {
                    value: parse_rgb_csv("Variable", "Color", &attribute.value)?,
                }),
            )?,
            _ => {
                return Err(TimeOfDayParseError::UnknownAttribute {
                    element: "Variable",
                    attribute: attribute.name,
                });
            }
        }
    }

    let name = name.ok_or(TimeOfDayParseError::MissingAttribute {
        element: "Variable",
        attribute: "Name",
    })?;
    let value =
        value.ok_or_else(|| TimeOfDayParseError::MissingVariableValue { name: name.clone() })?;
    Ok(CurrentVariable {
        name,
        value,
        spline: None,
    })
}

const fn set_value(
    target: &mut Option<TimeOfDayValueSource>,
    value: TimeOfDayValueSource,
) -> Result<(), TimeOfDayParseError> {
    if target.replace(value).is_some() {
        return Err(TimeOfDayParseError::MultipleVariableValues);
    }
    Ok(())
}

fn parse_spline(
    attributes: Vec<XmlAttribute>,
    value: TimeOfDayValueSource,
) -> Result<TimeOfDaySplineSource, TimeOfDayParseError> {
    let mut keys = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "Keys" => {
                if keys.replace(attribute.value).is_some() {
                    return Err(TimeOfDayParseError::DuplicateAttribute {
                        element: "Spline",
                        attribute: "Keys",
                    });
                }
            }
            _ => {
                return Err(TimeOfDayParseError::UnknownAttribute {
                    element: "Spline",
                    attribute: attribute.name,
                });
            }
        }
    }

    let keys = keys.ok_or(TimeOfDayParseError::MissingAttribute {
        element: "Spline",
        attribute: "Keys",
    })?;
    let key_kind = value.kind();
    let keys = keys
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| parse_spline_key(key, key_kind))
        .collect::<Result<_, _>>()?;
    Ok(TimeOfDaySplineSource { keys })
}

fn parse_spline_key(
    key: &str,
    kind: TimeOfDayValueKind,
) -> Result<TimeOfDaySplineKeySource, TimeOfDayParseError> {
    match kind {
        TimeOfDayValueKind::Float => parse_float_spline_key(key),
        TimeOfDayValueKind::Color => parse_color_spline_key(key),
    }
}

fn parse_float_spline_key(key: &str) -> Result<TimeOfDaySplineKeySource, TimeOfDayParseError> {
    let parts = key.split(':').map(str::trim).collect::<Vec<_>>();
    let [time, value] = parts.as_slice() else {
        let [time, value, flags] = parts.as_slice() else {
            return Err(TimeOfDayParseError::InvalidSplineKey {
                key: key.to_string(),
            });
        };
        return Ok(TimeOfDaySplineKeySource {
            time: parse_f32("Spline", "Keys", time)?,
            value: TimeOfDayValueSource::Float(TimeOfDayFloatValueSource {
                value: parse_f32("Spline", "Keys", value)?,
            }),
            flags: SplineKeyFlagsSource::from_raw(parse_u32("Spline", "Keys", flags)?),
        });
    };

    Ok(TimeOfDaySplineKeySource {
        time: parse_f32("Spline", "Keys", time)?,
        value: TimeOfDayValueSource::Float(TimeOfDayFloatValueSource {
            value: parse_f32("Spline", "Keys", value)?,
        }),
        flags: SplineKeyFlagsSource::from_raw(0),
    })
}

fn parse_color_spline_key(key: &str) -> Result<TimeOfDaySplineKeySource, TimeOfDayParseError> {
    let (time, rest) =
        key.split_once(":(")
            .ok_or_else(|| TimeOfDayParseError::InvalidSplineKey {
                key: key.to_string(),
            })?;
    let (rgb, suffix) =
        rest.split_once(')')
            .ok_or_else(|| TimeOfDayParseError::InvalidSplineKey {
                key: key.to_string(),
            })?;
    let flags = match suffix.trim() {
        "" => 0,
        suffix => {
            let flags =
                suffix
                    .strip_prefix(':')
                    .ok_or_else(|| TimeOfDayParseError::InvalidSplineKey {
                        key: key.to_string(),
                    })?;
            parse_u32("Spline", "Keys", flags)?
        }
    };

    Ok(TimeOfDaySplineKeySource {
        time: parse_f32("Spline", "Keys", time)?,
        value: TimeOfDayValueSource::Color(TimeOfDayColorValueSource {
            value: parse_rgb_colon("Spline", "Keys", rgb)?,
        }),
        flags: SplineKeyFlagsSource::from_raw(flags),
    })
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<XmlAttribute>, TimeOfDayParseError> {
    let mut attributes = Vec::new();
    for attribute in event.attributes() {
        let attribute = attribute?;
        let name = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())?
            .into_owned();
        attributes.push(XmlAttribute { name, value });
    }
    Ok(attributes)
}

fn element_name(event: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(event.name().as_ref()).into_owned()
}

fn parse_f32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<f32, TimeOfDayParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| TimeOfDayParseError::InvalidFloat {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}

fn parse_u32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<u32, TimeOfDayParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| TimeOfDayParseError::InvalidInteger {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}

fn parse_rgb_csv(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<ColorRgbSource, TimeOfDayParseError> {
    let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
    parse_rgb_parts(element, attribute, value, &parts)
}

fn parse_rgb_colon(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<ColorRgbSource, TimeOfDayParseError> {
    let parts = value.split(':').map(str::trim).collect::<Vec<_>>();
    parse_rgb_parts(element, attribute, value, &parts)
}

fn parse_rgb_parts(
    element: &'static str,
    attribute: &'static str,
    raw: &str,
    parts: &[&str],
) -> Result<ColorRgbSource, TimeOfDayParseError> {
    let [r, g, b] = parts else {
        return Err(TimeOfDayParseError::InvalidColor {
            element,
            attribute,
            value: raw.to_string(),
            components: parts.len(),
        });
    };
    Ok(ColorRgbSource {
        r: parse_f32(element, attribute, r)?,
        g: parse_f32(element, attribute, g)?,
        b: parse_f32(element, attribute, b)?,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum TimeOfDayParseError {
    #[error("time-of-day XML is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("missing <{element}> element")]
    MissingElement { element: &'static str },
    #[error("duplicate <{element}> element")]
    DuplicateElement { element: &'static str },
    #[error("element appears after closing </TimeOfDay>")]
    ElementAfterRoot,
    #[error("unexpected <{element}> element")]
    UnknownElement { element: String },
    #[error("unexpected {attribute:?} attribute on <{element}>")]
    UnknownAttribute {
        element: &'static str,
        attribute: String,
    },
    #[error("missing {attribute:?} attribute on <{element}>")]
    MissingAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("duplicate {attribute:?} attribute on <{element}>")]
    DuplicateAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("<{element}> cannot be nested")]
    NestedElement { element: &'static str },
    #[error("<{element}> cannot appear outside <{parent}>")]
    ElementInWrongParent {
        element: &'static str,
        parent: &'static str,
    },
    #[error("XML document ended before closing <{element}>")]
    UnclosedElement { element: String },
    #[error("unexpected text in time-of-day XML: {text:?}")]
    UnexpectedText { text: String },
    #[error("invalid integer {value:?} in <{element}> {attribute}: {source}")]
    InvalidInteger {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseIntError,
    },
    #[error("invalid float {value:?} in <{element}> {attribute}: {source}")]
    InvalidFloat {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseFloatError,
    },
    #[error(
        "invalid three-component color {value:?} in <{element}> {attribute}: got {components} components"
    )]
    InvalidColor {
        element: &'static str,
        attribute: &'static str,
        value: String,
        components: usize,
    },
    #[error("invalid TimeOfDay spline key {key:?}")]
    InvalidSplineKey { key: String },
    #[error("<Variable> has both Value and Color attributes")]
    MultipleVariableValues,
    #[error("<Variable name={name:?}> has no Value or Color attribute")]
    MissingVariableValue { name: String },
    #[error("<Variable name={name:?}> has no <Spline>")]
    MissingSpline { name: String },
}
