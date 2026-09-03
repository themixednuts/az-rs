use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{PhysicsBodyHandle, PhysicsEntityId, SurfaceIndex};

/// `RockNRoll` collision filters address 96 authored categories as three
/// 32-bit words.
pub const COLLISION_CATEGORY_WORDS: usize = 3;

/// Compact 96-category membership mask used by `RockNRoll` collision filters.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct CollisionCategoryMask([u32; COLLISION_CATEGORY_WORDS]);

impl CollisionCategoryMask {
    pub const EMPTY: Self = Self([0; COLLISION_CATEGORY_WORDS]);

    #[must_use]
    pub const fn from_words(words: [u32; COLLISION_CATEGORY_WORDS]) -> Self {
        Self(words)
    }

    #[must_use]
    pub const fn words(self) -> [u32; COLLISION_CATEGORY_WORDS] {
        self.0
    }

    #[must_use]
    pub const fn contains(self, category: usize) -> bool {
        let word = category / u32::BITS as usize;
        let bit = category % u32::BITS as usize;
        word < COLLISION_CATEGORY_WORDS && (self.0[word] & (1 << bit)) != 0
    }

    /// Sets one category bit and reports whether the category was in range.
    pub const fn insert(&mut self, category: usize) -> bool {
        let word = category / u32::BITS as usize;
        if word >= COLLISION_CATEGORY_WORDS {
            return false;
        }
        let bit = category % u32::BITS as usize;
        self.0[word] |= 1 << bit;
        true
    }

    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0[0] & other.0[0]) != 0
            || (self.0[1] & other.0[1]) != 0
            || (self.0[2] & other.0[2]) != 0
    }
}

impl AsRef<[u32; COLLISION_CATEGORY_WORDS]> for CollisionCategoryMask {
    fn as_ref(&self) -> &[u32; COLLISION_CATEGORY_WORDS] {
        &self.0
    }
}

impl From<[u32; COLLISION_CATEGORY_WORDS]> for CollisionCategoryMask {
    fn from(words: [u32; COLLISION_CATEGORY_WORDS]) -> Self {
        Self::from_words(words)
    }
}

impl From<CollisionCategoryMask> for [u32; COLLISION_CATEGORY_WORDS] {
    fn from(mask: CollisionCategoryMask) -> Self {
        mask.words()
    }
}

impl std::ops::BitOr for CollisionCategoryMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self::from_words([
            self.0[0] | rhs.0[0],
            self.0[1] | rhs.0[1],
            self.0[2] | rhs.0[2],
        ])
    }
}

impl std::ops::BitOrAssign for CollisionCategoryMask {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

/// Immutable compiled `RockNRoll` collision filter.
///
/// The filter retains three `is_categories` words, three
/// `collides_with_categories` words, and an inherited tag mask. Runtime bodies
/// retain this compiled value instead of reducing it to a 32-layer
/// approximation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct CollisionFilter {
    pub is_categories: CollisionCategoryMask,
    pub collides_with_categories: CollisionCategoryMask,
    pub tags: u32,
}

impl CollisionFilter {
    #[must_use]
    pub const fn new(
        is_categories: CollisionCategoryMask,
        collides_with_categories: CollisionCategoryMask,
        tags: u32,
    ) -> Self {
        Self {
            is_categories,
            collides_with_categories,
            tags,
        }
    }

    /// Exact `RockNRoll` pair rule.
    ///
    /// A pair is accepted when either filter asks to collide with at least one
    /// category supplied by the other filter.
    #[must_use]
    pub const fn interacts_with(self, other: Self) -> bool {
        self.collides_with_categories
            .intersects(other.is_categories)
            || other
                .collides_with_categories
                .intersects(self.is_categories)
    }

    pub fn inherit(&mut self, parent: Self) {
        self.is_categories |= parent.is_categories;
        self.collides_with_categories |= parent.collides_with_categories;
        self.tags |= parent.tags;
    }
}

/// `CryPhysics` collision class (`SCollisionClass`).
///
/// Two classes interact unless either class explicitly ignores one of the
/// other class's type bits. A zero type mask is valid and does not mean
/// "collide with nothing".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct CollisionClass {
    pub type_mask: u32,
    pub ignore_mask: u32,
}

impl CollisionClass {
    #[must_use]
    pub const fn new(type_mask: u32, ignore_mask: u32) -> Self {
        Self {
            type_mask,
            ignore_mask,
        }
    }

    /// Returns `true` when the pair must not produce contacts or sensor hits.
    #[must_use]
    pub const fn ignores(self, other: Self) -> bool {
        (self.type_mask & other.ignore_mask) != 0 || (other.type_mask & self.ignore_mask) != 0
    }

    #[must_use]
    pub const fn interacts_with(self, other: Self) -> bool {
        !self.ignores(other)
    }
}

/// Whether an interaction is a solver contact or a sensor overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsInteractionKind {
    Contact,
    Trigger,
}

/// Lifecycle transition emitted for an interaction pair after a physics step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsInteractionPhase {
    Started,
    Persisted,
    Stopped,
}

/// Backend-neutral contact/trigger event. The normal points from `body_a`
/// toward `body_b`; stopped interactions retain pair identity but have no
/// current contact geometry.
#[derive(
    bevy_ecs::message::Message, Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect,
)]
pub struct PhysicsInteraction {
    pub phase: PhysicsInteractionPhase,
    pub kind: PhysicsInteractionKind,
    pub body_a: PhysicsBodyHandle,
    pub body_b: PhysicsBodyHandle,
    pub entity_a: Option<PhysicsEntityId>,
    pub entity_b: Option<PhysicsEntityId>,
    pub surface_a: SurfaceIndex,
    pub surface_b: SurfaceIndex,
    pub tag_a: crate::ColliderTag,
    pub tag_b: crate::ColliderTag,
    pub point: Option<Vec3>,
    pub normal: Option<Vec3>,
    pub penetration_depth: f32,
    pub impulse: Vec3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_ignore_is_symmetric() {
        let player = CollisionClass::new(1 << 2, 1 << 5);
        let projectile = CollisionClass::new(1 << 5, 0);

        assert!(player.ignores(projectile));
        assert!(projectile.ignores(player));
        assert!(!CollisionClass::default().ignores(projectile));
    }

    #[test]
    fn rock_n_roll_filter_pair_test_is_symmetric_but_either_direction_can_accept() {
        let mut actor_categories = CollisionCategoryMask::EMPTY;
        assert!(actor_categories.insert(7));
        let actor = CollisionFilter::new(actor_categories, CollisionCategoryMask::EMPTY, 0);

        let mut cast_targets = CollisionCategoryMask::EMPTY;
        assert!(cast_targets.insert(7));
        let cast = CollisionFilter::new(CollisionCategoryMask::EMPTY, cast_targets, 0);

        assert!(actor.interacts_with(cast));
        assert!(cast.interacts_with(actor));
        assert!(!actor.interacts_with(actor));
    }

    #[test]
    fn rock_n_roll_filter_inheritance_ors_all_native_words_and_tags() {
        let mut child = CollisionFilter::new(
            CollisionCategoryMask::from_words([1, 0, 0]),
            CollisionCategoryMask::from_words([0, 2, 0]),
            0x01,
        );
        child.inherit(CollisionFilter::new(
            CollisionCategoryMask::from_words([0, 0, 4]),
            CollisionCategoryMask::from_words([8, 0, 0]),
            0x80,
        ));

        assert_eq!(child.is_categories.words(), [1, 0, 4]);
        assert_eq!(child.collides_with_categories.words(), [8, 2, 0]);
        assert_eq!(child.tags, 0x81);
    }
}
