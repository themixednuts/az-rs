//! Source transforms for `AzPhysics` authoring assets.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};
use bevy::color::LinearRgba;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::{
    CollisionFilterColor, CollisionFiltersAsset, CollisionFiltersParseError,
    EditableCollisionFilter, MaterialProperties, MaterialSetAsset, MaterialSetParseError,
    source_schemas,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollisionFiltersSource {
    pub version: u32,
    pub categories: Vec<String>,
    pub filters: Vec<CollisionFilterSource>,
    pub character_filter_color: LinearRgba,
    pub ghost_filter_color: LinearRgba,
    pub sleeping_body_color: LinearRgba,
    pub custom_filter_colors: Vec<CollisionFilterColorSource>,
}

impl CollisionFiltersSource {
    /// Parses a legacy `.collisionfilters` payload into authoring source.
    ///
    /// # Errors
    ///
    /// Returns any error [`CollisionFiltersAsset::parse`] returns.
    pub fn from_legacy(
        source_path: &str,
        bytes: &[u8],
    ) -> Result<Self, CollisionFiltersParseError> {
        let asset = CollisionFiltersAsset::parse(bytes)?;
        Ok(Self::from_asset(source_path, &asset))
    }

    #[must_use]
    pub fn from_asset(_source_path: &str, asset: &CollisionFiltersAsset) -> Self {
        Self {
            version: 1,
            categories: strings_from_boxes(asset.categories()),
            filters: asset
                .filters()
                .iter()
                .map(CollisionFilterSource::from)
                .collect(),
            character_filter_color: asset.character_filter_color(),
            ghost_filter_color: asset.ghost_filter_color(),
            sleeping_body_color: asset.sleeping_body_color(),
            custom_filter_colors: asset
                .custom_filter_colors()
                .iter()
                .map(CollisionFilterColorSource::from)
                .collect(),
        }
    }

    /// Serializes the authoring source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollisionFilterSource {
    pub name: String,
    pub description: String,
    pub inherits_filters: Vec<String>,
    pub is_categories: Vec<String>,
    pub collide_with_categories: Vec<String>,
    pub filter_tags: Vec<u8>,
}

impl From<&EditableCollisionFilter> for CollisionFilterSource {
    fn from(filter: &EditableCollisionFilter) -> Self {
        Self {
            name: filter.name.to_string(),
            description: filter.description.to_string(),
            inherits_filters: strings_from_boxes(&filter.inherits_filters),
            is_categories: strings_from_boxes(&filter.is_categories),
            collide_with_categories: strings_from_boxes(&filter.collide_with_categories),
            filter_tags: filter.filter_tags.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollisionFilterColorSource {
    pub name: String,
    pub color: LinearRgba,
}

impl From<&CollisionFilterColor> for CollisionFilterColorSource {
    fn from(color: &CollisionFilterColor) -> Self {
        Self {
            name: color.name.to_string(),
            color: color.color,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterialSetSource {
    pub version: u32,
    pub default_material: PhysicsMaterialSource,
    pub materials: Vec<PhysicsMaterialSource>,
}

impl PhysicsMaterialSetSource {
    /// Parses a legacy `.physicsmaterialset` payload into authoring source.
    ///
    /// # Errors
    ///
    /// Returns any error [`MaterialSetAsset::parse`] returns.
    pub fn from_legacy(source_path: &str, bytes: &[u8]) -> Result<Self, MaterialSetParseError> {
        let asset = MaterialSetAsset::parse(bytes)?;
        Ok(Self::from_asset(source_path, &asset))
    }

    #[must_use]
    pub fn from_asset(_source_path: &str, asset: &MaterialSetAsset) -> Self {
        let material_set = asset.material_set();
        Self {
            version: 1,
            default_material: (&material_set.default_material).into(),
            materials: material_set
                .materials
                .iter()
                .map(|entry| (&entry.configuration).into())
                .collect(),
        }
    }

    /// Serializes the authoring source model as pretty RON.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by the RON serializer when a field
    /// cannot be represented in RON.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterialSource {
    pub name: String,
    pub friction: f32,
    pub restitution: f32,
    pub traversable: bool,
    pub surface_type: String,
}

impl From<&MaterialProperties> for PhysicsMaterialSource {
    fn from(material: &MaterialProperties) -> Self {
        Self {
            name: material.name.to_string(),
            friction: material.friction,
            restitution: material.restitution,
            traversable: material.traversable,
            surface_type: material.surface_type.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollisionFiltersSourceTransform;

impl LegacySourceTransform for CollisionFiltersSourceTransform {
    type Error = CollisionFiltersSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_collision_filters_source(&input.source_path) {
            return Err(CollisionFiltersSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = CollisionFiltersSource::from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            collision_filters_source_path(&input.source_path),
            source_schemas::COLLISION_FILTERS,
            source.to_ron_bytes()?,
        ))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicsMaterialSetSourceTransform;

impl LegacySourceTransform for PhysicsMaterialSetSourceTransform {
    type Error = PhysicsMaterialSetSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_physics_material_set_source(&input.source_path) {
            return Err(PhysicsMaterialSetSourceTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let source = PhysicsMaterialSetSource::from_legacy(&input.source_path, input.bytes)?;
        Ok(LegacySourceOutput::authoring_source(
            physics_material_set_source_path(&input.source_path),
            source_schemas::PHYSICS_MATERIAL_SET,
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub fn is_legacy_collision_filters_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with(".collisionfilters")
}

#[must_use]
pub fn is_legacy_physics_material_set_source(source_path: &str) -> bool {
    normalize_source_path(source_path).ends_with(".physicsmaterialset")
}

#[must_use]
pub fn collision_filters_source_path(source_path: &str) -> String {
    format!("{}.ron", normalize_source_path(source_path))
}

#[must_use]
pub fn physics_material_set_source_path(source_path: &str) -> String {
    format!("{}.ron", normalize_source_path(source_path))
}

#[derive(Debug, thiserror::Error)]
pub enum CollisionFiltersSourceTransformError {
    #[error("unsupported collision filters path {path}")]
    UnsupportedPath { path: String },
    #[error("parse collision filters source: {0}")]
    Parse(#[from] CollisionFiltersParseError),
    #[error("serialize collision filters source RON: {0}")]
    Serialize(#[from] ron::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum PhysicsMaterialSetSourceTransformError {
    #[error("unsupported physics material set path {path}")]
    UnsupportedPath { path: String },
    #[error("parse physics material set source: {0}")]
    Parse(#[from] MaterialSetParseError),
    #[error("serialize physics material set source RON: {0}")]
    Serialize(#[from] ron::Error),
}

fn strings_from_boxes(values: &[Box<str>]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{LegacySourceInput, LegacySourceTransform};
    use az_objectstream::{
        Element, ObjectStream, ST_BINARYFLAG_ELEMENT_HEADER, ST_BINARYFLAG_EXTRA_SIZE_FIELD,
        ST_BINARYFLAG_HAS_NAME, ST_BINARYFLAG_HAS_VALUE, types,
    };

    #[test]
    fn collision_filters_transform_emits_authoring_source() {
        let output = CollisionFiltersSourceTransform
            .transform(LegacySourceInput::new(
                "Physics/default.collisionfilters",
                &collision_filters_fixture(),
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring source artifact");
        assert_eq!(artifact.path, "physics/default.collisionfilters.ron");
        assert_eq!(artifact.schema, source_schemas::COLLISION_FILTERS);

        let source: CollisionFiltersSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.version, 1);
        assert_eq!(source.categories, ["Default", "Player"]);
        assert_eq!(
            source.character_filter_color,
            LinearRgba::new(1.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            source.ghost_filter_color,
            LinearRgba::new(0.0, 1.0, 0.0, 1.0)
        );
        assert_eq!(
            source.sleeping_body_color,
            LinearRgba::new(0.0, 0.0, 1.0, 1.0)
        );
        assert_eq!(source.custom_filter_colors[0].name, "Custom");
        assert_eq!(
            source.custom_filter_colors[0].color,
            LinearRgba::new(0.25, 0.5, 0.75, 1.0)
        );

        let filter = &source.filters[0];
        assert_eq!(filter.name, "PlayerFilter");
        assert_eq!(filter.description, "Player collision");
        assert_eq!(filter.inherits_filters, ["Base"]);
        assert_eq!(filter.is_categories, ["Player"]);
        assert_eq!(filter.collide_with_categories, ["World"]);
        assert_eq!(filter.filter_tags, [1]);
    }

    #[test]
    fn physics_material_set_transform_emits_authoring_source() {
        let output = PhysicsMaterialSetSourceTransform
            .transform(LegacySourceInput::new(
                "Physics/default.physicsmaterialset",
                material_set_fixture().as_bytes(),
            ))
            .unwrap();

        let artifact = output.artifact().expect("authoring source artifact");
        assert_eq!(artifact.path, "physics/default.physicsmaterialset.ron");
        assert_eq!(artifact.schema, source_schemas::PHYSICS_MATERIAL_SET);

        let source: PhysicsMaterialSetSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.version, 1);
        assert_eq!(source.default_material.name, "Default");
        assert_exact(source.default_material.friction, 0.5);
        assert_exact(source.default_material.restitution, 0.0);
        assert!(source.default_material.traversable);
        assert_eq!(source.default_material.surface_type, "mat_default");
        assert_eq!(source.materials.len(), 1);
        assert_eq!(source.materials[0].name, "Wood NoTraverse");
        assert_exact(source.materials[0].friction, 0.6);
        assert_exact(source.materials[0].restitution, 0.1);
        assert!(!source.materials[0].traversable);
        assert_eq!(source.materials[0].surface_type, "mat_wood_notraverse");
    }

    /// Compares a parsed `f32` bit-exactly against the literal the fixture
    /// declares.
    ///
    /// The XML fixture spells each value exactly (`0.5000000`), so the parse
    /// is expected to round-trip the literal; an epsilon window would hide
    /// exactly the decode bugs this asserts against.
    #[track_caller]
    fn assert_exact(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    fn collision_filters_fixture() -> Vec<u8> {
        let mut stream = ObjectStream::new(3);
        stream.elements = vec![
            Element::new(crate::COLLISION_FILTERS_ASSET_TYPE_ID)
                .with_wire_flags(ST_BINARYFLAG_ELEMENT_HEADER)
                .with_children(vec![
                    named_element(
                        crate::STRING_VECTOR_TYPE_ID,
                        crate::CATEGORIES_FIELD_CRC,
                        vec![string_value("Default"), string_value("Player")],
                    ),
                    named_element(
                        crate::COLLISION_FILTER_VECTOR_TYPE_ID,
                        crate::FILTERS_FIELD_CRC,
                        vec![filter_element()],
                    ),
                    color_field(
                        crate::CHARACTER_FILTER_COLOR_FIELD_CRC,
                        [1.0, 0.0, 0.0, 1.0],
                    ),
                    color_field(crate::GHOST_FILTER_COLOR_FIELD_CRC, [0.0, 1.0, 0.0, 1.0]),
                    color_field(crate::SLEEPING_BODY_COLOR_FIELD_CRC, [0.0, 0.0, 1.0, 1.0]),
                    named_element(
                        crate::COLLISION_FILTER_COLOR_VECTOR_TYPE_ID,
                        crate::CUSTOM_FILTER_COLORS_FIELD_CRC,
                        vec![
                            Element::new(crate::COLLISION_FILTER_COLOR_TYPE_ID)
                                .with_wire_flags(ST_BINARYFLAG_ELEMENT_HEADER)
                                .with_children(vec![
                                    string_field(crate::NAME_FIELD_CRC, "Custom"),
                                    color_field(crate::COLOR_FIELD_CRC, [0.25, 0.5, 0.75, 1.0]),
                                ]),
                        ],
                    ),
                ]),
        ];
        let mut bytes = Vec::new();
        stream.write_to(&mut bytes).unwrap();
        bytes
    }

    fn filter_element() -> Element {
        Element::new(crate::EDITABLE_COLLISION_FILTER_TYPE_ID)
            .with_wire_flags(ST_BINARYFLAG_ELEMENT_HEADER)
            .with_children(vec![
                string_field(crate::NAME_FIELD_CRC, "PlayerFilter"),
                string_field(crate::DESCRIPTION_FIELD_CRC, "Player collision"),
                named_element(
                    crate::STRING_VECTOR_TYPE_ID,
                    crate::INHERITS_FILTERS_FIELD_CRC,
                    vec![string_value("Base")],
                ),
                named_element(
                    crate::STRING_VECTOR_TYPE_ID,
                    crate::IS_CATEGORIES_FIELD_CRC,
                    vec![string_value("Player")],
                ),
                named_element(
                    crate::STRING_VECTOR_TYPE_ID,
                    crate::COLLIDE_WITH_CATEGORIES_FIELD_CRC,
                    vec![string_value("World")],
                ),
                named_element(
                    crate::COLLISION_FILTER_TAG_VECTOR_TYPE_ID,
                    crate::FILTER_TAGS_FIELD_CRC,
                    vec![byte_field(0, 1)],
                ),
            ])
    }

    fn named_element(id: uuid::Uuid, name_crc: u32, elements: Vec<Element>) -> Element {
        Element::new(id)
            .with_wire_flags(ST_BINARYFLAG_ELEMENT_HEADER | ST_BINARYFLAG_HAS_NAME)
            .with_name_crc(name_crc)
            .with_children(elements)
    }

    fn string_field(name_crc: u32, value: &str) -> Element {
        let mut element = string_value(value);
        element.flags |= ST_BINARYFLAG_HAS_NAME;
        element.name_crc = Some(name_crc);
        element
    }

    fn string_value(value: &str) -> Element {
        Element::new(types::AZSTD_STRING)
            .with_wire_flags(
                ST_BINARYFLAG_ELEMENT_HEADER
                    | ST_BINARYFLAG_HAS_VALUE
                    | ST_BINARYFLAG_EXTRA_SIZE_FIELD
                    | 1,
            )
            .with_declared_data_size(value.len())
            .with_data(value.as_bytes().to_vec())
    }

    fn byte_field(name_crc: u32, value: u8) -> Element {
        let element = Element::new(types::UNSIGNED_CHAR)
            .with_wire_flags(
                ST_BINARYFLAG_ELEMENT_HEADER
                    | (if name_crc == 0 {
                        0
                    } else {
                        ST_BINARYFLAG_HAS_NAME
                    })
                    | ST_BINARYFLAG_HAS_VALUE
                    | ST_BINARYFLAG_EXTRA_SIZE_FIELD
                    | 1,
            )
            .with_declared_data_size(1)
            .with_data(vec![value]);
        if name_crc == 0 {
            element
        } else {
            element.with_name_crc(name_crc)
        }
    }

    fn color_field(name_crc: u32, value: [f32; 4]) -> Element {
        Element::new(types::COLOR)
            .with_wire_flags(
                ST_BINARYFLAG_ELEMENT_HEADER
                    | ST_BINARYFLAG_HAS_NAME
                    | ST_BINARYFLAG_HAS_VALUE
                    | ST_BINARYFLAG_EXTRA_SIZE_FIELD
                    | 1,
            )
            .with_name_crc(name_crc)
            .with_declared_data_size(16)
            .with_data(
                value
                    .into_iter()
                    .flat_map(f32::to_be_bytes)
                    .collect::<Vec<u8>>(),
            )
    }

    fn material_set_fixture() -> &'static str {
        r#"<ObjectStream version="3">
  <Class name="MaterialSetAsset" type="{9E366D8C-33BB-4825-9A1F-FA3ADBE11D0F}">
    <Class name="MaterialSet" field="BaseClass1" version="1" type="{84399E75-18AB-4000-8DCA-07B9D4E0F8E8}">
      <Class name="MaterialProperties" field="DefaultMaterial" version="1" type="{8807CAA1-AD08-4238-8FDB-2154ADD084A1}">
        <Class name="AZStd::string" field="Name" value="Default" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
        <Class name="float" field="Friction" value="0.5000000" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
        <Class name="float" field="Restitution" value="0.0000000" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
        <Class name="bool" field="Traversable" value="true" type="{A0CA880C-AFE4-43CB-926C-59AC48496112}"/>
        <Class name="AZStd::string" field="SurfaceType" value="mat_default" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
      </Class>
      <Class name="AZStd::list" field="Materials" type="{9800688D-64A7-5C0D-9F79-E32E310BB924}">
        <Class name="MaterialEntry" field="element" version="1" type="{C5207CC2-EF1B-4A11-BC8F-F1898282FBE5}">
          <Class name="MaterialProperties" field="Configuration" version="1" type="{8807CAA1-AD08-4238-8FDB-2154ADD084A1}">
            <Class name="AZStd::string" field="Name" value="Wood NoTraverse" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
            <Class name="float" field="Friction" value="0.6000000" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
            <Class name="float" field="Restitution" value="0.1000000" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
            <Class name="bool" field="Traversable" value="false" type="{A0CA880C-AFE4-43CB-926C-59AC48496112}"/>
            <Class name="AZStd::string" field="SurfaceType" value="mat_wood_notraverse" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
          </Class>
        </Class>
      </Class>
    </Class>
  </Class>
</ObjectStream>"#
    }
}
