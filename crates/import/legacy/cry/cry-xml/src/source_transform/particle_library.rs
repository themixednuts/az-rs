use std::str;

use az_asset_builder::normalize_source_path;
use quick_xml::{
    Reader,
    events::{BytesStart, Event, attributes::AttrError},
};
use serde::{Deserialize, Serialize};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::{XmlAttribute, to_ron_bytes};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleLibrarySource {
    pub source_path: String,
    pub name: String,
    pub filename: Option<String>,
    pub sandbox_version: Option<String>,
    pub particle_version: Option<String>,
    pub attributes: Vec<ParticleAttributeSource>,
    pub params: ParticleParamBagSource,
    pub dynamic_params: ParticleParamBagSource,
    pub dynamic_param_interpolation: ParticleParamBagSource,
    pub settings: Vec<ParticleLibrarySettingsSource>,
    pub folders: Vec<ParticleLibraryFolderSource>,
    pub effects: Vec<ParticleEffectSource>,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

impl ParticleLibrarySource {
    /// Parse a legacy particle-library payload.
    ///
    /// # Errors
    ///
    /// Returns [`ParticleLibraryParseError::UnsupportedPath`] when
    /// `source_path` is not a `libs/particles/` asset,
    /// [`ParticleLibraryParseError::InvalidUtf8`] when `bytes` are not UTF-8,
    /// and any parser error the XML reader reports for malformed markup or an
    /// unparsable attribute value.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, ParticleLibraryParseError> {
        let normalized_source_path = normalize_source_path(source_path);
        if !is_particle_library_xml_path(&normalized_source_path) {
            return Err(ParticleLibraryParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        ParticleLibraryParser::new(normalized_source_path).parse(str::from_utf8(bytes)?)
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParticleParamBagSource {
    pub entries: Vec<ParticleAttributeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticleAttributeSource {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleLibrarySettingsSource {
    pub name: String,
    pub attributes: Vec<ParticleAttributeSource>,
    pub params: ParticleParamBagSource,
    pub dynamic_params: ParticleParamBagSource,
    pub dynamic_param_interpolation: ParticleParamBagSource,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleLibraryFolderSource {
    pub name: String,
    pub attributes: Vec<ParticleAttributeSource>,
    pub params: ParticleParamBagSource,
    pub dynamic_params: ParticleParamBagSource,
    pub dynamic_param_interpolation: ParticleParamBagSource,
    pub children: Vec<ParticleEffectSource>,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleEffectSource {
    pub name: String,
    pub attributes: Vec<ParticleAttributeSource>,
    pub params: ParticleParamBagSource,
    pub dynamic_params: ParticleParamBagSource,
    pub dynamic_param_interpolation: ParticleParamBagSource,
    pub settings: Vec<ParticleLibrarySettingsSource>,
    pub children: Vec<Self>,
    pub lods: Vec<ParticleLodsSource>,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleLodsSource {
    pub attributes: Vec<ParticleAttributeSource>,
    pub levels: Vec<ParticleLodLevelSource>,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleLodLevelSource {
    pub attributes: Vec<ParticleAttributeSource>,
    pub particles: Vec<ParticleLodParticleSource>,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleLodParticleSource {
    pub distance: Option<String>,
    pub active: Option<String>,
    pub attributes: Vec<ParticleAttributeSource>,
    pub effect: ParticleEffectSource,
    pub extra_nodes: Vec<ParticleExtraNodeSource>,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleExtraNodeSource {
    pub name: String,
    pub attributes: Vec<ParticleAttributeSource>,
    pub children: Vec<Self>,
    pub comments: Vec<String>,
}

#[must_use]
pub fn is_particle_library_xml_path(normalized_source_path: &str) -> bool {
    normalized_source_path.starts_with("libs/particles/")
        && crate::has_extension(normalized_source_path, "xml")
}

struct ParticleLibraryParser {
    source_path: String,
    source: Option<ParticleLibrarySource>,
    stack: Vec<ParticleFrame>,
    pending_root_comments: Vec<String>,
    root_closed: bool,
}

impl ParticleLibraryParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            source: None,
            stack: Vec::new(),
            pending_root_comments: Vec::new(),
            root_closed: false,
        }
    }

    fn parse(mut self, xml: &str) -> Result<ParticleLibrarySource, ParticleLibraryParseError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        loop {
            match reader.read_event()? {
                Event::Start(event) => self.start_element(&reader, &event)?,
                Event::Empty(event) => {
                    let name = element_name(&event);
                    self.start_element(&reader, &event)?;
                    self.end_element(&name)?;
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

        if let Some(frame) = self.stack.last() {
            return Err(ParticleLibraryParseError::UnclosedElement {
                element: frame.element_name().to_string(),
            });
        }

        self.source
            .ok_or(ParticleLibraryParseError::MissingElement {
                element: "ParticleLibrary",
            })
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), ParticleLibraryParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;

        if self.root_closed {
            return Err(ParticleLibraryParseError::ElementAfterRoot);
        }

        if self.stack.is_empty() {
            return self.start_root(&name, attributes);
        }

        if matches!(self.stack.last(), Some(ParticleFrame::Extra(_))) {
            self.stack
                .push(ParticleFrame::Extra(parse_extra_node(name, attributes)));
            return Ok(());
        }

        match name.as_str() {
            "Folder" if self.can_start_root_child() => {
                self.stack
                    .push(ParticleFrame::Folder(parse_named_folder(attributes)?));
            }
            "Settings" if self.can_start_settings() => {
                self.stack
                    .push(ParticleFrame::Settings(parse_named_settings(attributes)?));
            }
            "Particles" if self.can_start_particle_effect() => {
                self.stack.push(ParticleFrame::Effect {
                    element: "Particles",
                    source: parse_particle_effect("Particles", attributes)?,
                });
            }
            "Particle" if self.can_start_lod_effect() => {
                self.stack.push(ParticleFrame::Effect {
                    element: "Particle",
                    source: parse_particle_effect("Particle", attributes)?,
                });
            }
            "Childs" if self.can_start_childs() => {
                self.stack.push(ParticleFrame::Childs {
                    children: Vec::new(),
                    extra_nodes: Vec::new(),
                    comments: Vec::new(),
                });
            }
            "LODs" if self.current_effect_mut().is_some() => {
                self.stack.push(ParticleFrame::Lods(ParticleLodsSource {
                    attributes: raw_attributes(attributes),
                    levels: Vec::new(),
                    extra_nodes: Vec::new(),
                    comments: Vec::new(),
                }));
            }
            "LevelOfDetail" if matches!(self.stack.last(), Some(ParticleFrame::Lods(_))) => {
                self.stack
                    .push(ParticleFrame::Level(ParticleLodLevelSource {
                        attributes: raw_attributes(attributes),
                        particles: Vec::new(),
                        extra_nodes: Vec::new(),
                        comments: Vec::new(),
                    }));
            }
            "LodParticle" if matches!(self.stack.last(), Some(ParticleFrame::Level(_))) => {
                self.stack
                    .push(ParticleFrame::LodParticle(parse_lod_particle(attributes)));
            }
            "Params" => self.add_param_bag("Params", attributes)?,
            "DynamicParams" => self.add_param_bag("DynamicParams", attributes)?,
            "DynamicParamsInterpolateOverride" => {
                self.add_param_bag("DynamicParamsInterpolateOverride", attributes)?;
            }
            _ => {
                self.stack
                    .push(ParticleFrame::Extra(parse_extra_node(name, attributes)));
            }
        }

        Ok(())
    }

    fn start_root(
        &mut self,
        name: &str,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), ParticleLibraryParseError> {
        if name != "ParticleLibrary" {
            return Err(ParticleLibraryParseError::MissingElement {
                element: "ParticleLibrary",
            });
        }
        if self.source.is_some() {
            return Err(ParticleLibraryParseError::DuplicateElement {
                element: "ParticleLibrary",
            });
        }

        let mut source = parse_particle_library(self.source_path.clone(), attributes)?;
        source.comments = std::mem::take(&mut self.pending_root_comments);
        self.stack.push(ParticleFrame::Library(source));
        Ok(())
    }

    fn end_element(&mut self, name: &str) -> Result<(), ParticleLibraryParseError> {
        let frame =
            self.stack
                .pop()
                .ok_or_else(|| ParticleLibraryParseError::UnexpectedEndElement {
                    element: name.to_string(),
                })?;

        match frame {
            ParticleFrame::Library(source) => {
                expect_end(name, "ParticleLibrary")?;
                self.source = Some(source);
                self.root_closed = true;
            }
            ParticleFrame::Folder(folder) => {
                expect_end(name, "Folder")?;
                self.add_folder_to_parent(folder)?;
            }
            ParticleFrame::Settings(settings) => {
                expect_end(name, "Settings")?;
                self.add_settings_to_parent(settings)?;
            }
            ParticleFrame::Effect { element, source } => {
                expect_end(name, element)?;
                self.add_effect_to_parent(source)?;
            }
            ParticleFrame::Childs {
                children,
                extra_nodes,
                comments,
            } => {
                expect_end(name, "Childs")?;
                self.add_childs_to_parent(children, extra_nodes, comments)?;
            }
            ParticleFrame::Lods(lods) => {
                expect_end(name, "LODs")?;
                self.require_effect_parent("LODs")?.lods.push(lods);
            }
            ParticleFrame::Level(level) => {
                expect_end(name, "LevelOfDetail")?;
                let Some(ParticleFrame::Lods(lods)) = self.stack.last_mut() else {
                    return Err(element_parent_error("LevelOfDetail"));
                };
                lods.levels.push(level);
            }
            ParticleFrame::LodParticle(builder) => {
                expect_end(name, "LodParticle")?;
                let particle = builder.finish()?;
                let Some(ParticleFrame::Level(level)) = self.stack.last_mut() else {
                    return Err(element_parent_error("LodParticle"));
                };
                level.particles.push(particle);
            }
            ParticleFrame::Extra(node) => {
                expect_end(name, &node.name)?;
                self.add_extra_to_parent(node)?;
            }
            ParticleFrame::ParamBag(expected) => {
                expect_end(name, expected)?;
            }
        }

        Ok(())
    }

    fn add_param_bag(
        &mut self,
        element: &'static str,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), ParticleLibraryParseError> {
        let entries = raw_attributes(attributes);
        let bag = ParticleParamBagSource { entries };

        match self.current_param_bag_target_mut() {
            Some(mut target) => match element {
                "Params" => target.params_mut().entries.extend(bag.entries),
                "DynamicParams" => target.dynamic_params_mut().entries.extend(bag.entries),
                "DynamicParamsInterpolateOverride" => target
                    .dynamic_param_interpolation_mut()
                    .entries
                    .extend(bag.entries),
                _ => unreachable!("unknown particle param bag"),
            },
            None => return Err(element_parent_error(element)),
        }

        self.stack.push(ParticleFrame::ParamBag(element));
        Ok(())
    }

    fn add_folder_to_parent(
        &mut self,
        folder: ParticleLibraryFolderSource,
    ) -> Result<(), ParticleLibraryParseError> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Library(source)) => source.folders.push(folder),
            _ => return Err(element_parent_error("Folder")),
        }
        Ok(())
    }

    fn add_settings_to_parent(
        &mut self,
        settings: ParticleLibrarySettingsSource,
    ) -> Result<(), ParticleLibraryParseError> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Library(source)) => source.settings.push(settings),
            Some(ParticleFrame::Effect { source, .. }) => source.settings.push(settings),
            _ => return Err(element_parent_error("Settings")),
        }
        Ok(())
    }

    fn add_effect_to_parent(
        &mut self,
        effect: ParticleEffectSource,
    ) -> Result<(), ParticleLibraryParseError> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Library(source)) => source.effects.push(effect),
            Some(ParticleFrame::Childs { children, .. }) => children.push(effect),
            Some(ParticleFrame::Effect { source, .. }) => source.children.push(effect),
            Some(ParticleFrame::LodParticle(builder)) => {
                if builder.effect.replace(effect).is_some() {
                    return Err(ParticleLibraryParseError::DuplicateElement {
                        element: "Particle",
                    });
                }
            }
            _ => return Err(element_parent_error("Particles/Particle")),
        }
        Ok(())
    }

    fn add_childs_to_parent(
        &mut self,
        children: Vec<ParticleEffectSource>,
        extra_nodes: Vec<ParticleExtraNodeSource>,
        comments: Vec<String>,
    ) -> Result<(), ParticleLibraryParseError> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Folder(folder)) => {
                folder.children.extend(children);
                folder.extra_nodes.extend(extra_nodes);
                folder.comments.extend(comments);
            }
            Some(ParticleFrame::Effect { source, .. }) => {
                source.children.extend(children);
                source.extra_nodes.extend(extra_nodes);
                source.comments.extend(comments);
            }
            _ => return Err(element_parent_error("Childs")),
        }
        Ok(())
    }

    fn add_extra_to_parent(
        &mut self,
        node: ParticleExtraNodeSource,
    ) -> Result<(), ParticleLibraryParseError> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Library(source)) => source.extra_nodes.push(node),
            Some(ParticleFrame::Folder(folder)) => folder.extra_nodes.push(node),
            Some(ParticleFrame::Settings(settings)) => settings.extra_nodes.push(node),
            Some(ParticleFrame::Effect { source, .. }) => source.extra_nodes.push(node),
            Some(ParticleFrame::Childs { extra_nodes, .. }) => extra_nodes.push(node),
            Some(ParticleFrame::Lods(lods)) => lods.extra_nodes.push(node),
            Some(ParticleFrame::Level(level)) => level.extra_nodes.push(node),
            Some(ParticleFrame::LodParticle(builder)) => builder.extra_nodes.push(node),
            Some(ParticleFrame::Extra(parent)) => parent.children.push(node),
            Some(ParticleFrame::ParamBag(_)) | None => return Err(element_parent_error("extra")),
        }
        Ok(())
    }

    fn text(text: String) -> Result<(), ParticleLibraryParseError> {
        if text.trim().is_empty() {
            return Ok(());
        }

        Err(ParticleLibraryParseError::UnexpectedText { text })
    }

    fn comment(&mut self, bytes: &[u8]) {
        let comment = String::from_utf8_lossy(bytes).trim().to_string();
        if comment.is_empty() {
            return;
        }

        if self.stack.is_empty() {
            self.pending_root_comments.push(comment);
            return;
        }

        let index = if matches!(self.stack.last(), Some(ParticleFrame::ParamBag(_))) {
            self.stack.len().saturating_sub(2)
        } else {
            self.stack.len() - 1
        };

        match &mut self.stack[index] {
            ParticleFrame::Library(source) => source.comments.push(comment),
            ParticleFrame::Folder(folder) => folder.comments.push(comment),
            ParticleFrame::Settings(settings) => settings.comments.push(comment),
            ParticleFrame::Effect { source, .. } => source.comments.push(comment),
            ParticleFrame::Childs { comments, .. } => comments.push(comment),
            ParticleFrame::Lods(lods) => lods.comments.push(comment),
            ParticleFrame::Level(level) => level.comments.push(comment),
            ParticleFrame::LodParticle(builder) => builder.comments.push(comment),
            ParticleFrame::Extra(node) => node.comments.push(comment),
            ParticleFrame::ParamBag(_) => self.pending_root_comments.push(comment),
        }
    }

    fn can_start_root_child(&self) -> bool {
        matches!(self.stack.last(), Some(ParticleFrame::Library(_)))
    }

    fn can_start_settings(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(ParticleFrame::Library(_) | ParticleFrame::Effect { .. })
        )
    }

    fn can_start_particle_effect(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(
                ParticleFrame::Library(_)
                    | ParticleFrame::Childs { .. }
                    | ParticleFrame::Effect { .. }
            )
        )
    }

    fn can_start_childs(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(ParticleFrame::Folder(_) | ParticleFrame::Effect { .. })
        )
    }

    fn can_start_lod_effect(&self) -> bool {
        matches!(self.stack.last(), Some(ParticleFrame::LodParticle(_)))
    }

    fn current_effect_mut(&mut self) -> Option<&mut ParticleEffectSource> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Effect { source, .. }) => Some(source),
            _ => None,
        }
    }

    fn require_effect_parent(
        &mut self,
        element: &'static str,
    ) -> Result<&mut ParticleEffectSource, ParticleLibraryParseError> {
        match self.stack.last_mut() {
            Some(ParticleFrame::Effect { source, .. }) => Ok(source),
            _ => Err(element_parent_error(element)),
        }
    }

    fn current_param_bag_target_mut(&mut self) -> Option<ParticleParamBagTarget<'_>> {
        match self.stack.last_mut()? {
            ParticleFrame::Library(source) => Some(ParticleParamBagTarget::Library(source)),
            ParticleFrame::Folder(folder) => Some(ParticleParamBagTarget::Folder(folder)),
            ParticleFrame::Settings(settings) => Some(ParticleParamBagTarget::Settings(settings)),
            ParticleFrame::Effect { source, .. } => Some(ParticleParamBagTarget::Effect(source)),
            _ => None,
        }
    }
}

enum ParticleParamBagTarget<'a> {
    Library(&'a mut ParticleLibrarySource),
    Folder(&'a mut ParticleLibraryFolderSource),
    Settings(&'a mut ParticleLibrarySettingsSource),
    Effect(&'a mut ParticleEffectSource),
}

impl ParticleParamBagTarget<'_> {
    const fn params_mut(&mut self) -> &mut ParticleParamBagSource {
        match self {
            Self::Library(source) => &mut source.params,
            Self::Folder(folder) => &mut folder.params,
            Self::Settings(settings) => &mut settings.params,
            Self::Effect(effect) => &mut effect.params,
        }
    }

    const fn dynamic_params_mut(&mut self) -> &mut ParticleParamBagSource {
        match self {
            Self::Library(source) => &mut source.dynamic_params,
            Self::Folder(folder) => &mut folder.dynamic_params,
            Self::Settings(settings) => &mut settings.dynamic_params,
            Self::Effect(effect) => &mut effect.dynamic_params,
        }
    }

    const fn dynamic_param_interpolation_mut(&mut self) -> &mut ParticleParamBagSource {
        match self {
            Self::Library(source) => &mut source.dynamic_param_interpolation,
            Self::Folder(folder) => &mut folder.dynamic_param_interpolation,
            Self::Settings(settings) => &mut settings.dynamic_param_interpolation,
            Self::Effect(effect) => &mut effect.dynamic_param_interpolation,
        }
    }
}

enum ParticleFrame {
    Library(ParticleLibrarySource),
    Folder(ParticleLibraryFolderSource),
    Settings(ParticleLibrarySettingsSource),
    Effect {
        element: &'static str,
        source: ParticleEffectSource,
    },
    Childs {
        children: Vec<ParticleEffectSource>,
        extra_nodes: Vec<ParticleExtraNodeSource>,
        comments: Vec<String>,
    },
    Lods(ParticleLodsSource),
    Level(ParticleLodLevelSource),
    LodParticle(ParticleLodParticleBuilder),
    Extra(ParticleExtraNodeSource),
    ParamBag(&'static str),
}

impl ParticleFrame {
    fn element_name(&self) -> &str {
        match self {
            Self::Library(_) => "ParticleLibrary",
            Self::Folder(_) => "Folder",
            Self::Settings(_) => "Settings",
            Self::Effect { element, .. } | Self::ParamBag(element) => element,
            Self::Childs { .. } => "Childs",
            Self::Lods(_) => "LODs",
            Self::Level(_) => "LevelOfDetail",
            Self::LodParticle(_) => "LodParticle",
            Self::Extra(node) => &node.name,
        }
    }
}

struct ParticleLodParticleBuilder {
    distance: Option<String>,
    active: Option<String>,
    attributes: Vec<ParticleAttributeSource>,
    effect: Option<ParticleEffectSource>,
    extra_nodes: Vec<ParticleExtraNodeSource>,
    comments: Vec<String>,
}

impl ParticleLodParticleBuilder {
    fn finish(self) -> Result<ParticleLodParticleSource, ParticleLibraryParseError> {
        Ok(ParticleLodParticleSource {
            distance: self.distance,
            active: self.active,
            attributes: self.attributes,
            effect: self
                .effect
                .ok_or(ParticleLibraryParseError::MissingElement {
                    element: "Particle",
                })?,
            extra_nodes: self.extra_nodes,
            comments: self.comments,
        })
    }
}

fn parse_particle_library(
    source_path: String,
    attributes: Vec<XmlAttribute>,
) -> Result<ParticleLibrarySource, ParticleLibraryParseError> {
    let mut name = None;
    let mut filename = None;
    let mut sandbox_version = None;
    let mut particle_version = None;
    let mut raw = Vec::new();

    for attribute in attributes {
        match attribute.name.as_str() {
            "Name" => name = Some(attribute.value),
            "Filename" => filename = Some(attribute.value),
            "SandboxVersion" => sandbox_version = Some(attribute.value),
            "ParticleVersion" => particle_version = Some(attribute.value),
            _ => raw.push(attribute.into()),
        }
    }

    Ok(ParticleLibrarySource {
        source_path,
        name: name.ok_or(ParticleLibraryParseError::MissingAttribute {
            element: "ParticleLibrary",
            attribute: "Name",
        })?,
        filename,
        sandbox_version,
        particle_version,
        attributes: raw,
        params: ParticleParamBagSource::default(),
        dynamic_params: ParticleParamBagSource::default(),
        dynamic_param_interpolation: ParticleParamBagSource::default(),
        settings: Vec::new(),
        folders: Vec::new(),
        effects: Vec::new(),
        extra_nodes: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_named_folder(
    attributes: Vec<XmlAttribute>,
) -> Result<ParticleLibraryFolderSource, ParticleLibraryParseError> {
    let (name, attributes) = parse_named_attributes("Folder", attributes)?;
    Ok(ParticleLibraryFolderSource {
        name,
        attributes,
        params: ParticleParamBagSource::default(),
        dynamic_params: ParticleParamBagSource::default(),
        dynamic_param_interpolation: ParticleParamBagSource::default(),
        children: Vec::new(),
        extra_nodes: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_named_settings(
    attributes: Vec<XmlAttribute>,
) -> Result<ParticleLibrarySettingsSource, ParticleLibraryParseError> {
    let (name, attributes) = parse_named_attributes("Settings", attributes)?;
    Ok(ParticleLibrarySettingsSource {
        name,
        attributes,
        params: ParticleParamBagSource::default(),
        dynamic_params: ParticleParamBagSource::default(),
        dynamic_param_interpolation: ParticleParamBagSource::default(),
        extra_nodes: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_particle_effect(
    element: &'static str,
    attributes: Vec<XmlAttribute>,
) -> Result<ParticleEffectSource, ParticleLibraryParseError> {
    let (name, attributes) = parse_named_attributes(element, attributes)?;
    Ok(ParticleEffectSource {
        name,
        attributes,
        params: ParticleParamBagSource::default(),
        dynamic_params: ParticleParamBagSource::default(),
        dynamic_param_interpolation: ParticleParamBagSource::default(),
        settings: Vec::new(),
        children: Vec::new(),
        lods: Vec::new(),
        extra_nodes: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_named_attributes(
    element: &'static str,
    attributes: Vec<XmlAttribute>,
) -> Result<(String, Vec<ParticleAttributeSource>), ParticleLibraryParseError> {
    let mut name = None;
    let mut raw = Vec::new();
    for attribute in attributes {
        if attribute.name == "Name" {
            name = Some(attribute.value);
        } else {
            raw.push(attribute.into());
        }
    }

    Ok((
        name.ok_or(ParticleLibraryParseError::MissingAttribute {
            element,
            attribute: "Name",
        })?,
        raw,
    ))
}

fn parse_lod_particle(attributes: Vec<XmlAttribute>) -> ParticleLodParticleBuilder {
    let mut distance = None;
    let mut active = None;
    let mut raw = Vec::new();

    for attribute in attributes {
        match attribute.name.as_str() {
            "Distance" => distance = Some(attribute.value),
            "Active" => active = Some(attribute.value),
            _ => raw.push(attribute.into()),
        }
    }

    ParticleLodParticleBuilder {
        distance,
        active,
        attributes: raw,
        effect: None,
        extra_nodes: Vec::new(),
        comments: Vec::new(),
    }
}

fn parse_extra_node(name: String, attributes: Vec<XmlAttribute>) -> ParticleExtraNodeSource {
    ParticleExtraNodeSource {
        name,
        attributes: raw_attributes(attributes),
        children: Vec::new(),
        comments: Vec::new(),
    }
}

fn raw_attributes(attributes: Vec<XmlAttribute>) -> Vec<ParticleAttributeSource> {
    attributes
        .into_iter()
        .map(ParticleAttributeSource::from)
        .collect()
}

impl From<XmlAttribute> for ParticleAttributeSource {
    fn from(attribute: XmlAttribute) -> Self {
        Self {
            name: attribute.name,
            value: attribute.value,
        }
    }
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<XmlAttribute>, ParticleLibraryParseError> {
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

fn expect_end(found: &str, expected: &str) -> Result<(), ParticleLibraryParseError> {
    if found == expected {
        return Ok(());
    }

    Err(ParticleLibraryParseError::MismatchedEndElement {
        expected: expected.to_string(),
        found: found.to_string(),
    })
}

const fn element_parent_error(element: &'static str) -> ParticleLibraryParseError {
    ParticleLibraryParseError::ElementInWrongParent { element }
}

#[derive(Debug, thiserror::Error)]
pub enum ParticleLibraryParseError {
    #[error("unsupported particle-library XML path {path}")]
    UnsupportedPath { path: String },
    #[error("particle-library XML is not UTF-8")]
    InvalidUtf8(#[from] str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("missing <{element}> element")]
    MissingElement { element: &'static str },
    #[error("duplicate <{element}> element")]
    DuplicateElement { element: &'static str },
    #[error("element appears after closing </ParticleLibrary>")]
    ElementAfterRoot,
    #[error("missing {attribute:?} attribute on <{element}>")]
    MissingAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("<{element}> cannot appear in this particle XML parent")]
    ElementInWrongParent { element: &'static str },
    #[error("unexpected </{element}>")]
    UnexpectedEndElement { element: String },
    #[error("mismatched closing element </{found}>; expected </{expected}>")]
    MismatchedEndElement { expected: String, found: String },
    #[error("XML document ended before closing <{element}>")]
    UnclosedElement { element: String },
    #[error("unexpected text in particle-library XML: {text:?}")]
    UnexpectedText { text: String },
}
