use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

use super::trigger_area::TriggerAreaTag;

/// Lumberyard `LmbrCentral::TagComponent` AZ component UUID.
pub const TAG_COMPONENT_TYPE_ID: Uuid = uuid!("0F16A377-EAA0-47D2-8472-9EAAA680B169");

/// `LmbrCentral` entity tag.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Scripting/TagComponentBus.h:22`.
pub type Tag = TriggerAreaTag;

/// Component that labels an entity with one or more scripting tags.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Scripting/TagComponent.h:26`.
#[derive(
    Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.TagComponent", version = 1)]
pub struct TagComponent {
    pub tags: Vec<Tag>,
}

impl TagComponent {
    #[must_use]
    pub fn from_tags(tags: impl IntoIterator<Item = Tag>) -> Self {
        let mut tags = tags.into_iter().collect::<Vec<_>>();
        tags.sort_unstable();
        tags.dedup();
        Self { tags }
    }

    #[must_use]
    pub fn has_tag(&self, tag: Tag) -> bool {
        self.tags.contains(&tag)
    }

    pub fn add_tag(&mut self, tag: Tag) {
        match self.tags.binary_search(&tag) {
            Ok(_) => {}
            Err(index) => self.tags.insert(index, tag),
        }
    }

    pub fn remove_tag(&mut self, tag: Tag) -> bool {
        match self.tags.binary_search(&tag) {
            Ok(index) => {
                self.tags.remove(index);
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_component_keeps_tags_unique_and_sorted() {
        let high = Tag::from_raw_crc32(0xFFFF_0000);
        let low = Tag::from_raw_crc32(0x0000_0001);

        let mut component = TagComponent::from_tags([high, low, high]);
        assert_eq!(component.tags, vec![low, high]);
        assert!(component.has_tag(high));

        component.add_tag(low);
        assert_eq!(component.tags, vec![low, high]);

        assert!(component.remove_tag(low));
        assert_eq!(component.tags, vec![high]);
        assert!(!component.remove_tag(low));
    }
}
