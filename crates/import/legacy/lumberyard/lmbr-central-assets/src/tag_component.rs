//! Legacy `ObjectStream` reader for `LmbrCentral::TagComponent`.
//!
//! Lumberyard registers the component's `Tags` field in
//! `Gems/LmbrCentral/Code/Source/Scripting/TagComponent.cpp`.

use az_core::component::EntityId;
use az_gem_lmbr_central::{TAG_COMPONENT_TYPE_ID, Tag, TagComponent};
use az_objectstream::context::ContainerShape;
use az_objectstream::query::{az_entity_elements, pointee};
use az_objectstream::value::{self, ObjectStreamValueError};
use az_objectstream::{Element, types};
use thiserror::Error;
use uuid::Uuid;

/// One `TagComponent` and the serialized entity that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTagComponent {
    /// The `AZ::EntityId` read from the owning entity.
    pub entity_id: EntityId,
    /// The normalized runtime component.
    pub component: TagComponent,
}

#[derive(Debug, Error)]
pub enum TagComponentObjectStreamError {
    #[error("ObjectStream type identity could not be read")]
    Type(#[source] ObjectStreamValueError),
    #[error("expected LmbrCentral::TagComponent, got {actual}")]
    UnexpectedType { actual: Uuid },
    #[error("LmbrCentral::TagComponent is missing `Tags`")]
    MissingTags,
    #[error("LmbrCentral::TagComponent field `Tags` could not be read")]
    Tags(#[source] ObjectStreamValueError),
    #[error("tagged AZ::Entity field `Id` could not be read")]
    EntityId(#[source] ObjectStreamValueError),
}

/// Read a reflected `LmbrCentral::TagComponent`.
///
/// `element` may be either the component itself or the serialized smart-pointer
/// wrapper used by `AZ::Entity::Components`. The returned value is the runtime
/// owner's [`TagComponent`], not an import-only copy.
///
/// # Errors
///
/// Returns [`TagComponentObjectStreamError::Type`] when the reflected type
/// cannot be resolved, [`TagComponentObjectStreamError::UnexpectedType`] for
/// another reflected type, [`TagComponentObjectStreamError::MissingTags`] when
/// the recognized component has no `Tags` field, or
/// [`TagComponentObjectStreamError::Tags`] when a tag CRC cannot be decoded.
pub fn read_tag_component(
    element: &Element,
) -> Result<TagComponent, TagComponentObjectStreamError> {
    let element = pointee(element).unwrap_or(element);
    let actual = value::semantic_type_id(element).map_err(TagComponentObjectStreamError::Type)?;
    if actual != TAG_COMPONENT_TYPE_ID {
        return Err(TagComponentObjectStreamError::UnexpectedType { actual });
    }

    let tags = value::child_by_field(element, "Tags")
        .ok_or(TagComponentObjectStreamError::MissingTags)
        .and_then(|tags| read_tag_set(tags).map_err(TagComponentObjectStreamError::Tags))?;

    Ok(TagComponent::from_tags(
        tags.into_iter().map(Tag::from_raw_crc32),
    ))
}

/// Read the direct `LmbrCentral::TagComponent` owned by an `AZ::Entity`.
///
/// Returns `Ok(None)` for another reflected type, an entity without a direct
/// `Components` field, or an entity without this component. If malformed input
/// contains the component more than once, the reader merges and deduplicates
/// its tags through [`TagComponent::from_tags`].
///
/// # Errors
///
/// Returns [`TagComponentObjectStreamError::EntityId`] when a tagged entity has
/// no readable `Id`. A recognized component can also return any error described
/// by [`read_tag_component`].
pub fn read_entity_tag_component(
    entity: &Element,
) -> Result<Option<EntityTagComponent>, TagComponentObjectStreamError> {
    if value::semantic_type_id(entity).map_err(TagComponentObjectStreamError::Type)?
        != types::AZ_ENTITY
    {
        return Ok(None);
    }
    let Some(components) = value::child_by_field(entity, "Components") else {
        return Ok(None);
    };

    let mut tags = Vec::new();
    let mut found = false;
    for candidate in components.children() {
        let component = pointee(candidate).unwrap_or(candidate);
        if value::semantic_type_id(component).map_err(TagComponentObjectStreamError::Type)?
            != TAG_COMPONENT_TYPE_ID
        {
            continue;
        }
        found = true;
        tags.extend(read_tag_component(candidate)?.tags);
    }
    if !found {
        return Ok(None);
    }

    let entity_id = value::child_by_field(entity, "Id")
        .ok_or_else(|| ObjectStreamValueError::MissingField {
            field: "Id".to_owned(),
        })
        .and_then(value::read_entity_id)
        .map(EntityId::new)
        .map_err(TagComponentObjectStreamError::EntityId)?;

    Ok(Some(EntityTagComponent {
        entity_id,
        component: TagComponent::from_tags(tags),
    }))
}

fn read_tag_set(element: &Element) -> Result<Vec<u32>, ObjectStreamValueError> {
    match value::require_container_shape(
        element,
        ContainerShape::Set,
        "AZStd::unordered_set<AZ::Crc32>",
    ) {
        Ok(()) | Err(ObjectStreamValueError::UnexpectedContainerShape { actual: None, .. }) => {}
        Err(error) => return Err(error),
    }

    element
        .children()
        .iter()
        .filter(|child| {
            child
                .resolved_type_id()
                .copied()
                .unwrap_or_else(|| *child.raw_type_id())
                == types::CRC32
        })
        .map(value::read_crc32)
        .collect()
}

/// Read every direct `LmbrCentral::TagComponent` under `ObjectStream` roots.
///
/// Results are sorted by numeric entity id and tag CRCs. Duplicate entity ids
/// remain separate because nested source data can contain distinct serialized
/// entities with the same id.
///
/// # Errors
///
/// Returns the first error from [`read_entity_tag_component`].
pub fn read_entity_tag_components(
    roots: &[Element],
) -> Result<Vec<EntityTagComponent>, TagComponentObjectStreamError> {
    let mut components = az_entity_elements(roots)
        .filter_map(|entity| read_entity_tag_component(entity).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    components.sort_by(|left, right| {
        left.entity_id
            .value()
            .cmp(&right.entity_id.value())
            .then_with(|| left.component.tags.cmp(&right.component.tags))
    });
    Ok(components)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_direct_tags_into_the_owner_type() {
        let roots = vec![entity(42, vec![tag_component(&[9, 2, 9])])];

        assert_eq!(
            read_entity_tag_components(&roots).unwrap(),
            vec![EntityTagComponent {
                entity_id: EntityId::new(42),
                component: TagComponent::from_tags([
                    Tag::from_raw_crc32(2),
                    Tag::from_raw_crc32(9),
                ]),
            }]
        );
    }

    #[test]
    fn reads_a_component_behind_the_entity_smart_pointer() {
        let wrapper =
            Element::new(types::AZSTD_SHARED_PTR).with_children(vec![tag_component(&[0x1234])]);
        let roots = vec![entity(7, vec![wrapper])];

        let components = read_entity_tag_components(&roots).unwrap();

        assert_eq!(components.len(), 1);
        assert_eq!(components[0].entity_id, EntityId::new(7));
        assert_eq!(
            components[0].component.tags,
            vec![Tag::from_raw_crc32(0x1234)]
        );
    }

    #[test]
    fn ignores_nested_and_unrelated_components() {
        let nested = Element::new(Uuid::from_u128(1)).with_children(vec![tag_component(&[3])]);
        let roots = vec![entity(7, vec![nested])];

        assert!(read_entity_tag_components(&roots).unwrap().is_empty());
    }

    #[test]
    fn merges_duplicate_components_deterministically() {
        let roots = vec![entity(
            9,
            vec![tag_component(&[5, 2]), tag_component(&[5, 7])],
        )];

        let components = read_entity_tag_components(&roots).unwrap();

        assert_eq!(
            components[0].component.tags,
            vec![
                Tag::from_raw_crc32(2),
                Tag::from_raw_crc32(5),
                Tag::from_raw_crc32(7),
            ]
        );
    }

    #[test]
    fn malformed_recognized_tags_fail_closed() {
        let malformed_crc = Element::new(types::CRC32).with_children(vec![
            Element::new(types::UNSIGNED_INT)
                .with_field("value")
                .with_data([0, 1]),
        ]);
        let malformed = Element::new(TAG_COMPONENT_TYPE_ID).with_children(vec![
            Element::new(types::AZSTD_UNORDERED_SET)
                .with_field("Tags")
                .with_children(vec![malformed_crc]),
        ]);

        assert!(matches!(
            read_tag_component(&malformed),
            Err(TagComponentObjectStreamError::Tags(
                ObjectStreamValueError::InvalidLength { .. }
            ))
        ));
    }

    #[test]
    fn missing_recognized_tags_field_fails_closed() {
        let component = Element::new(TAG_COMPONENT_TYPE_ID);

        assert!(matches!(
            read_tag_component(&component),
            Err(TagComponentObjectStreamError::MissingTags)
        ));
    }

    #[test]
    fn rejects_an_unrelated_component_type() {
        let component = Element::new(Uuid::from_u128(1));

        assert!(matches!(
            read_tag_component(&component),
            Err(TagComponentObjectStreamError::UnexpectedType { .. })
        ));
    }

    #[test]
    fn rejects_a_known_non_set_tags_container() {
        let component = Element::new(TAG_COMPONENT_TYPE_ID)
            .with_children(vec![Element::new(types::AZSTD_VECTOR).with_field("Tags")]);

        assert!(matches!(
            read_tag_component(&component),
            Err(TagComponentObjectStreamError::Tags(
                ObjectStreamValueError::UnexpectedContainerShape {
                    actual: Some(ContainerShape::Sequence),
                    ..
                }
            ))
        ));
    }

    #[test]
    fn tagged_entity_requires_an_entity_id() {
        let entity = Element::new(types::AZ_ENTITY).with_children(vec![
            Element::new(types::AZSTD_VECTOR)
                .with_field("Components")
                .with_children(vec![tag_component(&[1])]),
        ]);

        assert!(matches!(
            read_entity_tag_component(&entity),
            Err(TagComponentObjectStreamError::EntityId(
                ObjectStreamValueError::MissingField { .. }
            ))
        ));
    }

    #[test]
    fn sorts_entities_by_numeric_id() {
        let roots = vec![
            entity(20, vec![tag_component(&[1])]),
            entity(3, vec![tag_component(&[2])]),
        ];

        let components = read_entity_tag_components(&roots).unwrap();

        assert_eq!(
            components
                .into_iter()
                .map(|component| component.entity_id.value())
                .collect::<Vec<_>>(),
            vec![3, 20]
        );
    }

    fn entity(id: u64, components: Vec<Element>) -> Element {
        Element::new(types::AZ_ENTITY).with_children(vec![
            entity_id(id).with_field("Id"),
            Element::new(types::AZSTD_VECTOR)
                .with_field("Components")
                .with_children(components),
        ])
    }

    fn entity_id(id: u64) -> Element {
        Element::new(types::ENTITY_ID).with_children(vec![
            Element::new(types::AZ_U64)
                .with_field("id")
                .with_data(id.to_be_bytes()),
        ])
    }

    fn tag_component(values: &[u32]) -> Element {
        Element::new(TAG_COMPONENT_TYPE_ID).with_children(vec![tags(values)])
    }

    fn tags(values: &[u32]) -> Element {
        Element::new(types::AZSTD_UNORDERED_SET)
            .with_field("Tags")
            .with_children(values.iter().copied().map(crc32).collect::<Vec<_>>())
    }

    fn crc32(value: u32) -> Element {
        Element::new(types::CRC32).with_children(vec![
            Element::new(types::UNSIGNED_INT)
                .with_field("value")
                .with_data(value.to_be_bytes()),
        ])
    }
}
