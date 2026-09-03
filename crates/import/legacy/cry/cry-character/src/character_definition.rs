use std::{collections::BTreeMap, fmt::Display, str, str::FromStr, sync::Arc};

use az_animation::character::definition::{
    AttachmentBinding, AttachmentFlags, AttachmentMaterials, AttachmentTransform, BoneAttachment,
    CharacterAttachment, CharacterAttachmentKind, CharacterDefinition, CharacterDefinitionSource,
    CharacterMirroring, ClothAttachment, ClothCollisionAttachment, ClothJointPhysics,
    FaceAttachment, JointPhysics, MirroringAxis, PendulumRowAttachment, ProxyAttachment,
    RelativeAttachmentTransform, RopeJointPhysics, RowConstraint, RowSimulation, SkinAttachment,
    SocketConstraint, SocketSimulation,
};
use az_core::{AssetPathBuf, AssetPathError};
use bevy_math::{Quat, Vec2, Vec3, Vec4};
use quick_xml::{
    Reader,
    events::{BytesStart, Event, attributes::AttrError},
};

use crate::{xml_cdata_content, xml_general_reference_content, xml_text_content};

use super::to_ron_bytes;

pub trait CharacterDefinitionSourceExt: Sized {
    /// Parses a legacy Cry `.cdf` document into the authoring source model.
    ///
    /// # Errors
    ///
    /// Returns [`CharacterDefinitionParseError::UnsupportedPath`] when
    /// `source_path` does not name a `.cdf` document,
    /// [`CharacterDefinitionParseError::InvalidUtf8`] when `bytes` is not
    /// UTF-8, [`CharacterDefinitionParseError::Xml`] for malformed XML, and
    /// the element/attachment/attribute variants — for example
    /// [`CharacterDefinitionParseError::MissingElement`] for a document with
    /// no `<Model>`, or
    /// [`CharacterDefinitionParseError::UnknownAttachmentType`] for an
    /// attachment `Type` the reader does not map.
    fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, CharacterDefinitionParseError>;

    /// Serializes the authoring source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error>;
}

impl CharacterDefinitionSourceExt for CharacterDefinitionSource {
    fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, CharacterDefinitionParseError> {
        let normalized_source_path = az_asset_builder::normalize_source_path(source_path);
        if !is_character_definition_source_path(&normalized_source_path) {
            return Err(CharacterDefinitionParseError::UnsupportedPath {
                path: normalized_source_path,
            });
        }

        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let xml = str::from_utf8(bytes).map_err(CharacterDefinitionParseError::InvalidUtf8)?;
        CharacterDefinitionParser::default().parse(xml)
    }

    fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        to_ron_bytes(self)
    }
}

#[must_use]
pub fn is_character_definition_source_path(normalized_source_path: &str) -> bool {
    std::path::Path::new(normalized_source_path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cdf"))
}

/// Where the reader currently sits relative to the single
/// `<CharacterDefinition>` root.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum RootPosition {
    #[default]
    Before,
    Inside,
    After,
}

/// Where the reader currently sits inside the `<Modifiers>` subtree.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ModifierPosition {
    #[default]
    Outside,
    InModifiers,
    InInstance,
}

#[derive(Default)]
struct CharacterDefinitionParser {
    model: Option<AssetPathBuf>,
    parameters: Option<AssetPathBuf>,
    material: Option<AssetPathBuf>,
    keep_models_in_memory: bool,
    mirroring: CharacterMirroring,
    attachments: Vec<CharacterAttachment<AssetPathBuf>>,
    root: RootPosition,
    modifiers: ModifierPosition,
}

impl CharacterDefinitionParser {
    fn parse(
        mut self,
        xml: &str,
    ) -> Result<CharacterDefinitionSource, CharacterDefinitionParseError> {
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
                Event::Text(event) => self.text(&xml_text_content(&event)?)?,
                Event::CData(event) => self.text(&xml_cdata_content(&event)?)?,
                Event::GeneralRef(event) => self.text(&xml_general_reference_content(&event)?)?,
                Event::Comment(_) | Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
                Event::Eof => break,
            }
        }

        if self.root != RootPosition::After {
            return Err(CharacterDefinitionParseError::MissingElement {
                element: "CharacterDefinition",
            });
        }
        let model = self
            .model
            .ok_or(CharacterDefinitionParseError::MissingElement { element: "Model" })?;
        Ok(CharacterDefinition {
            model,
            parameters: self.parameters,
            material: self.material,
            keep_models_in_memory: self.keep_models_in_memory,
            mirroring: self.mirroring,
            attachments: self.attachments,
        })
    }

    fn start_element(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        empty: bool,
    ) -> Result<(), CharacterDefinitionParseError> {
        let name = element_name(event);
        let attributes = LegacyAttributes::read(reader, event)?;

        match self.modifiers {
            ModifierPosition::InInstance => {
                return Err(CharacterDefinitionParseError::UnsupportedPoseModifierData {
                    element: name,
                });
            }
            ModifierPosition::InModifiers => {
                return self.start_modifier_element(name, attributes, empty);
            }
            ModifierPosition::Outside => {}
        }

        match name.as_str() {
            "CharacterDefinition" if self.root == RootPosition::Before => {
                attributes.finish("CharacterDefinition")?;
                self.root = if empty {
                    RootPosition::After
                } else {
                    RootPosition::Inside
                };
                Ok(())
            }
            _ if self.root != RootPosition::Inside => {
                Err(CharacterDefinitionParseError::ElementOutsideRoot { element: name })
            }
            "Model" => self.parse_model(attributes),
            "Mirroring" => self.parse_mirroring(attributes),
            "AttachmentList" => attributes.finish("AttachmentList"),
            "Attachment" => {
                self.attachments.push(parse_attachment(attributes)?);
                Ok(())
            }
            "Modifiers" => {
                attributes.finish("Modifiers")?;
                if !empty {
                    self.modifiers = ModifierPosition::InModifiers;
                }
                Ok(())
            }
            _ => Err(CharacterDefinitionParseError::UnexpectedElement { element: name }),
        }
    }

    fn start_modifier_element(
        &mut self,
        name: String,
        mut attributes: LegacyAttributes,
        empty: bool,
    ) -> Result<(), CharacterDefinitionParseError> {
        match name.as_str() {
            "Element" => attributes.finish("Element"),
            "enabled" => {
                let _ = attributes.take_bool("value")?;
                attributes.finish("enabled")
            }
            "instance" => {
                attributes.finish("instance")?;
                if !empty {
                    self.modifiers = ModifierPosition::InInstance;
                }
                Ok(())
            }
            _ => Err(CharacterDefinitionParseError::UnsupportedPoseModifierData { element: name }),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), CharacterDefinitionParseError> {
        match name {
            "CharacterDefinition" if self.root == RootPosition::Inside => {
                self.root = RootPosition::After;
            }
            "Modifiers" if self.modifiers == ModifierPosition::InModifiers => {
                self.modifiers = ModifierPosition::Outside;
            }
            "instance" if self.modifiers == ModifierPosition::InInstance => {
                self.modifiers = ModifierPosition::InModifiers;
            }
            "Model" | "Mirroring" | "AttachmentList" | "Attachment" | "Element" | "enabled" => {}
            _ => {
                return Err(CharacterDefinitionParseError::UnexpectedEnd {
                    element: name.to_string(),
                });
            }
        }
        Ok(())
    }

    fn text(&self, text: &str) -> Result<(), CharacterDefinitionParseError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        if self.modifiers == ModifierPosition::InInstance {
            return Err(CharacterDefinitionParseError::UnsupportedPoseModifierData {
                element: "instance text".to_owned(),
            });
        }
        Err(CharacterDefinitionParseError::UnexpectedText {
            text: text.to_owned(),
        })
    }

    fn parse_model(
        &mut self,
        mut attributes: LegacyAttributes,
    ) -> Result<(), CharacterDefinitionParseError> {
        if self.model.is_some() {
            return Err(CharacterDefinitionParseError::DuplicateElement { element: "Model" });
        }
        let file = attributes.require("File", "Model")?;
        self.model = Some(canonical_model_path(&file)?);
        self.parameters = attributes
            .take("ParamsOverride")
            .filter(|value| !value.is_empty())
            .map(|value| canonical_character_parameters_path(&value))
            .transpose()?;
        self.material = attributes
            .take("Material")
            .filter(|value| !value.is_empty())
            .map(|value| canonical_material_path(&value))
            .transpose()?;
        self.keep_models_in_memory = attributes.take_bool("KeepModelsInMemory")?.unwrap_or(false);

        // These older authoring hints are not consumed by the current loader.
        attributes.discard("Physics");
        attributes.discard("Rig");
        attributes.discard("Rig File");
        attributes.finish("Model")
    }

    fn parse_mirroring(
        &mut self,
        mut attributes: LegacyAttributes,
    ) -> Result<(), CharacterDefinitionParseError> {
        self.mirroring.axis = match attributes.take("Axis").as_deref() {
            Some("X") => Some(MirroringAxis::X),
            Some("Y") => Some(MirroringAxis::Y),
            Some("Z") => Some(MirroringAxis::Z),
            Some(_) | None => None,
        };
        self.mirroring.enabled = attributes.take_bool("Enabled")?.unwrap_or(false);
        attributes.finish("Mirroring")
    }
}

fn parse_attachment(
    mut attributes: LegacyAttributes,
) -> Result<CharacterAttachment<AssetPathBuf>, CharacterDefinitionParseError> {
    let attachment_type = attributes.require("Type", "Attachment")?;
    let name = attributes.take_arc("AName");
    let joint = attributes.take_arc("BoneName");
    let row_joint = attributes
        .take_arc("RowJointName")
        .or_else(|| attributes.take_arc("rowJointName"));
    let binding = attributes
        .take("Binding")
        .filter(|value| !value.is_empty())
        .map(|value| canonical_attachment_binding(&value))
        .transpose()?
        .flatten();
    let flags = AttachmentFlags::from_bits_retain(attributes.take_parse("Flags")?.unwrap_or(0));
    let attachment_name = name.as_deref();
    let absolute = AttachmentTransform {
        rotation: attributes
            .take_native_optional_quat("Rotation", attachment_name)
            .unwrap_or(Quat::IDENTITY),
        translation: attributes
            .take_native_optional_vec3("Position", attachment_name)
            .unwrap_or(Vec3::ZERO),
    };
    let relative = RelativeAttachmentTransform {
        rotation: attributes.take_native_optional_quat("RelRotation", attachment_name),
        translation: attributes.take_native_optional_vec3("RelPosition", attachment_name),
    };
    let proxy_parameters = attributes.take_vec4("ProxyParams")?.unwrap_or(Vec4::ZERO);
    let proxy_purpose = attributes.take_parse("ProxyPurpose")?.unwrap_or(0);
    let materials = parse_materials(&mut attributes)?;

    let kind = match attachment_type.as_str() {
        "CA_BONE" => CharacterAttachmentKind::Bone(BoneAttachment {
            joint: joint.clone(),
            simulation: parse_socket_simulation(&mut attributes, joint.as_deref())?,
            procedural_function: attributes.take_arc("ProcFunction"),
            physics_lods: parse_joint_physics(&mut attributes)?,
        }),
        "CA_FACE" => CharacterAttachmentKind::Face(FaceAttachment),
        "CA_SKIN" => CharacterAttachmentKind::Skin(SkinAttachment),
        "CA_PROX" => CharacterAttachmentKind::Proxy(ProxyAttachment {
            joint,
            parameters: proxy_parameters,
            purpose: proxy_purpose,
        }),
        "CA_PROW" => CharacterAttachmentKind::PendulumRow(PendulumRowAttachment {
            row_joint,
            simulation: parse_row_simulation(&mut attributes)?,
        }),
        "CA_CLOTH" => CharacterAttachmentKind::Cloth(ClothAttachment {
            hidden: attributes.take_bool("hide")?.unwrap_or(false),
            collision_layer_mask: attributes.take_parse("CollisionLayerMask")?.unwrap_or(1),
            max_simulation_distance: attributes
                .take_parse("MaxSimulationDistance")?
                .unwrap_or(3.0),
            local_wind: attributes
                .take_vec3("LocalWindVector")?
                .unwrap_or(Vec3::ZERO),
        }),
        "CA_CLOTH_COLLISION" => CharacterAttachmentKind::ClothCollision(ClothCollisionAttachment {
            joint,
            parameters: proxy_parameters,
            collision_layer: attributes.take_parse("CollisionLayer")?.unwrap_or(0),
        }),
        "CA_VCLOTH" => {
            return Err(CharacterDefinitionParseError::UnsupportedAttachmentType {
                value: attachment_type,
            });
        }
        _ => {
            return Err(CharacterDefinitionParseError::UnknownAttachmentType {
                value: attachment_type,
            });
        }
    };

    // Editor identity/distance fields and the unused row translation axis are
    // deliberately absent from the runtime product.
    attributes.discard("SerialNumber");
    attributes.discard("ViewDistRatio");
    attributes.discard("ROW_TranslationAxis");
    attributes.finish("Attachment")?;

    Ok(CharacterAttachment {
        name,
        flags,
        absolute,
        relative,
        binding,
        materials,
        kind,
    })
}

fn parse_materials(
    attributes: &mut LegacyAttributes,
) -> Result<AttachmentMaterials<AssetPathBuf>, CharacterDefinitionParseError> {
    let shared = attributes
        .take("Material")
        .filter(|value| !value.is_empty())
        .map(|value| canonical_material_path(&value))
        .transpose()?;
    let mut lods = std::array::from_fn(|_| None);
    for (index, slot) in lods.iter_mut().enumerate() {
        let name = format!("MaterialLOD{index}");
        *slot = attributes
            .take(&name)
            .filter(|value| !value.is_empty())
            .map(|value| canonical_material_path(&value))
            .transpose()?;
    }
    Ok(AttachmentMaterials { shared, lods })
}

fn parse_socket_simulation(
    attributes: &mut LegacyAttributes,
    joint: Option<&str>,
) -> Result<Option<SocketSimulation>, CharacterDefinitionParseError> {
    let mut simulation = SocketSimulation::default();

    // Each block consumes its own `PA_`/`SA_`/`P_` attribute family in the
    // native order, so all three run even when an earlier one already
    // authored the socket.
    let pendulum = parse_pendulum_socket(attributes, joint, &mut simulation)?;
    let spring = parse_spring_socket(attributes, &mut simulation)?;
    let projection = parse_projection_socket(attributes, joint, &mut simulation)?;

    Ok((pendulum || spring || projection).then_some(simulation))
}

/// Reads the `PA_*` pendulum family; reports whether it authored the socket.
fn parse_pendulum_socket(
    attributes: &mut LegacyAttributes,
    joint: Option<&str>,
    simulation: &mut SocketSimulation,
) -> Result<bool, CharacterDefinitionParseError> {
    let mut authored = false;

    if let Some(kind) = attributes.take_parse::<u32>("PA_PendulumType")? {
        simulation.constraint = match kind {
            1 => SocketConstraint::PendulumCone,
            2 => SocketConstraint::PendulumHingePlane,
            3 => SocketConstraint::PendulumHalfCone,
            _ => SocketConstraint::Disabled,
        };
        if simulation.constraint != SocketConstraint::Disabled {
            authored = true;
            overwrite_u8(attributes, "PA_FPS", &mut simulation.frames_per_second)?;
            overwrite_bool(attributes, "PA_Redirect", &mut simulation.redirect)?;
            overwrite_f32(attributes, "PA_MaxAngle", &mut simulation.max_angle_degrees)?;
            overwrite_vec2_x(
                attributes,
                "PA_HRotation",
                &mut simulation.disk_rotation_degrees,
            )?;
            overwrite_f32(attributes, "PA_Mass", &mut simulation.mass)?;
            overwrite_f32(attributes, "PA_Gravity", &mut simulation.gravity)?;
            overwrite_f32(attributes, "PA_Damping", &mut simulation.damping)?;
            overwrite_f32(attributes, "PA_Stiffness", &mut simulation.stiffness)?;
            overwrite_vec3(attributes, "PA_PivotOffset", &mut simulation.pivot_offset)?;
            overwrite_vec3(
                attributes,
                "PA_PendulumOffset",
                &mut simulation.simulation_axis,
            )?;
            overwrite_vec3(
                attributes,
                "PA_SimulationAxis",
                &mut simulation.simulation_axis,
            )?;
            overwrite_vec3(
                attributes,
                "PA_StiffnessTarget",
                &mut simulation.stiffness_target,
            )?;
            overwrite_vec2_x(attributes, "PA_CapsuleX", &mut simulation.capsule)?;
            overwrite_vec2_y(attributes, "PA_CapsuleY", &mut simulation.capsule)?;
            overwrite_i32(
                attributes,
                "PA_ProjectionType",
                &mut simulation.projection_type,
            )?;
            simulation.directional_translation_joint =
                normalized_directional_joint(attributes.take_arc("PA_DirTransJointName"), joint);
            append_proxies(attributes, "PA_Proxy", &mut simulation.collision_proxies);
        }
    }

    Ok(authored)
}

/// Reads the `SA_*` spring-ellipsoid family; reports whether it authored the
/// socket.
fn parse_spring_socket(
    attributes: &mut LegacyAttributes,
    simulation: &mut SocketSimulation,
) -> Result<bool, CharacterDefinitionParseError> {
    let mut authored = false;

    if attributes
        .take_parse::<u32>("SA_SpringType")?
        .is_some_and(|value| value != 0)
    {
        authored = true;
        simulation.constraint = SocketConstraint::SpringEllipsoid;
        overwrite_u8(attributes, "SA_FPS", &mut simulation.frames_per_second)?;
        overwrite_f32(attributes, "SA_Radius", &mut simulation.radius)?;
        overwrite_vec2_x(attributes, "SA_ScaleZP", &mut simulation.sphere_scale)?;
        overwrite_vec2_y(attributes, "SA_ScaleZN", &mut simulation.sphere_scale)?;
        overwrite_vec2_x(
            attributes,
            "SA_DiskRotX",
            &mut simulation.disk_rotation_degrees,
        )?;
        overwrite_vec2_y(
            attributes,
            "SA_DiskRotZ",
            &mut simulation.disk_rotation_degrees,
        )?;
        overwrite_vec2_x(
            attributes,
            "SA_HRotation",
            &mut simulation.disk_rotation_degrees,
        )?;
        overwrite_bool(attributes, "SA_Redirect", &mut simulation.redirect)?;
        overwrite_f32(attributes, "SA_Mass", &mut simulation.mass)?;
        overwrite_f32(attributes, "SA_Gravity", &mut simulation.gravity)?;
        overwrite_f32(attributes, "SA_Damping", &mut simulation.damping)?;
        overwrite_f32(attributes, "SA_Stiffness", &mut simulation.stiffness)?;
        overwrite_vec3(attributes, "SA_PivotOffset", &mut simulation.pivot_offset)?;
        overwrite_vec3(
            attributes,
            "SA_StiffnessTarget",
            &mut simulation.stiffness_target,
        )?;
        simulation.capsule.x = 0.0;
        overwrite_vec2_y(attributes, "SA_CapsuleY", &mut simulation.capsule)?;
        overwrite_i32(
            attributes,
            "SA_ProjectionType",
            &mut simulation.projection_type,
        )?;
        append_proxies(attributes, "SA_Proxy", &mut simulation.collision_proxies);
    }

    Ok(authored)
}

/// Reads the `P_*` translational-projection family; reports whether it
/// authored the socket.
fn parse_projection_socket(
    attributes: &mut LegacyAttributes,
    joint: Option<&str>,
    simulation: &mut SocketSimulation,
) -> Result<bool, CharacterDefinitionParseError> {
    let mut authored = false;

    if attributes
        .take_parse::<u32>("P_Projection")?
        .is_some_and(|value| value != 0)
    {
        authored = true;
        simulation.constraint = SocketConstraint::TranslationalProjection;
        simulation.redirect = true;
        overwrite_i32(
            attributes,
            "P_ProjectionType",
            &mut simulation.projection_type,
        )?;
        simulation.directional_translation_joint =
            normalized_directional_joint(attributes.take_arc("P_DirTransJointName"), joint);
        overwrite_vec3(
            attributes,
            "P_TranslationAxis",
            &mut simulation.simulation_axis,
        )?;
        overwrite_vec2_x(attributes, "P_CapsuleX", &mut simulation.capsule)?;
        overwrite_vec2_y(attributes, "P_CapsuleY", &mut simulation.capsule)?;
        overwrite_vec3(attributes, "P_PivotOffset", &mut simulation.pivot_offset)?;
        append_proxies(attributes, "P_Proxy", &mut simulation.collision_proxies);
    }

    Ok(authored)
}

fn parse_row_simulation(
    attributes: &mut LegacyAttributes,
) -> Result<Option<RowSimulation>, CharacterDefinitionParseError> {
    let Some(constraint) = attributes.take_parse::<u32>("ROW_ClampMode")? else {
        return Ok(None);
    };
    let mut simulation = RowSimulation {
        constraint: match constraint {
            0 => RowConstraint::PendulumCone,
            1 => RowConstraint::PendulumHingePlane,
            2 => RowConstraint::PendulumHalfCone,
            3 => RowConstraint::TranslationalProjection,
            value => {
                return Err(CharacterDefinitionParseError::InvalidEnumDiscriminant {
                    attribute: "ROW_ClampMode".to_owned(),
                    value,
                });
            }
        },
        ..RowSimulation::default()
    };
    overwrite_u8(attributes, "ROW_FPS", &mut simulation.frames_per_second)?;
    overwrite_f32(
        attributes,
        "ROW_ConeAngle",
        &mut simulation.cone_angle_degrees,
    )?;
    overwrite_vec3(
        attributes,
        "ROW_ConeRotation",
        &mut simulation.cone_rotation_degrees,
    )?;
    overwrite_f32(attributes, "ROW_Mass", &mut simulation.mass)?;
    overwrite_f32(attributes, "ROW_Gravity", &mut simulation.gravity)?;
    overwrite_f32(attributes, "ROW_Damping", &mut simulation.damping)?;
    overwrite_f32(attributes, "ROW_JointSpring", &mut simulation.joint_spring)?;
    overwrite_f32(attributes, "ROW_RodLength", &mut simulation.rod_length)?;
    overwrite_vec2(
        attributes,
        "ROW_StiffnessTarget",
        &mut simulation.stiffness_target,
    )?;
    overwrite_vec2(attributes, "ROW_Turbulence", &mut simulation.turbulence)?;
    overwrite_f32(attributes, "ROW_MaxVelocity", &mut simulation.max_velocity)?;
    overwrite_vec3(
        attributes,
        "ROW_WorldSpaceDamping",
        &mut simulation.world_space_damping,
    )?;
    overwrite_bool(attributes, "ROW_Cycle", &mut simulation.cycle)?;
    overwrite_u32(
        attributes,
        "ROW_RelaxLoops",
        &mut simulation.relaxation_loops,
    )?;
    overwrite_f32(attributes, "ROW_Stretch", &mut simulation.stretch)?;
    overwrite_vec2_x(attributes, "ROW_CapsuleX", &mut simulation.capsule)?;
    overwrite_vec2_y(attributes, "ROW_CapsuleY", &mut simulation.capsule)?;
    overwrite_i32(
        attributes,
        "ROW_ProjectionType",
        &mut simulation.projection_type,
    )?;
    append_proxies(attributes, "ROW_Proxy", &mut simulation.collision_proxies);
    Ok(Some(simulation))
}

fn parse_joint_physics(
    attributes: &mut LegacyAttributes,
) -> Result<[Option<JointPhysics>; 2], CharacterDefinitionParseError> {
    let Some(kind) = attributes.take("PhysPropType") else {
        return Ok([None, None]);
    };
    let mut lods = [None, None];
    for (lod, slot) in lods.iter_mut().enumerate() {
        *slot = match kind.as_str() {
            "Rope" => parse_rope_physics_lod(attributes, lod)?.map(JointPhysics::Rope),
            "Cloth" => parse_cloth_physics_lod(attributes, lod)?.map(JointPhysics::Cloth),
            _ => None,
        };
    }
    Ok(lods)
}

fn parse_rope_physics_lod(
    attributes: &mut LegacyAttributes,
    lod: usize,
) -> Result<Option<RopeJointPhysics>, CharacterDefinitionParseError> {
    let mut physics = RopeJointPhysics::default();
    let mut used = false;
    macro_rules! field {
        ($suffix:literal, $target:expr) => {
            if let Some(value) = attributes.take_parse(&format!("lod{lod}_{}", $suffix))? {
                $target = value;
                used = true;
            }
        };
    }
    macro_rules! bool_field {
        ($suffix:literal, $target:expr) => {
            if let Some(value) = attributes.take_bool(&format!("lod{lod}_{}", $suffix))? {
                $target = value;
                used = true;
            }
        };
    }
    field!("Gravity", physics.gravity);
    field!("JointLimit", physics.joint_limit_degrees);
    field!("JointLimitIncrease", physics.joint_limit_increase);
    field!("MaxTimestep", physics.max_timestep);
    field!("Stiffness", physics.stiffness_degrees);
    field!("StiffnessDecay", physics.stiffness_decay_degrees);
    field!("Damping", physics.damping_degrees);
    field!("Friction", physics.friction);
    bool_field!("SimpleBlending", physics.simple_blending);
    field!("Mass", physics.mass);
    field!("Thickness", physics.thickness);
    bool_field!("HingeY", physics.hinge_y);
    bool_field!("HingeZ", physics.hinge_z);
    field!("StiffnessControlBone", physics.stiffness_control_bone);
    bool_field!("EnvCollisions", physics.environment_collisions);
    bool_field!("BodyCollisions", physics.body_collisions);
    Ok(used.then_some(physics))
}

fn parse_cloth_physics_lod(
    attributes: &mut LegacyAttributes,
    lod: usize,
) -> Result<Option<ClothJointPhysics>, CharacterDefinitionParseError> {
    let mut physics = ClothJointPhysics::default();
    let mut used = false;
    macro_rules! field {
        ($suffix:literal, $target:expr) => {
            if let Some(value) = attributes.take_parse(&format!("lod{lod}_{}", $suffix))? {
                $target = value;
                used = true;
            }
        };
    }
    macro_rules! bool_field {
        ($suffix:literal, $target:expr) => {
            if let Some(value) = attributes.take_bool(&format!("lod{lod}_{}", $suffix))? {
                $target = value;
                used = true;
            }
        };
    }
    field!("MaxTimestep", physics.max_timestep);
    field!("MaxStretch", physics.max_stretch);
    field!("Stiffness", physics.stiffness_degrees);
    field!("Thickness", physics.thickness);
    field!("Friction", physics.friction);
    field!("StiffnessNorm", physics.normal_stiffness_degrees);
    field!("StiffnessTang", physics.tangential_stiffness_degrees);
    field!("Damping", physics.damping);
    field!("AirResistance", physics.air_resistance);
    field!("StiffnessAnim", physics.animation_stiffness);
    field!("StiffnessDecayAnim", physics.animation_stiffness_decay);
    field!("DampingAnim", physics.animation_damping);
    if let Some(value) = attributes.take_parse::<f32>(&format!("lod{lod}_MaxIters"))? {
        physics.max_iterations = rounded_iteration_count(value);
        used = true;
    }
    field!("MaxDistAnim", physics.max_animation_distance);
    field!("CharacterSpace", physics.character_space);
    bool_field!("EnvCollisions", physics.environment_collisions);
    bool_field!("BodyCollisions", physics.body_collisions);
    Ok(used.then_some(physics))
}

/// Rounds a native float iteration count into the engine's `u32` field.
///
/// Legacy `.cdf` writes `lodN_MaxIters` as a float (`"4.000000"`). Clamping to
/// `0.0` maps negative and NaN values to zero. Rust's float-to-integer cast
/// saturates values above `u32::MAX` instead of wrapping.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the rounded count intentionally uses Rust's saturating float-to-integer cast"
)]
const fn rounded_iteration_count(value: f32) -> u32 {
    value.max(0.0).round() as u32
}

fn normalized_directional_joint(
    value: Option<Arc<str>>,
    attachment_joint: Option<&str>,
) -> Option<Arc<str>> {
    value.filter(|value| attachment_joint.is_none_or(|joint| !value.eq_ignore_ascii_case(joint)))
}

fn append_proxies(
    attributes: &mut LegacyAttributes,
    prefix: &str,
    output: &mut smallvec::SmallVec<[Arc<str>; 10]>,
) {
    for index in 0..10 {
        if let Some(proxy) = attributes.take_arc(&format!("{prefix}{index:02}")) {
            output.push(proxy);
        }
    }
}

fn overwrite_f32(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut f32,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_parse(name)? {
        *target = value;
    }
    Ok(())
}

fn overwrite_i32(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut i32,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_parse(name)? {
        *target = value;
    }
    Ok(())
}

fn overwrite_u32(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut u32,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_parse(name)? {
        *target = value;
    }
    Ok(())
}

fn overwrite_u8(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut u8,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_parse(name)? {
        *target = value;
    }
    Ok(())
}

fn overwrite_bool(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut bool,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_bool(name)? {
        *target = value;
    }
    Ok(())
}

fn overwrite_vec2(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut Vec2,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_vec2(name)? {
        *target = value;
    }
    Ok(())
}

fn overwrite_vec2_x(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut Vec2,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_parse(name)? {
        target.x = value;
    }
    Ok(())
}

fn overwrite_vec2_y(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut Vec2,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_parse(name)? {
        target.y = value;
    }
    Ok(())
}

fn overwrite_vec3(
    attributes: &mut LegacyAttributes,
    name: &str,
    target: &mut Vec3,
) -> Result<(), CharacterDefinitionParseError> {
    if let Some(value) = attributes.take_vec3(name)? {
        *target = value;
    }
    Ok(())
}

fn canonical_model_path(value: &str) -> Result<AssetPathBuf, CharacterDefinitionParseError> {
    canonical_path_with_suffix(value, ".chr", ".skeleton.glb", "character skeleton")
}

fn canonical_character_parameters_path(
    value: &str,
) -> Result<AssetPathBuf, CharacterDefinitionParseError> {
    canonical_path_with_suffix(
        value,
        ".chrparams",
        ".character-parameters.ron",
        "character parameters",
    )
}

fn canonical_material_path(value: &str) -> Result<AssetPathBuf, CharacterDefinitionParseError> {
    let normalized = az_asset_builder::normalize_source_path(value);
    let canonical = if let Some(stem) = normalized.strip_suffix(".mtl") {
        format!("{stem}.material.ron")
    } else if normalized
        .rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'))
    {
        return Err(CharacterDefinitionParseError::UnsupportedReference {
            kind: "material",
            path: normalized,
        });
    } else {
        format!("{normalized}.material.ron")
    };
    Ok(AssetPathBuf::new(canonical)?)
}

fn canonical_attachment_binding(
    value: &str,
) -> Result<Option<AttachmentBinding<AssetPathBuf>>, CharacterDefinitionParseError> {
    let normalized = az_asset_builder::normalize_source_path(value);
    let mapped = if let Some(stem) = normalized.strip_suffix(".cdf") {
        Some(AttachmentBinding::Character(AssetPathBuf::new(format!(
            "{stem}.character.ron"
        ))?))
    } else if let Some(stem) = normalized
        .strip_suffix(".chr")
        .or_else(|| normalized.strip_suffix(".skin"))
    {
        Some(AttachmentBinding::SkinnedMesh(AssetPathBuf::new(format!(
            "{stem}.skinnedmesh.glb"
        ))?))
    } else if let Some(stem) = normalized
        .strip_suffix(".cgf")
        .or_else(|| normalized.strip_suffix(".cga"))
    {
        Some(AttachmentBinding::StaticMesh(AssetPathBuf::new(format!(
            "{stem}.staticmesh.glb"
        ))?))
    } else if let Some(stem) = normalized.strip_suffix(".cloth") {
        Some(AttachmentBinding::Cloth(AssetPathBuf::new(format!(
            "{stem}.cloth.ron"
        ))?))
    } else if std::path::Path::new(&normalized)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mtl"))
    {
        // A material accidentally authored as Binding is ignored by the native
        // attachment initializer; the separate Material attribute still applies.
        None
    } else {
        return Err(CharacterDefinitionParseError::UnsupportedReference {
            kind: "attachment binding",
            path: normalized,
        });
    };
    Ok(mapped)
}

fn canonical_path_with_suffix(
    value: &str,
    legacy_suffix: &str,
    canonical_suffix: &str,
    kind: &'static str,
) -> Result<AssetPathBuf, CharacterDefinitionParseError> {
    let normalized = az_asset_builder::normalize_source_path(value);
    let Some(stem) = normalized.strip_suffix(legacy_suffix) else {
        return Err(CharacterDefinitionParseError::UnsupportedReference {
            kind,
            path: normalized,
        });
    };
    Ok(AssetPathBuf::new(format!("{stem}{canonical_suffix}"))?)
}

#[derive(Default)]
struct LegacyAttributes(BTreeMap<String, String>);

impl LegacyAttributes {
    fn read(
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
    ) -> Result<Self, CharacterDefinitionParseError> {
        let mut values = BTreeMap::new();
        for attribute in event.attributes() {
            let attribute = attribute?;
            let name = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::default(), reader.decoder())?
                .into_owned();
            values.insert(name, value);
        }
        Ok(Self(values))
    }

    fn take(&mut self, name: &str) -> Option<String> {
        self.0.remove(name)
    }

    fn take_arc(&mut self, name: &str) -> Option<Arc<str>> {
        self.take(name)
            .filter(|value| !value.is_empty())
            .map(Arc::from)
    }

    fn take_parse<T: FromStr>(
        &mut self,
        name: &str,
    ) -> Result<Option<T>, CharacterDefinitionParseError>
    where
        T::Err: Display,
    {
        self.take(name)
            .map(|value| {
                value.parse::<T>().map_err(|error| {
                    CharacterDefinitionParseError::InvalidAttribute {
                        attribute: name.to_owned(),
                        value,
                        reason: error.to_string(),
                    }
                })
            })
            .transpose()
    }

    fn take_bool(&mut self, name: &str) -> Result<Option<bool>, CharacterDefinitionParseError> {
        self.take(name)
            .map(|value| match value.to_ascii_lowercase().as_str() {
                "0" | "false" | "no" => Ok(false),
                "1" | "true" | "yes" => Ok(true),
                _ => Err(CharacterDefinitionParseError::InvalidAttribute {
                    attribute: name.to_owned(),
                    value,
                    reason: "expected a boolean or zero/one".to_owned(),
                }),
            })
            .transpose()
    }

    fn take_vec2(&mut self, name: &str) -> Result<Option<Vec2>, CharacterDefinitionParseError> {
        self.take_array::<2>(name)
            .map(|value| value.map(Vec2::from_array))
    }

    fn take_vec3(&mut self, name: &str) -> Result<Option<Vec3>, CharacterDefinitionParseError> {
        self.take_array::<3>(name)
            .map(|value| value.map(Vec3::from_array))
    }

    fn take_vec4(&mut self, name: &str) -> Result<Option<Vec4>, CharacterDefinitionParseError> {
        self.take_array::<4>(name)
            .map(|value| value.map(Vec4::from_array))
    }

    /// Reads an optional transform using `CryAnimation`'s native XML `getAttr`
    /// contract.
    ///
    /// `CryAnimation::CAttachmentManager::ParseXMLAttachmentList` leaves the
    /// caller-provided transform default in place when `getAttr` returns false.
    /// Its quaternion overload accepts four values in `w,x,y,z` text order and
    /// stores them as `x,y,z,w`.
    ///
    /// Legacy XML is therefore allowed to contain a malformed optional hint
    /// that the shipping loader ignores. Canonical authoring source remains
    /// strongly typed: the ignored raw value is reported here and is not
    /// serialized as a pretend quaternion.
    fn take_native_optional_quat(&mut self, name: &str, attachment: Option<&str>) -> Option<Quat> {
        self.take_native_optional_array::<4>(name, attachment)
            .map(|[w, x, y, z]| Quat::from_xyzw(x, y, z, w))
    }

    fn take_native_optional_vec3(&mut self, name: &str, attachment: Option<&str>) -> Option<Vec3> {
        self.take_native_optional_array::<3>(name, attachment)
            .map(Vec3::from_array)
    }

    fn take_native_optional_array<const N: usize>(
        &mut self,
        name: &str,
        attachment: Option<&str>,
    ) -> Option<[f32; N]> {
        let value = self.take(name)?;
        match parse_float_array_value::<N>(&value) {
            Ok(parsed) => Some(parsed),
            Err(reason) => {
                tracing::warn!(
                    attribute = name,
                    attachment = attachment.unwrap_or("<unnamed>"),
                    raw_value = value,
                    %reason,
                    "native character XML loader ignores malformed optional transform attribute"
                );
                None
            }
        }
    }

    fn take_array<const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<Option<[f32; N]>, CharacterDefinitionParseError> {
        self.take(name)
            .map(|value| {
                parse_float_array_value(&value).map_err(|reason| {
                    CharacterDefinitionParseError::InvalidAttribute {
                        attribute: name.to_owned(),
                        value,
                        reason,
                    }
                })
            })
            .transpose()
    }

    fn require(
        &mut self,
        name: &'static str,
        element: &'static str,
    ) -> Result<String, CharacterDefinitionParseError> {
        self.take(name).filter(|value| !value.is_empty()).ok_or(
            CharacterDefinitionParseError::MissingAttribute {
                element,
                attribute: name,
            },
        )
    }

    fn discard(&mut self, name: &str) {
        self.0.remove(name);
    }

    fn finish(self, element: &'static str) -> Result<(), CharacterDefinitionParseError> {
        if let Some((attribute, _)) = self.0.into_iter().next() {
            return Err(CharacterDefinitionParseError::UnknownAttribute { element, attribute });
        }
        Ok(())
    }
}

fn parse_float_array_value<const N: usize>(value: &str) -> Result<[f32; N], String> {
    let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != N {
        return Err(format!("expected {N} comma-separated numbers"));
    }
    let mut output = [0.0; N];
    for (index, part) in parts.into_iter().enumerate() {
        output[index] = part
            .parse()
            .map_err(|error: std::num::ParseFloatError| error.to_string())?;
    }
    Ok(output)
}

fn element_name(event: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(event.name().as_ref()).into_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum CharacterDefinitionParseError {
    #[error("unsupported character definition XML path {path}")]
    UnsupportedPath { path: String },
    #[error("character definition XML is not UTF-8")]
    InvalidUtf8(str::Utf8Error),
    #[error("XML parser error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XML attribute error: {0}")]
    Attribute(#[from] AttrError),
    #[error("invalid canonical asset path: {0}")]
    AssetPath(#[from] AssetPathError),
    #[error("missing <{element}> element")]
    MissingElement { element: &'static str },
    #[error("duplicate <{element}> element")]
    DuplicateElement { element: &'static str },
    #[error("missing {attribute:?} attribute on <{element}>")]
    MissingAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    #[error("unsupported attachment Type {value:?}")]
    UnsupportedAttachmentType { value: String },
    #[error("unknown attachment Type {value:?}")]
    UnknownAttachmentType { value: String },
    #[error("unsupported {kind} reference {path}")]
    UnsupportedReference { kind: &'static str, path: String },
    #[error("invalid {attribute}={value:?}: {reason}")]
    InvalidAttribute {
        attribute: String,
        value: String,
        reason: String,
    },
    #[error("invalid {attribute} discriminant {value}")]
    InvalidEnumDiscriminant { attribute: String, value: u32 },
    #[error("unknown attribute {attribute:?} on <{element}>")]
    UnknownAttribute {
        element: &'static str,
        attribute: String,
    },
    #[error("pose modifier data is not representable yet at <{element}>")]
    UnsupportedPoseModifierData { element: String },
    #[error("unexpected <{element}> element in character definition")]
    UnexpectedElement { element: String },
    #[error("unexpected closing </{element}> in character definition")]
    UnexpectedEnd { element: String },
    #[error("element <{element}> appears outside <CharacterDefinition>")]
    ElementOutsideRoot { element: String },
    #[error("unexpected text in character definition XML: {text:?}")]
    UnexpectedText { text: String },
}
