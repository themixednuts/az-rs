use std::str;

use quick_xml::{
    Reader,
    events::{BytesStart, Event, attributes::AttrError},
};
use serde::{Deserialize, Serialize};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::to_ron_bytes;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialOverrideSource {
    pub source_path: String,
    pub hold_last_frame: Option<bool>,
    pub max_trigger_distance: Option<MaterialOverrideMaxTriggerDistanceSource>,
    pub is_transparent: Option<bool>,
    pub materials: Vec<MaterialOverrideMaterialSource>,
    pub sub_materials: Vec<MaterialOverrideSubMaterialSource>,
    pub comments: Vec<String>,
}

impl MaterialOverrideSource {
    /// Parse a legacy material-override library payload.
    ///
    /// # Errors
    ///
    /// Returns [`MaterialOverrideParseError::UnsupportedPath`] when
    /// `source_path` is not a `libs/materialoverrides/` asset,
    /// [`MaterialOverrideParseError::InvalidUtf8`] when `bytes` are not UTF-8,
    /// and any parser error the XML reader reports for malformed markup or an
    /// unparsable attribute value.
    pub fn from_legacy(
        source_path: &str,
        bytes: &[u8],
    ) -> Result<Self, MaterialOverrideParseError> {
        let normalized_source_path = az_asset_builder::normalize_source_path(source_path);
        if !is_material_override_xml_path(&normalized_source_path) {
            return Err(MaterialOverrideParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let xml = str::from_utf8(bytes).map_err(MaterialOverrideParseError::InvalidUtf8)?;
        MaterialOverrideParser::new(normalized_source_path).parse(xml)
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterialOverrideMaxTriggerDistanceSource {
    Distance(MaterialOverrideMaxTriggerDistanceValueSource),
    Preset(MaterialOverrideMaxTriggerDistancePresetSource),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialOverrideMaxTriggerDistanceValueSource {
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialOverrideMaxTriggerDistancePresetSource {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialOverrideMaterialSource {
    pub name: String,
    pub exclude: Option<String>,
    pub nodes: Vec<MaterialOverrideNodeSource>,
    pub sub_materials: Vec<MaterialOverrideSubMaterialSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialOverrideSubMaterialSource {
    pub name: String,
    pub nodes: Vec<MaterialOverrideNodeSource>,
    pub sub_materials: Vec<Self>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialOverrideNodeSource {
    pub name: String,
    pub attributes: Vec<MaterialOverrideAttributeSource>,
    pub params: Vec<MaterialOverrideParamSource>,
    pub children: Vec<Self>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialOverrideAttributeSource {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialOverrideParamSource {
    pub name: String,
    pub value_type: String,
    pub value: String,
}

#[must_use]
pub fn is_material_override_xml_path(normalized_source_path: &str) -> bool {
    normalized_source_path.starts_with("libs/materialoverrides/")
        && crate::has_extension(normalized_source_path, "xml")
}

struct MaterialOverrideParser {
    source_path: String,
    source: Option<MaterialOverrideSource>,
    target_stack: Vec<MaterialOverrideTargetBuilder>,
    node_stack: Vec<MaterialOverrideNodeSource>,
    pending_root_comments: Vec<String>,
    root_closed: bool,
}

impl MaterialOverrideParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            source: None,
            target_stack: Vec::new(),
            node_stack: Vec::new(),
            pending_root_comments: Vec::new(),
            root_closed: false,
        }
    }

    fn parse(mut self, xml: &str) -> Result<MaterialOverrideSource, MaterialOverrideParseError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        loop {
            match reader.read_event()? {
                Event::Start(event) => self.start_element(&reader, &event)?,
                Event::Empty(event) => {
                    self.start_element(&reader, &event)?;
                    self.end_element(&element_name(&event))?;
                }
                Event::End(event) => {
                    let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                    self.end_element(&name)?;
                }
                Event::Text(event) => {
                    let text = xml_text_content(&event)?.into_owned();
                    Self::text(text)?;
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?.into_owned();
                    Self::text(text)?;
                }
                Event::Comment(event) => self.comment(event.as_ref()),
                Event::GeneralRef(event) => {
                    Self::text(xml_general_reference_content(&event)?.into_owned())?;
                }
                Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if let Some(node) = self.node_stack.last() {
            return Err(MaterialOverrideParseError::UnclosedElement {
                element: node.name.clone(),
            });
        }
        if let Some(target) = self.target_stack.last() {
            return Err(MaterialOverrideParseError::UnclosedElement {
                element: target.element_name().to_string(),
            });
        }

        self.source
            .ok_or(MaterialOverrideParseError::MissingElement {
                element: "MaterialParamsOverride",
            })
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialOverrideParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;
        match name.as_str() {
            "MaterialParamsOverride" => self.start_root(attributes),
            "Material" => self.start_material(attributes),
            "SubMaterial" => self.start_sub_material(attributes),
            "param" => self.add_param(attributes),
            _ => self.start_node(name, attributes),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), MaterialOverrideParseError> {
        match name {
            "MaterialParamsOverride" => self.end_root(),
            "Material" => self.finish_material(),
            "SubMaterial" => self.finish_sub_material(),
            "param" => Ok(()),
            _ => self.finish_node(name),
        }
    }

    fn start_root(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialOverrideParseError> {
        if self.source.is_some() {
            return Err(MaterialOverrideParseError::DuplicateElement {
                element: "MaterialParamsOverride",
            });
        }

        let mut source = MaterialOverrideSource {
            source_path: self.source_path.clone(),
            hold_last_frame: None,
            max_trigger_distance: None,
            is_transparent: None,
            materials: Vec::new(),
            sub_materials: Vec::new(),
            comments: std::mem::take(&mut self.pending_root_comments),
        };

        for attribute in attributes {
            match attribute.name.as_str() {
                "HoldLastFrame" => {
                    source.hold_last_frame = Some(parse_bool(
                        "MaterialParamsOverride",
                        "HoldLastFrame",
                        &attribute.value,
                    )?);
                }
                "MaxTriggerDistance" => {
                    source.max_trigger_distance =
                        Some(parse_max_trigger_distance(&attribute.value));
                }
                "IsTransparent" => {
                    source.is_transparent = Some(parse_bool(
                        "MaterialParamsOverride",
                        "IsTransparent",
                        &attribute.value,
                    )?);
                }
                _ => {
                    return Err(MaterialOverrideParseError::UnknownAttribute {
                        element: "MaterialParamsOverride",
                        attribute: attribute.name,
                    });
                }
            }
        }

        self.source = Some(source);
        Ok(())
    }

    fn end_root(&mut self) -> Result<(), MaterialOverrideParseError> {
        if let Some(node) = self.node_stack.last() {
            return Err(MaterialOverrideParseError::UnclosedElement {
                element: node.name.clone(),
            });
        }
        if let Some(target) = self.target_stack.last() {
            return Err(MaterialOverrideParseError::UnclosedElement {
                element: target.element_name().to_string(),
            });
        }

        self.root_closed = true;
        Ok(())
    }

    fn start_material(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialOverrideParseError> {
        self.source_mut()?;
        if !self.target_stack.is_empty() || !self.node_stack.is_empty() {
            return Err(MaterialOverrideParseError::ElementInWrongParent {
                element: "Material",
                parent: "MaterialParamsOverride",
            });
        }

        self.target_stack
            .push(MaterialOverrideTargetBuilder::Material(parse_material(
                attributes,
            )?));
        Ok(())
    }

    fn finish_material(&mut self) -> Result<(), MaterialOverrideParseError> {
        if let Some(node) = self.node_stack.last() {
            return Err(MaterialOverrideParseError::UnclosedElement {
                element: node.name.clone(),
            });
        }

        let target = self.target_stack.pop().ok_or_else(|| {
            MaterialOverrideParseError::UnexpectedEndElement {
                element: "Material".to_string(),
            }
        })?;
        let MaterialOverrideTargetBuilder::Material(material) = target else {
            return Err(MaterialOverrideParseError::UnexpectedEndElement {
                element: "Material".to_string(),
            });
        };
        self.source_mut()?.materials.push(material);
        Ok(())
    }

    fn start_sub_material(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialOverrideParseError> {
        self.source_mut()?;
        if !self.node_stack.is_empty() {
            return Err(MaterialOverrideParseError::ElementInWrongParent {
                element: "SubMaterial",
                parent: "Material/SubMaterial",
            });
        }

        self.target_stack
            .push(MaterialOverrideTargetBuilder::SubMaterial(
                parse_sub_material(attributes)?,
            ));
        Ok(())
    }

    fn finish_sub_material(&mut self) -> Result<(), MaterialOverrideParseError> {
        if let Some(node) = self.node_stack.last() {
            return Err(MaterialOverrideParseError::UnclosedElement {
                element: node.name.clone(),
            });
        }

        let target = self.target_stack.pop().ok_or_else(|| {
            MaterialOverrideParseError::UnexpectedEndElement {
                element: "SubMaterial".to_string(),
            }
        })?;
        let MaterialOverrideTargetBuilder::SubMaterial(sub_material) = target else {
            return Err(MaterialOverrideParseError::UnexpectedEndElement {
                element: "SubMaterial".to_string(),
            });
        };

        if let Some(parent) = self.target_stack.last_mut() {
            parent.sub_materials_mut().push(sub_material);
        } else {
            self.source_mut()?.sub_materials.push(sub_material);
        }
        Ok(())
    }

    fn start_node(
        &mut self,
        name: String,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialOverrideParseError> {
        self.current_target_mut()?;
        self.node_stack.push(MaterialOverrideNodeSource {
            name,
            attributes: attributes
                .into_iter()
                .map(|attribute| MaterialOverrideAttributeSource {
                    name: attribute.name,
                    value: attribute.value,
                })
                .collect(),
            params: Vec::new(),
            children: Vec::new(),
            comments: Vec::new(),
        });
        Ok(())
    }

    fn finish_node(&mut self, name: &str) -> Result<(), MaterialOverrideParseError> {
        let node = self.node_stack.pop().ok_or_else(|| {
            MaterialOverrideParseError::UnexpectedEndElement {
                element: name.to_string(),
            }
        })?;
        if node.name != name {
            return Err(MaterialOverrideParseError::UnexpectedEndElement {
                element: name.to_string(),
            });
        }

        if let Some(parent) = self.node_stack.last_mut() {
            parent.children.push(node);
        } else {
            self.current_target_mut()?.nodes_mut().push(node);
        }
        Ok(())
    }

    fn add_param(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialOverrideParseError> {
        let param = parse_param(attributes)?;
        let node =
            self.node_stack
                .last_mut()
                .ok_or(MaterialOverrideParseError::ElementInWrongParent {
                    element: "param",
                    parent: "override node",
                })?;
        node.params.push(param);
        Ok(())
    }

    fn text(text: String) -> Result<(), MaterialOverrideParseError> {
        if text.trim().is_empty() {
            return Ok(());
        }

        Err(MaterialOverrideParseError::UnexpectedText { text })
    }

    fn comment(&mut self, bytes: &[u8]) {
        let comment = String::from_utf8_lossy(bytes).trim().to_string();
        if comment.is_empty() {
            return;
        }

        if let Some(node) = self.node_stack.last_mut() {
            node.comments.push(comment);
        } else if let Some(target) = self.target_stack.last_mut() {
            target.comments_mut().push(comment);
        } else if let Some(source) = self.source.as_mut() {
            source.comments.push(comment);
        } else {
            self.pending_root_comments.push(comment);
        }
    }

    fn source_mut(&mut self) -> Result<&mut MaterialOverrideSource, MaterialOverrideParseError> {
        if self.root_closed {
            return Err(MaterialOverrideParseError::ElementAfterRoot);
        }
        self.source
            .as_mut()
            .ok_or(MaterialOverrideParseError::MissingElement {
                element: "MaterialParamsOverride",
            })
    }

    fn current_target_mut(
        &mut self,
    ) -> Result<&mut MaterialOverrideTargetBuilder, MaterialOverrideParseError> {
        self.target_stack
            .last_mut()
            .ok_or(MaterialOverrideParseError::ElementInWrongParent {
                element: "override node",
                parent: "Material/SubMaterial",
            })
    }
}

enum MaterialOverrideTargetBuilder {
    Material(MaterialOverrideMaterialSource),
    SubMaterial(MaterialOverrideSubMaterialSource),
}

impl MaterialOverrideTargetBuilder {
    const fn element_name(&self) -> &'static str {
        match self {
            Self::Material(_) => "Material",
            Self::SubMaterial(_) => "SubMaterial",
        }
    }

    const fn nodes_mut(&mut self) -> &mut Vec<MaterialOverrideNodeSource> {
        match self {
            Self::Material(material) => &mut material.nodes,
            Self::SubMaterial(sub_material) => &mut sub_material.nodes,
        }
    }

    const fn sub_materials_mut(&mut self) -> &mut Vec<MaterialOverrideSubMaterialSource> {
        match self {
            Self::Material(material) => &mut material.sub_materials,
            Self::SubMaterial(sub_material) => &mut sub_material.sub_materials,
        }
    }

    const fn comments_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Material(material) => &mut material.comments,
            Self::SubMaterial(sub_material) => &mut sub_material.comments,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlAttribute {
    name: String,
    value: String,
}

fn parse_material(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialOverrideMaterialSource, MaterialOverrideParseError> {
    let mut name = None;
    let mut exclude = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            "exclude" => exclude = Some(attribute.value),
            _ => {
                return Err(MaterialOverrideParseError::UnknownAttribute {
                    element: "Material",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialOverrideMaterialSource {
        name: name.ok_or(MaterialOverrideParseError::MissingAttribute {
            element: "Material",
            attribute: "name",
        })?,
        exclude,
        nodes: Vec::new(),
        sub_materials: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_sub_material(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialOverrideSubMaterialSource, MaterialOverrideParseError> {
    let mut name = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            _ => {
                return Err(MaterialOverrideParseError::UnknownAttribute {
                    element: "SubMaterial",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialOverrideSubMaterialSource {
        name: name.ok_or(MaterialOverrideParseError::MissingAttribute {
            element: "SubMaterial",
            attribute: "name",
        })?,
        nodes: Vec::new(),
        sub_materials: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_param(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialOverrideParamSource, MaterialOverrideParseError> {
    let mut name = None;
    let mut value_type = None;
    let mut value = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            "type" => value_type = Some(attribute.value),
            "value" => value = Some(attribute.value),
            _ => {
                return Err(MaterialOverrideParseError::UnknownAttribute {
                    element: "param",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialOverrideParamSource {
        name: name.ok_or(MaterialOverrideParseError::MissingAttribute {
            element: "param",
            attribute: "name",
        })?,
        value_type: value_type.ok_or(MaterialOverrideParseError::MissingAttribute {
            element: "param",
            attribute: "type",
        })?,
        value: value.ok_or(MaterialOverrideParseError::MissingAttribute {
            element: "param",
            attribute: "value",
        })?,
    })
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<XmlAttribute>, MaterialOverrideParseError> {
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

fn parse_bool(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<bool, MaterialOverrideParseError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => Err(MaterialOverrideParseError::InvalidBool {
            element,
            attribute,
            value: value.to_string(),
        }),
    }
}

fn parse_max_trigger_distance(value: &str) -> MaterialOverrideMaxTriggerDistanceSource {
    value.trim().parse().map_or_else(
        |_| {
            MaterialOverrideMaxTriggerDistanceSource::Preset(
                MaterialOverrideMaxTriggerDistancePresetSource {
                    name: value.to_string(),
                },
            )
        },
        |distance| {
            MaterialOverrideMaxTriggerDistanceSource::Distance(
                MaterialOverrideMaxTriggerDistanceValueSource { value: distance },
            )
        },
    )
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialOverrideParseError {
    #[error("unsupported material-override XML path {path}")]
    UnsupportedPath { path: String },
    #[error("material-override XML is not UTF-8")]
    InvalidUtf8(str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("missing <{element}> element")]
    MissingElement { element: &'static str },
    #[error("duplicate <{element}> element")]
    DuplicateElement { element: &'static str },
    #[error("element appears after closing </MaterialParamsOverride>")]
    ElementAfterRoot,
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
    #[error("<{element}> cannot appear inside <{parent}>")]
    ElementInWrongParent {
        element: &'static str,
        parent: &'static str,
    },
    #[error("unexpected </{element}>")]
    UnexpectedEndElement { element: String },
    #[error("XML document ended before closing <{element}>")]
    UnclosedElement { element: String },
    #[error("unexpected text in material-override XML: {text:?}")]
    UnexpectedText { text: String },
    #[error("invalid bool {value:?} in <{element}> {attribute}")]
    InvalidBool {
        element: &'static str,
        attribute: &'static str,
        value: String,
    },
}
