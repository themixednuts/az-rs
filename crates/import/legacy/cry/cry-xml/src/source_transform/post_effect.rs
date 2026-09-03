use std::{
    num::{ParseFloatError, ParseIntError},
    str,
};

use az_asset_builder::normalize_source_path;
use quick_xml::{
    Reader,
    errors::IllFormedError,
    events::{BytesStart, Event, attributes::AttrError},
};
use serde::{Deserialize, Serialize};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::{XmlAttribute, to_ron_bytes};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostEffectGroupSource {
    pub source_path: String,
    pub priority: u32,
    pub hold: bool,
    pub fade_distance: Option<f32>,
    pub effects: Vec<PostEffectEffectSource>,
    pub blend_in: Option<PostEffectBlendSource>,
    pub blend_out: Option<PostEffectBlendSource>,
    pub comments: Vec<String>,
}

impl PostEffectGroupSource {
    /// Parse a legacy post-effect group payload.
    ///
    /// # Errors
    ///
    /// Returns [`PostEffectGroupParseError::UnsupportedPath`] when
    /// `source_path` is not a `libs/posteffectgroups/` asset,
    /// [`PostEffectGroupParseError::InvalidUtf8`] when `bytes` are not UTF-8,
    /// and any parser error the XML reader reports for malformed markup or an
    /// unparsable attribute value.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, PostEffectGroupParseError> {
        let normalized_source_path = normalize_source_path(source_path);
        if !is_post_effect_group_xml_path(&normalized_source_path) {
            return Err(PostEffectGroupParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        PostEffectGroupParser::new(normalized_source_path).parse(str::from_utf8(bytes)?)
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostEffectEffectSource {
    pub name: String,
    pub params: Vec<PostEffectParamSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostEffectParamSource {
    pub name: String,
    pub value: PostEffectParamValueSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PostEffectParamValueSource {
    Float(PostEffectFloatParamValueSource),
    Vec4(PostEffectVec4ParamValueSource),
    Color(PostEffectColorParamValueSource),
    String(PostEffectStringParamValueSource),
    Texture(PostEffectTextureParamValueSource),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostEffectFloatParamValueSource {
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostEffectVec4ParamValueSource {
    pub value: Vec4Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostEffectColorParamValueSource {
    pub value: ColorRgbaSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostEffectStringParamValueSource {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostEffectTextureParamValueSource {
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec4Source {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorRgbaSource {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostEffectBlendSource {
    pub curve: PostEffectBlendCurve,
    pub keys: Vec<PostEffectKeySource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostEffectBlendCurve {
    Smooth,
    Linear,
    Step,
    Unknown(PostEffectUnknownBlendCurveSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostEffectUnknownBlendCurveSource {
    pub raw: String,
}

impl PostEffectBlendCurve {
    fn from_legacy(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::Smooth;
        };

        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "smooth" => Self::Smooth,
            "linear" => Self::Linear,
            "step" => Self::Step,
            _ => Self::Unknown(PostEffectUnknownBlendCurveSource {
                raw: raw.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostEffectKeySource {
    pub time: f32,
    pub value: f32,
}
fn is_post_effect_group_xml_path(normalized_source_path: &str) -> bool {
    normalized_source_path.starts_with("libs/posteffectgroups/")
        && crate::has_extension(normalized_source_path, "xml")
}

struct PostEffectGroupParser {
    source_path: String,
    source: Option<PostEffectGroupSource>,
    current_effect: Option<PostEffectEffectSource>,
    current_blend: Option<CurrentBlend>,
    root_closed: bool,
}

impl PostEffectGroupParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            source: None,
            current_effect: None,
            current_blend: None,
            root_closed: false,
        }
    }

    fn parse(mut self, xml: &str) -> Result<PostEffectGroupSource, PostEffectGroupParseError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        reader.config_mut().check_end_names = false;

        loop {
            let event = match reader.read_event() {
                Ok(event) => event,
                Err(quick_xml::Error::IllFormed(IllFormedError::UnmatchedEndTag(name))) => {
                    self.end_element(&name)?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            match event {
                Event::Start(event) => self.start_element(&reader, &event)?,
                Event::Empty(event) => self.empty_element(&reader, &event)?,
                Event::End(event) => {
                    let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                    self.end_element(&name)?;
                }
                Event::Text(event) => {
                    let text = xml_text_content(&event)?;
                    let trimmed = text.trim();
                    // Some shipped post-effect files leave a dangling comment close marker after
                    // an active element, e.g. `</BlendIn>-->`.
                    if !trimmed.is_empty() && trimmed != "-->" {
                        return Err(PostEffectGroupParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?;
                    if !text.trim().is_empty() {
                        return Err(PostEffectGroupParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::Comment(event) => {
                    let comment = String::from_utf8_lossy(event.as_ref()).trim().to_string();
                    if !comment.is_empty() {
                        self.source_mut()?.comments.push(comment);
                    }
                }
                Event::GeneralRef(event) => {
                    let text = xml_general_reference_content(&event)?;
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && trimmed != "-->" {
                        return Err(PostEffectGroupParseError::UnexpectedText {
                            text: text.into_owned(),
                        });
                    }
                }
                Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if self.current_effect.is_some() {
            return Err(PostEffectGroupParseError::UnclosedElement {
                element: "Effect".to_string(),
            });
        }
        if self.current_blend.is_some() {
            return Err(PostEffectGroupParseError::UnclosedElement {
                element: "BlendIn/BlendOut".to_string(),
            });
        }

        self.source.ok_or({
            PostEffectGroupParseError::MissingElement {
                element: "PostEffectGroup",
            }
        })
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), PostEffectGroupParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;
        match name.as_str() {
            "PostEffectGroup" => self.start_group(attributes),
            "Effect" => self.start_effect(attributes),
            "Param" => self.add_param(attributes),
            "BlendIn" => self.start_blend(PostEffectBlendKind::In, attributes),
            "BlendOut" => self.start_blend(PostEffectBlendKind::Out, attributes),
            "Key" => self.add_key(attributes),
            _ => Err(PostEffectGroupParseError::UnknownElement { element: name }),
        }
    }

    fn empty_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), PostEffectGroupParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;
        match name.as_str() {
            "PostEffectGroup" => {
                self.start_group(attributes)?;
                self.end_element("PostEffectGroup")
            }
            "Effect" => {
                let effect = parse_effect(attributes)?;
                self.source_mut()?.effects.push(effect);
                Ok(())
            }
            "Param" => self.add_param(attributes),
            "BlendIn" => {
                let blend = parse_blend(attributes)?;
                self.store_blend(PostEffectBlendKind::In, blend)
            }
            "BlendOut" => {
                let blend = parse_blend(attributes)?;
                self.store_blend(PostEffectBlendKind::Out, blend)
            }
            "Key" => self.add_key(attributes),
            _ => Err(PostEffectGroupParseError::UnknownElement { element: name }),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), PostEffectGroupParseError> {
        match name {
            "PostEffectGroup" => {
                if self.current_effect.is_some() {
                    return Err(PostEffectGroupParseError::UnclosedElement {
                        element: "Effect".to_string(),
                    });
                }
                if self.current_blend.is_some() {
                    return Err(PostEffectGroupParseError::UnclosedElement {
                        element: "BlendIn/BlendOut".to_string(),
                    });
                }
                self.root_closed = true;
                Ok(())
            }
            "Effect" => {
                if let Some(effect) = self.current_effect.take() {
                    self.source_mut()?.effects.push(effect);
                }
                Ok(())
            }
            "BlendIn" | "BlendOut" => {
                if let Some(blend) = self.current_blend.take() {
                    self.store_blend(blend.kind, blend.source)?;
                }
                Ok(())
            }
            "Param" | "Key" => Ok(()),
            _ => Err(PostEffectGroupParseError::UnknownElement {
                element: name.to_string(),
            }),
        }
    }

    fn start_group(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), PostEffectGroupParseError> {
        if self.source.is_some() {
            return Err(PostEffectGroupParseError::DuplicateElement {
                element: "PostEffectGroup",
            });
        }

        let mut priority = None;
        let mut hold = false;
        let mut fade_distance = None;

        for attribute in attributes {
            match attribute.name.as_str() {
                "priority" => {
                    priority = Some(parse_u32("PostEffectGroup", "priority", &attribute.value)?);
                }
                "hold" => hold = parse_bool("PostEffectGroup", "hold", &attribute.value)?,
                "fadeDistance" => {
                    fade_distance = Some(parse_f32(
                        "PostEffectGroup",
                        "fadeDistance",
                        &attribute.value,
                    )?);
                }
                _ => {
                    return Err(PostEffectGroupParseError::UnknownAttribute {
                        element: "PostEffectGroup",
                        attribute: attribute.name,
                    });
                }
            }
        }

        let priority = priority.ok_or(PostEffectGroupParseError::MissingAttribute {
            element: "PostEffectGroup",
            attribute: "priority",
        })?;

        self.source = Some(PostEffectGroupSource {
            source_path: self.source_path.clone(),
            priority,
            hold,
            fade_distance,
            effects: Vec::new(),
            blend_in: None,
            blend_out: None,
            comments: Vec::new(),
        });
        Ok(())
    }

    fn start_effect(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), PostEffectGroupParseError> {
        self.source_mut()?;
        if self.current_effect.is_some() {
            return Err(PostEffectGroupParseError::NestedElement { element: "Effect" });
        }
        if self.current_blend.is_some() {
            return Err(PostEffectGroupParseError::ElementInWrongParent {
                element: "Effect",
                parent: "BlendIn/BlendOut",
            });
        }

        self.current_effect = Some(parse_effect(attributes)?);
        Ok(())
    }

    fn add_param(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), PostEffectGroupParseError> {
        let param = parse_param(attributes)?;
        let effect = self.current_effect.as_mut().ok_or(
            PostEffectGroupParseError::ElementInWrongParent {
                element: "Param",
                parent: "PostEffectGroup",
            },
        )?;
        effect.params.push(param);
        Ok(())
    }

    fn start_blend(
        &mut self,
        kind: PostEffectBlendKind,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), PostEffectGroupParseError> {
        self.source_mut()?;
        if self.current_effect.is_some() {
            return Err(PostEffectGroupParseError::ElementInWrongParent {
                element: kind.element_name(),
                parent: "Effect",
            });
        }
        if self.current_blend.is_some() {
            return Err(PostEffectGroupParseError::NestedElement {
                element: kind.element_name(),
            });
        }

        self.current_blend = Some(CurrentBlend {
            kind,
            source: parse_blend(attributes)?,
        });
        Ok(())
    }

    fn add_key(&mut self, attributes: Vec<XmlAttribute>) -> Result<(), PostEffectGroupParseError> {
        let key = parse_key(attributes)?;
        let blend =
            self.current_blend
                .as_mut()
                .ok_or(PostEffectGroupParseError::ElementInWrongParent {
                    element: "Key",
                    parent: "PostEffectGroup",
                })?;
        blend.source.keys.push(key);
        Ok(())
    }

    fn store_blend(
        &mut self,
        kind: PostEffectBlendKind,
        blend: PostEffectBlendSource,
    ) -> Result<(), PostEffectGroupParseError> {
        let source = self.source_mut()?;
        let slot = match kind {
            PostEffectBlendKind::In => &mut source.blend_in,
            PostEffectBlendKind::Out => &mut source.blend_out,
        };
        if slot.replace(blend).is_some() {
            return Err(PostEffectGroupParseError::DuplicateElement {
                element: kind.element_name(),
            });
        }
        Ok(())
    }

    fn source_mut(&mut self) -> Result<&mut PostEffectGroupSource, PostEffectGroupParseError> {
        if self.root_closed {
            return Err(PostEffectGroupParseError::ElementAfterRoot);
        }
        self.source
            .as_mut()
            .ok_or(PostEffectGroupParseError::MissingElement {
                element: "PostEffectGroup",
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostEffectBlendKind {
    In,
    Out,
}

impl PostEffectBlendKind {
    const fn element_name(self) -> &'static str {
        match self {
            Self::In => "BlendIn",
            Self::Out => "BlendOut",
        }
    }
}

struct CurrentBlend {
    kind: PostEffectBlendKind,
    source: PostEffectBlendSource,
}

fn parse_effect(
    attributes: Vec<XmlAttribute>,
) -> Result<PostEffectEffectSource, PostEffectGroupParseError> {
    let mut name = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            _ => {
                return Err(PostEffectGroupParseError::UnknownAttribute {
                    element: "Effect",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(PostEffectEffectSource {
        name: name.ok_or(PostEffectGroupParseError::MissingAttribute {
            element: "Effect",
            attribute: "name",
        })?,
        params: Vec::new(),
    })
}

fn parse_param(
    attributes: Vec<XmlAttribute>,
) -> Result<PostEffectParamSource, PostEffectGroupParseError> {
    let mut name = None;
    let mut value = None;

    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            "floatValue" => {
                set_param_value(
                    &mut value,
                    PostEffectParamValueSource::Float(PostEffectFloatParamValueSource {
                        value: parse_f32("Param", "floatValue", &attribute.value)?,
                    }),
                )?;
            }
            "vec4Value" => {
                set_param_value(
                    &mut value,
                    PostEffectParamValueSource::Vec4(PostEffectVec4ParamValueSource {
                        value: parse_vec4("Param", "vec4Value", &attribute.value)?,
                    }),
                )?;
            }
            "colorValue" => {
                set_param_value(
                    &mut value,
                    PostEffectParamValueSource::Color(PostEffectColorParamValueSource {
                        value: parse_color("Param", "colorValue", &attribute.value)?,
                    }),
                )?;
            }
            "stringValue" => {
                set_param_value(
                    &mut value,
                    PostEffectParamValueSource::String(PostEffectStringParamValueSource {
                        value: attribute.value,
                    }),
                )?;
            }
            "textureValue" => {
                set_param_value(
                    &mut value,
                    PostEffectParamValueSource::Texture(PostEffectTextureParamValueSource {
                        path: attribute.value,
                    }),
                )?;
            }
            _ => {
                return Err(PostEffectGroupParseError::UnknownAttribute {
                    element: "Param",
                    attribute: attribute.name,
                });
            }
        }
    }

    let name = name.ok_or(PostEffectGroupParseError::MissingAttribute {
        element: "Param",
        attribute: "name",
    })?;
    let value =
        value.ok_or_else(|| PostEffectGroupParseError::MissingParamValue { name: name.clone() })?;
    Ok(PostEffectParamSource { name, value })
}

fn set_param_value(
    target: &mut Option<PostEffectParamValueSource>,
    value: PostEffectParamValueSource,
) -> Result<(), PostEffectGroupParseError> {
    if target.replace(value).is_some() {
        return Err(PostEffectGroupParseError::MultipleParamValues);
    }
    Ok(())
}

fn parse_blend(
    attributes: Vec<XmlAttribute>,
) -> Result<PostEffectBlendSource, PostEffectGroupParseError> {
    let mut curve = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "curve" => curve = Some(attribute.value),
            _ => {
                return Err(PostEffectGroupParseError::UnknownAttribute {
                    element: "BlendIn/BlendOut",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(PostEffectBlendSource {
        curve: PostEffectBlendCurve::from_legacy(curve.as_deref()),
        keys: Vec::new(),
    })
}

fn parse_key(
    attributes: Vec<XmlAttribute>,
) -> Result<PostEffectKeySource, PostEffectGroupParseError> {
    let mut time = None;
    let mut value = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "time" => time = Some(parse_f32("Key", "time", &attribute.value)?),
            "value" | "Value" => {
                if value
                    .replace(parse_f32("Key", "value", &attribute.value)?)
                    .is_some()
                {
                    return Err(PostEffectGroupParseError::DuplicateAttribute {
                        element: "Key",
                        attribute: "value",
                    });
                }
            }
            _ => {
                return Err(PostEffectGroupParseError::UnknownAttribute {
                    element: "Key",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(PostEffectKeySource {
        time: time.ok_or(PostEffectGroupParseError::MissingAttribute {
            element: "Key",
            attribute: "time",
        })?,
        value: value.ok_or(PostEffectGroupParseError::MissingAttribute {
            element: "Key",
            attribute: "value",
        })?,
    })
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<XmlAttribute>, PostEffectGroupParseError> {
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

fn parse_u32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<u32, PostEffectGroupParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| PostEffectGroupParseError::InvalidInteger {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}

fn parse_bool(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<bool, PostEffectGroupParseError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => Err(PostEffectGroupParseError::InvalidBool {
            element,
            attribute,
            value: value.to_string(),
        }),
    }
}

fn parse_f32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<f32, PostEffectGroupParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| PostEffectGroupParseError::InvalidFloat {
            element,
            attribute,
            value: value.to_string(),
            source,
        })
}

fn parse_vec4(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<Vec4Source, PostEffectGroupParseError> {
    let values = parse_four_floats(element, attribute, value)?;
    Ok(Vec4Source {
        x: values[0],
        y: values[1],
        z: values[2],
        w: values[3],
    })
}

fn parse_color(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<ColorRgbaSource, PostEffectGroupParseError> {
    let values = parse_four_floats(element, attribute, value)?;
    Ok(ColorRgbaSource {
        r: values[0],
        g: values[1],
        b: values[2],
        a: values[3],
    })
}

fn parse_four_floats(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<[f32; 4], PostEffectGroupParseError> {
    let parts: Vec<_> = value.split(',').map(str::trim).collect();
    let values: [&str; 4] =
        parts.try_into().map_err(
            |parts: Vec<&str>| PostEffectGroupParseError::InvalidVector {
                element,
                attribute,
                value: value.to_string(),
                components: parts.len(),
            },
        )?;

    Ok([
        parse_f32(element, attribute, values[0])?,
        parse_f32(element, attribute, values[1])?,
        parse_f32(element, attribute, values[2])?,
        parse_f32(element, attribute, values[3])?,
    ])
}
#[derive(Debug, thiserror::Error)]
pub enum PostEffectGroupParseError {
    #[error("unsupported post-effect XML path {path}")]
    UnsupportedPath { path: String },
    #[error("post-effect XML is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("missing <{element}> element")]
    MissingElement { element: &'static str },
    #[error("duplicate <{element}> element")]
    DuplicateElement { element: &'static str },
    #[error("element appears after closing </PostEffectGroup>")]
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
    #[error("<{element}> cannot appear inside <{parent}>")]
    ElementInWrongParent {
        element: &'static str,
        parent: &'static str,
    },
    #[error("XML document ended before closing <{element}>")]
    UnclosedElement { element: String },
    #[error("unexpected text in post-effect XML: {text:?}")]
    UnexpectedText { text: String },
    #[error("invalid integer {value:?} in <{element}> {attribute}: {source}")]
    InvalidInteger {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseIntError,
    },
    #[error("invalid bool {value:?} in <{element}> {attribute}")]
    InvalidBool {
        element: &'static str,
        attribute: &'static str,
        value: String,
    },
    #[error("invalid float {value:?} in <{element}> {attribute}: {source}")]
    InvalidFloat {
        element: &'static str,
        attribute: &'static str,
        value: String,
        source: ParseFloatError,
    },
    #[error(
        "invalid four-component vector {value:?} in <{element}> {attribute}: got {components} components"
    )]
    InvalidVector {
        element: &'static str,
        attribute: &'static str,
        value: String,
        components: usize,
    },
    #[error("<Param> has more than one value attribute")]
    MultipleParamValues,
    #[error("<Param name={name:?}> has no value attribute")]
    MissingParamValue { name: String },
}
