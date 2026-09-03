use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::{
    MaterialEffectAudioSource, MaterialEffectAudioSwitchSource, MaterialEffectDecalSource,
    MaterialEffectFilterSource, MaterialEffectForceFeedbackSource,
    MaterialEffectParticleDirectionSource, MaterialEffectParticleNameSource,
    MaterialEffectParticleSource, MaterialEffectRandomSource, MaterialEffectResourceSource,
    MaterialEffectSource, MaterialEffectsLibrarySource, MaterialEffectsParseError,
    MaterialEffectsSource, str,
};

pub(super) fn parse_fx_library(
    source_path: String,
    xml: &str,
) -> Result<MaterialEffectsSource, MaterialEffectsParseError> {
    MaterialEffectsParser::new(source_path).parse(xml)
}

struct MaterialEffectsParser {
    source_path: String,
    source: Option<MaterialEffectsSource>,
    current_effect: Option<MaterialEffectSource>,
    resource_stack: Vec<MaterialEffectResourceBuilder>,
    text_capture: Option<MaterialEffectTextCapture>,
    root_closed: bool,
}

impl MaterialEffectsParser {
    const fn new(source_path: String) -> Self {
        Self {
            source_path,
            source: None,
            current_effect: None,
            resource_stack: Vec::new(),
            text_capture: None,
            root_closed: false,
        }
    }

    fn parse(mut self, xml: &str) -> Result<MaterialEffectsSource, MaterialEffectsParseError> {
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
                    self.text(text)?;
                }
                Event::CData(event) => {
                    let text = xml_cdata_content(&event)?.into_owned();
                    self.text(text)?;
                }
                Event::Comment(event) => self.comment(event.as_ref()),
                Event::GeneralRef(event) => {
                    self.text(xml_general_reference_content(&event)?.into_owned())?;
                }
                Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if self.current_effect.is_some() {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: "Effect".to_string(),
            });
        }
        if let Some(resource) = self.resource_stack.last() {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: resource.element_name().to_string(),
            });
        }
        if let Some(capture) = self.text_capture {
            return Err(MaterialEffectsParseError::UnclosedElement {
                element: capture.element_name().to_string(),
            });
        }

        self.source
            .ok_or(MaterialEffectsParseError::MissingElement { element: "FXLib" })
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<(), MaterialEffectsParseError> {
        let name = element_name(event);
        let attributes = attributes(reader, event)?;

        match name.as_str() {
            "FXLib" => self.start_library(attributes),
            "Effect" => self.start_effect(attributes),
            "Audio" => self.push_resource(MaterialEffectResourceBuilder::Audio(parse_audio(
                attributes,
            )?)),
            "Switch" => self.add_audio_switch(attributes),
            "Particle" => self.push_resource(MaterialEffectResourceBuilder::Particle(
                parse_particle(attributes)?,
            )),
            "Name" => self.start_particle_name(attributes),
            "Direction" => self.start_text_capture(MaterialEffectTextCapture::ParticleDirection {
                text: String::new(),
            }),
            "Decal" => self.push_resource(MaterialEffectResourceBuilder::Decal(parse_decal(
                attributes,
            )?)),
            "Material" => self.start_text_capture(MaterialEffectTextCapture::DecalMaterial {
                text: String::new(),
            }),
            "ForceFeedback" => self.push_resource(MaterialEffectResourceBuilder::ForceFeedback(
                parse_force_feedback(attributes)?,
            )),
            "RandEffect" => self.push_resource(MaterialEffectResourceBuilder::Random(
                parse_random(attributes)?,
            )),
            _ => Err(MaterialEffectsParseError::UnknownElement { element: name }),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), MaterialEffectsParseError> {
        match name {
            "FXLib" => {
                if self.current_effect.is_some() {
                    return Err(MaterialEffectsParseError::UnclosedElement {
                        element: "Effect".to_string(),
                    });
                }
                self.root_closed = true;
                Ok(())
            }
            "Effect" => {
                if !self.resource_stack.is_empty() {
                    let element = self
                        .resource_stack
                        .last()
                        .unwrap()
                        .element_name()
                        .to_string();
                    return Err(MaterialEffectsParseError::UnclosedElement { element });
                }
                let effect = self.current_effect.take().ok_or_else(|| {
                    MaterialEffectsParseError::UnexpectedEndElement {
                        element: "Effect".to_string(),
                    }
                })?;
                self.library_mut()?.effects.push(effect);
                Ok(())
            }
            "Audio" | "Particle" | "Decal" | "ForceFeedback" | "RandEffect" => {
                let resource = self
                    .resource_stack
                    .pop()
                    .ok_or_else(|| MaterialEffectsParseError::UnexpectedEndElement {
                        element: name.to_string(),
                    })?
                    .finish();
                self.add_finished_resource(resource)
            }
            "Switch" => Ok(()),
            "Name" | "Direction" | "Material" => self.finish_text_capture(name),
            _ => Err(MaterialEffectsParseError::UnknownElement {
                element: name.to_string(),
            }),
        }
    }

    fn text(&mut self, text: String) -> Result<(), MaterialEffectsParseError> {
        if text.trim().is_empty() {
            return Ok(());
        }

        let Some(capture) = self.text_capture.as_mut() else {
            return Err(MaterialEffectsParseError::UnexpectedText { text });
        };
        capture.push_str(&text);
        Ok(())
    }

    fn comment(&mut self, bytes: &[u8]) {
        let comment = String::from_utf8_lossy(bytes).trim().to_string();
        if comment.is_empty() {
            return;
        }

        if let Some(effect) = self.current_effect.as_mut() {
            effect.comments.push(comment);
        } else if let Some(MaterialEffectsSource::Library(library)) = self.source.as_mut() {
            library.comments.push(comment);
        }
    }

    fn start_library(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialEffectsParseError> {
        if self.source.is_some() {
            return Err(MaterialEffectsParseError::DuplicateElement { element: "FXLib" });
        }

        let mut kind = None;
        for attribute in attributes {
            match attribute.name.as_str() {
                "type" => kind = Some(attribute.value),
                "xmlns:xsi" => {}
                _ => {
                    return Err(MaterialEffectsParseError::UnknownAttribute {
                        element: "FXLib",
                        attribute: attribute.name,
                    });
                }
            }
        }

        self.source = Some(MaterialEffectsSource::Library(
            MaterialEffectsLibrarySource {
                source_path: self.source_path.clone(),
                kind: kind.ok_or(MaterialEffectsParseError::MissingAttribute {
                    element: "FXLib",
                    attribute: "type",
                })?,
                effects: Vec::new(),
                comments: Vec::new(),
            },
        ));
        Ok(())
    }

    fn start_effect(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialEffectsParseError> {
        self.library_mut()?;
        if self.root_closed {
            return Err(MaterialEffectsParseError::ElementAfterRoot);
        }
        if self.current_effect.is_some() {
            return Err(MaterialEffectsParseError::NestedElement { element: "Effect" });
        }

        self.current_effect = Some(parse_effect(attributes)?);
        Ok(())
    }

    fn push_resource(
        &mut self,
        resource: MaterialEffectResourceBuilder,
    ) -> Result<(), MaterialEffectsParseError> {
        self.current_effect_mut()?;
        if self.root_closed {
            return Err(MaterialEffectsParseError::ElementAfterRoot);
        }
        self.resource_stack.push(resource);
        Ok(())
    }

    fn add_finished_resource(
        &mut self,
        resource: MaterialEffectResourceSource,
    ) -> Result<(), MaterialEffectsParseError> {
        if let Some(MaterialEffectResourceBuilder::Random(parent)) = self.resource_stack.last_mut()
        {
            parent.resources.push(resource);
            return Ok(());
        }

        self.current_effect_mut()?.resources.push(resource);
        Ok(())
    }

    fn add_audio_switch(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialEffectsParseError> {
        let switch = parse_audio_switch(attributes)?;
        match self.resource_stack.last_mut() {
            Some(MaterialEffectResourceBuilder::Audio(audio)) => {
                audio.switches.push(switch);
                Ok(())
            }
            _ => Err(MaterialEffectsParseError::ElementInWrongParent {
                element: "Switch",
                parent: "Audio",
            }),
        }
    }

    fn start_particle_name(
        &mut self,
        attributes: Vec<XmlAttribute>,
    ) -> Result<(), MaterialEffectsParseError> {
        match self.resource_stack.last() {
            Some(MaterialEffectResourceBuilder::Particle(_)) => self.start_text_capture(
                MaterialEffectTextCapture::ParticleName(parse_particle_name(attributes)?),
            ),
            _ => Err(MaterialEffectsParseError::ElementInWrongParent {
                element: "Name",
                parent: "Particle",
            }),
        }
    }

    fn start_text_capture(
        &mut self,
        capture: MaterialEffectTextCapture,
    ) -> Result<(), MaterialEffectsParseError> {
        if self.text_capture.is_some() {
            return Err(MaterialEffectsParseError::NestedTextElement {
                element: capture.element_name(),
            });
        }

        self.text_capture = Some(capture);
        Ok(())
    }

    fn finish_text_capture(&mut self, name: &str) -> Result<(), MaterialEffectsParseError> {
        let capture = self.text_capture.take().ok_or_else(|| {
            MaterialEffectsParseError::UnexpectedEndElement {
                element: name.to_string(),
            }
        })?;
        if capture.element_name() != name {
            return Err(MaterialEffectsParseError::UnexpectedEndElement {
                element: name.to_string(),
            });
        }

        match capture {
            MaterialEffectTextCapture::ParticleName(mut source) => {
                source.path = source.path.trim().to_string();
                match self.resource_stack.last_mut() {
                    Some(MaterialEffectResourceBuilder::Particle(particle)) => {
                        particle.names.push(source);
                        Ok(())
                    }
                    _ => Err(MaterialEffectsParseError::ElementInWrongParent {
                        element: "Name",
                        parent: "Particle",
                    }),
                }
            }
            MaterialEffectTextCapture::ParticleDirection { text } => {
                match self.resource_stack.last_mut() {
                    Some(MaterialEffectResourceBuilder::Particle(particle)) => {
                        particle.direction = Some(
                            MaterialEffectParticleDirectionSource::from_legacy(text.trim()),
                        );
                        Ok(())
                    }
                    _ => Err(MaterialEffectsParseError::ElementInWrongParent {
                        element: "Direction",
                        parent: "Particle",
                    }),
                }
            }
            MaterialEffectTextCapture::DecalMaterial { text } => {
                match self.resource_stack.last_mut() {
                    Some(MaterialEffectResourceBuilder::Decal(decal)) => {
                        decal.material = Some(text.trim().to_string());
                        Ok(())
                    }
                    _ => Err(MaterialEffectsParseError::ElementInWrongParent {
                        element: "Material",
                        parent: "Decal",
                    }),
                }
            }
        }
    }

    const fn library_mut(
        &mut self,
    ) -> Result<&mut MaterialEffectsLibrarySource, MaterialEffectsParseError> {
        match self.source.as_mut() {
            Some(MaterialEffectsSource::Library(library)) => Ok(library),
            _ => Err(MaterialEffectsParseError::MissingElement { element: "FXLib" }),
        }
    }

    fn current_effect_mut(
        &mut self,
    ) -> Result<&mut MaterialEffectSource, MaterialEffectsParseError> {
        self.current_effect
            .as_mut()
            .ok_or(MaterialEffectsParseError::ElementInWrongParent {
                element: "resource",
                parent: "Effect",
            })
    }
}

enum MaterialEffectResourceBuilder {
    Audio(MaterialEffectAudioSource),
    Particle(MaterialEffectParticleSource),
    Decal(MaterialEffectDecalSource),
    ForceFeedback(MaterialEffectForceFeedbackSource),
    Random(MaterialEffectRandomSource),
}

impl MaterialEffectResourceBuilder {
    const fn element_name(&self) -> &'static str {
        match self {
            Self::Audio(_) => "Audio",
            Self::Particle(_) => "Particle",
            Self::Decal(_) => "Decal",
            Self::ForceFeedback(_) => "ForceFeedback",
            Self::Random(_) => "RandEffect",
        }
    }

    fn finish(self) -> MaterialEffectResourceSource {
        match self {
            Self::Audio(source) => MaterialEffectResourceSource::Audio(source),
            Self::Particle(source) => MaterialEffectResourceSource::Particle(source),
            Self::Decal(source) => MaterialEffectResourceSource::Decal(source),
            Self::ForceFeedback(source) => MaterialEffectResourceSource::ForceFeedback(source),
            Self::Random(source) => MaterialEffectResourceSource::Random(source),
        }
    }
}

enum MaterialEffectTextCapture {
    ParticleName(MaterialEffectParticleNameSource),
    ParticleDirection { text: String },
    DecalMaterial { text: String },
}

impl MaterialEffectTextCapture {
    const fn element_name(&self) -> &'static str {
        match self {
            Self::ParticleName(_) => "Name",
            Self::ParticleDirection { .. } => "Direction",
            Self::DecalMaterial { .. } => "Material",
        }
    }

    fn push_str(&mut self, value: &str) {
        match self {
            Self::ParticleName(source) => source.path.push_str(value),
            Self::ParticleDirection { text } | Self::DecalMaterial { text } => {
                text.push_str(value);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlAttribute {
    name: String,
    value: String,
}

fn parse_effect(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectSource, MaterialEffectsParseError> {
    let mut name = None;
    let mut delay = None;
    let mut filter = MaterialEffectFilterSource::default();

    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            "delay" => delay = Some(parse_f32("Effect", "delay", &attribute.value)?),
            "GAME" => filter.game = Some(attribute.value),
            "DEVMODE" => {
                filter.devmode = Some(parse_i32("Effect", "DEVMODE", &attribute.value)?);
            }
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "Effect",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialEffectSource {
        name: name.ok_or(MaterialEffectsParseError::MissingAttribute {
            element: "Effect",
            attribute: "name",
        })?,
        delay,
        filter,
        resources: Vec::new(),
        comments: Vec::new(),
    })
}

fn parse_audio(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectAudioSource, MaterialEffectsParseError> {
    let mut trigger = None;
    let mut filter = MaterialEffectFilterSource::default();
    for attribute in attributes {
        match attribute.name.as_str() {
            "trigger" => trigger = Some(attribute.value),
            "GAME" => filter.game = Some(attribute.value),
            "DEVMODE" => {
                filter.devmode = Some(parse_i32("Audio", "DEVMODE", &attribute.value)?);
            }
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "Audio",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialEffectAudioSource {
        trigger: trigger.ok_or(MaterialEffectsParseError::MissingAttribute {
            element: "Audio",
            attribute: "trigger",
        })?,
        filter,
        switches: Vec::new(),
    })
}

fn parse_audio_switch(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectAudioSwitchSource, MaterialEffectsParseError> {
    let mut name = None;
    let mut state = None;
    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            "state" => state = Some(attribute.value),
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "Switch",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialEffectAudioSwitchSource {
        name: name.ok_or(MaterialEffectsParseError::MissingAttribute {
            element: "Switch",
            attribute: "name",
        })?,
        state: state.ok_or(MaterialEffectsParseError::MissingAttribute {
            element: "Switch",
            attribute: "state",
        })?,
    })
}

fn parse_particle(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectParticleSource, MaterialEffectsParseError> {
    let mut filter = MaterialEffectFilterSource::default();
    for attribute in attributes {
        match attribute.name.as_str() {
            "GAME" => filter.game = Some(attribute.value),
            "DEVMODE" => {
                filter.devmode = Some(parse_i32("Particle", "DEVMODE", &attribute.value)?);
            }
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "Particle",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialEffectParticleSource {
        filter,
        names: Vec::new(),
        direction: None,
    })
}

fn parse_particle_name(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectParticleNameSource, MaterialEffectsParseError> {
    let mut source = MaterialEffectParticleNameSource {
        path: String::new(),
        direction: None,
        user_data: None,
        scale: None,
        max_distance: None,
        min_scale: None,
        max_scale: None,
        max_scale_distance: None,
        attach: None,
    };

    for attribute in attributes {
        match attribute.name.as_str() {
            "direction" => source.direction = Some(attribute.value),
            "userdata" => source.user_data = Some(attribute.value),
            "scale" => source.scale = Some(parse_f32("Name", "scale", &attribute.value)?),
            "maxdist" => {
                source.max_distance = Some(parse_f32("Name", "maxdist", &attribute.value)?);
            }
            "minscale" => {
                source.min_scale = Some(parse_f32("Name", "minscale", &attribute.value)?);
            }
            "maxscale" => {
                source.max_scale = Some(parse_f32("Name", "maxscale", &attribute.value)?);
            }
            "maxscaledist" => {
                source.max_scale_distance =
                    Some(parse_f32("Name", "maxscaledist", &attribute.value)?);
            }
            "attach" => source.attach = Some(parse_bool("Name", "attach", &attribute.value)?),
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "Name",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(source)
}

fn parse_decal(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectDecalSource, MaterialEffectsParseError> {
    let mut source = MaterialEffectDecalSource {
        filter: MaterialEffectFilterSource::default(),
        material: None,
        min_scale: None,
        max_scale: None,
        rotation_degrees: None,
        grow_time: None,
        assemble_decals: None,
        force_edge: None,
        lifetime: None,
    };

    for attribute in attributes {
        match attribute.name.as_str() {
            "minscale" => {
                source.min_scale = Some(parse_f32("Decal", "minscale", &attribute.value)?);
            }
            "maxscale" => {
                source.max_scale = Some(parse_f32("Decal", "maxscale", &attribute.value)?);
            }
            "rotation" => {
                source.rotation_degrees = Some(parse_f32("Decal", "rotation", &attribute.value)?);
            }
            "growTime" => {
                source.grow_time = Some(parse_f32("Decal", "growTime", &attribute.value)?);
            }
            "assembledecals" => {
                source.assemble_decals =
                    Some(parse_bool("Decal", "assembledecals", &attribute.value)?);
            }
            "forceedge" => {
                source.force_edge = Some(parse_bool("Decal", "forceedge", &attribute.value)?);
            }
            "lifetime" => {
                source.lifetime = Some(parse_f32("Decal", "lifetime", &attribute.value)?);
            }
            "GAME" => source.filter.game = Some(attribute.value),
            "DEVMODE" => {
                source.filter.devmode = Some(parse_i32("Decal", "DEVMODE", &attribute.value)?);
            }
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "Decal",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(source)
}

fn parse_force_feedback(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectForceFeedbackSource, MaterialEffectsParseError> {
    let mut name = None;
    let mut filter = MaterialEffectFilterSource::default();
    let mut min_falloff_distance = None;
    let mut max_falloff_distance = None;

    for attribute in attributes {
        match attribute.name.as_str() {
            "name" => name = Some(attribute.value),
            "minFallOffDistance" => {
                min_falloff_distance = Some(parse_f32(
                    "ForceFeedback",
                    "minFallOffDistance",
                    &attribute.value,
                )?);
            }
            "maxFallOffDistance" => {
                max_falloff_distance = Some(parse_f32(
                    "ForceFeedback",
                    "maxFallOffDistance",
                    &attribute.value,
                )?);
            }
            "GAME" => filter.game = Some(attribute.value),
            "DEVMODE" => {
                filter.devmode = Some(parse_i32("ForceFeedback", "DEVMODE", &attribute.value)?);
            }
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "ForceFeedback",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialEffectForceFeedbackSource {
        name: name.ok_or(MaterialEffectsParseError::MissingAttribute {
            element: "ForceFeedback",
            attribute: "name",
        })?,
        filter,
        min_falloff_distance,
        max_falloff_distance,
    })
}

fn parse_random(
    attributes: Vec<XmlAttribute>,
) -> Result<MaterialEffectRandomSource, MaterialEffectsParseError> {
    let mut filter = MaterialEffectFilterSource::default();
    for attribute in attributes {
        match attribute.name.as_str() {
            "GAME" => filter.game = Some(attribute.value),
            "DEVMODE" => {
                filter.devmode = Some(parse_i32("RandEffect", "DEVMODE", &attribute.value)?);
            }
            _ => {
                return Err(MaterialEffectsParseError::UnknownAttribute {
                    element: "RandEffect",
                    attribute: attribute.name,
                });
            }
        }
    }

    Ok(MaterialEffectRandomSource {
        filter,
        resources: Vec::new(),
    })
}

fn attributes(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<Vec<XmlAttribute>, MaterialEffectsParseError> {
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

fn parse_i32(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<i32, MaterialEffectsParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| MaterialEffectsParseError::InvalidInteger {
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
) -> Result<f32, MaterialEffectsParseError> {
    value
        .trim()
        .parse()
        .map_err(|source| MaterialEffectsParseError::InvalidFloat {
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
) -> Result<bool, MaterialEffectsParseError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => Err(MaterialEffectsParseError::InvalidBool {
            element,
            attribute,
            value: value.to_string(),
        }),
    }
}
