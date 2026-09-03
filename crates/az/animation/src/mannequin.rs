//! Typed `CryAction` Mannequin runtime primitives.

use std::mem::size_of;

use az_core::crc::Crc32;
use bevy_math::Isometry3d;
use bevy_reflect::Reflect;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

mod controller;
mod database;
mod database_driver;
mod definition;
mod scope;

pub use controller::*;
pub use database::*;
pub use database_driver::*;
pub use definition::*;
pub use scope::*;

macro_rules! index_id {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Reflect,
            Serialize,
            Deserialize,
        )]
        pub struct $name(i32);

        impl $name {
            #[must_use]
            pub fn new(index: usize) -> Option<Self> {
                i32::try_from(index).ok().map(Self)
            }

            #[must_use]
            pub const fn native_value(self) -> i32 {
                self.0
            }

            #[must_use]
            pub fn index(self) -> usize {
                usize::try_from(self.0).expect("validated Mannequin id")
            }
        }

        impl TryFrom<i32> for $name {
            type Error = InvalidMannequinId;

            fn try_from(value: i32) -> Result<Self, Self::Error> {
                (value >= 0)
                    .then_some(Self(value))
                    .ok_or(InvalidMannequinId(value))
            }
        }

        impl From<$name> for i32 {
            fn from(value: $name) -> Self {
                value.native_value()
            }
        }
    };
}

index_id!(TagId);
index_id!(TagGroupId);
index_id!(FragmentId);
index_id!(ScopeId);
index_id!(ScopeContextId);
index_id!(SubContextId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid Mannequin id {0}")]
pub struct InvalidMannequinId(pub i32);

/// Cry Mannequin tag state. Lumberyard defines `TAGSTATE_MAX_BYTES` as 12 in
/// `dev/Code/CryEngine/CryCommon/ICryMannequinTagDefs.h`.
///
/// The bits are assigned by [`TagDefinition`]. A `TagId` is an index into a
/// definition, not a bit position: mutually-exclusive tags in the same group
/// share a compact encoded value in one byte.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub struct TagState([u32; 3]);

impl TagState {
    pub const EMPTY: Self = Self([0; 3]);
    pub const FULL: Self = Self([u32::MAX; 3]);

    #[must_use]
    pub const fn from_words(words: [u32; 3]) -> Self {
        Self(words)
    }

    #[must_use]
    pub const fn words(self) -> [u32; 3] {
        self.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0
    }

    fn masked_byte(self, mask: TagMask) -> u8 {
        self.byte(mask.byte) & mask.mask
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the shifted word is masked with u8::MAX first, so the cast is exact"
    )]
    fn byte(self, byte: u8) -> u8 {
        let byte = usize::from(byte);
        let word = self.0[byte / size_of::<u32>()];
        ((word >> ((byte % size_of::<u32>()) * u8::BITS as usize)) & u32::from(u8::MAX)) as u8
    }

    fn set_mask(&mut self, mask: TagMask, enabled: bool) {
        let byte = usize::from(mask.byte);
        let shift = (byte % size_of::<u32>()) * u8::BITS as usize;
        let shifted_mask = u32::from(mask.mask) << shift;
        let word = &mut self.0[byte / size_of::<u32>()];
        if enabled {
            *word |= shifted_mask;
        } else {
            *word &= !shifted_mask;
        }
    }
}

/// Cry's byte-local tag mask. Group masks never straddle a byte boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub struct TagMask {
    byte: u8,
    mask: u8,
}

impl TagMask {
    #[must_use]
    pub const fn byte(self) -> u8 {
        self.byte
    }

    #[must_use]
    pub const fn mask(self) -> u8 {
        self.mask
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct TagDefinition {
    tag_masks: Vec<TagMask>,
    tag_groups: Vec<Option<TagGroupId>>,
    tag_priorities: Vec<i32>,
    group_masks: Vec<TagMask>,
    num_bits: u32,
}

impl TagDefinition {
    pub const MAX_BYTES: usize = 12;

    #[must_use]
    pub fn builder() -> TagDefinitionBuilder {
        TagDefinitionBuilder::default()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.tag_masks.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.tag_masks.is_empty()
    }

    #[must_use]
    pub const fn num_bits(&self) -> u32 {
        self.num_bits
    }

    #[must_use]
    pub fn tag_mask(&self, tag: TagId) -> Option<TagMask> {
        self.tag_masks.get(tag.index()).copied()
    }

    #[must_use]
    pub fn group_mask(&self, group: TagGroupId) -> Option<TagMask> {
        self.group_masks.get(group.index()).copied()
    }

    #[must_use]
    pub fn group(&self, tag: TagId) -> Option<TagGroupId> {
        self.tag_groups.get(tag.index()).copied().flatten()
    }

    #[must_use]
    pub fn priority(&self, tag: TagId) -> Option<i32> {
        self.tag_priorities.get(tag.index()).copied()
    }

    #[must_use]
    pub fn is_set(&self, state: TagState, tag: TagId) -> bool {
        let Some(tag_mask) = self.tag_mask(tag) else {
            return false;
        };
        if let Some(group) = self.group(tag) {
            let Some(group_mask) = self.group_mask(group) else {
                return false;
            };
            tag_mask.mask != 0 && state.masked_byte(group_mask) == tag_mask.mask
        } else {
            tag_mask.mask != 0 && state.masked_byte(tag_mask) == tag_mask.mask
        }
    }

    /// Sets or clears `tag` in `state`, honouring mutually-exclusive groups.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTagId`] when `tag` is not part of this definition, or
    /// when its group has no mask.
    pub fn set(&self, state: &mut TagState, tag: TagId, enabled: bool) -> Result<(), InvalidTagId> {
        let tag_mask = self.tag_mask(tag).ok_or(InvalidTagId(tag))?;
        if let Some(group) = self.group(tag) {
            let group_mask = self.group_mask(group).ok_or(InvalidTagId(tag))?;
            if self.is_set(*state, tag) != enabled {
                state.set_mask(group_mask, false);
                if enabled {
                    state.set_mask(tag_mask, true);
                }
            }
        } else {
            state.set_mask(tag_mask, enabled);
        }
        Ok(())
    }

    /// Clears every tag belonging to `group` in `state`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTagGroupId`] when `group` is not part of this
    /// definition.
    pub fn clear_group(
        &self,
        state: &mut TagState,
        group: TagGroupId,
    ) -> Result<(), InvalidTagGroupId> {
        let mask = self.group_mask(group).ok_or(InvalidTagGroupId(group))?;
        state.set_mask(mask, false);
        Ok(())
    }

    /// Cry `CTagDefinition::GetUnion`: apply every tag present in `right` to
    /// `left`, replacing an existing value when both tags belong to the same
    /// mutually-exclusive group.
    ///
    /// # Panics
    ///
    /// Never in practice: every tag is enumerated from this definition's own
    /// mask table, so both the id construction and the set-back succeed.
    #[must_use]
    pub fn union(&self, mut left: TagState, right: TagState) -> TagState {
        for index in 0..self.tag_masks.len() {
            let tag = TagId::new(index).expect("validated tag-definition index");
            if self.is_set(right, tag) {
                self.set(&mut left, tag, true)
                    .expect("tag came from this definition");
            }
        }
        left
    }

    /// Cry's grouped-tag-aware containment test.
    #[must_use]
    pub fn contains(&self, parent: TagState, child: TagState) -> bool {
        if parent
            .0
            .iter()
            .zip(child.0)
            .any(|(parent, child)| parent & child != child)
        {
            return false;
        }

        let mut comparison_mask = child;
        for group_mask in &self.group_masks {
            if child.masked_byte(*group_mask) != 0 {
                comparison_mask.set_mask(*group_mask, true);
            }
        }
        parent
            .0
            .iter()
            .zip(child.0)
            .zip(comparison_mask.0)
            .all(|((parent, child), mask)| parent & mask == child)
    }

    /// Whether an authored candidate contains every required global tag.
    #[must_use]
    pub fn contains_required(&self, candidate: TagState, required: TagState) -> bool {
        let comparison_mask = self.comparison_mask(required);
        candidate
            .0
            .iter()
            .zip(required.0)
            .zip(comparison_mask.0)
            .all(|((candidate, required), mask)| candidate & mask == required)
    }

    #[must_use]
    pub fn rate(&self, state: TagState) -> u32 {
        self.rate_with_tallies(state, &self.priority_tallies())
    }

    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the tally counts tags in two definitions, each capped at \
                  TagDefinition::MAX_BYTES * 8 = 96 tags"
    )]
    pub fn combined_priority_tallies(&self, other: &Self) -> Vec<TagPriorityCount> {
        let mut priorities = self
            .tag_priorities
            .iter()
            .chain(&other.tag_priorities)
            .copied()
            .collect::<Vec<_>>();
        priorities.sort_unstable();
        priorities.dedup();
        priorities
            .into_iter()
            .map(|priority| TagPriorityCount {
                priority,
                count: self
                    .tag_priorities
                    .iter()
                    .chain(&other.tag_priorities)
                    .filter(|candidate| **candidate == priority)
                    .count() as u32,
            })
            .collect()
    }

    #[must_use]
    pub fn rate_with_tallies(&self, state: TagState, tallies: &[TagPriorityCount]) -> u32 {
        self.tag_priorities
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, _)| TagId::new(*index).is_some_and(|tag| self.is_set(state, tag)))
            .fold(0_u32, |score, (_, priority)| {
                let tally = tallies
                    .iter()
                    .take_while(|count| priority > count.priority)
                    .fold(1_u32, |value, count| {
                        value.saturating_mul(count.count.saturating_add(1))
                    });
                score.saturating_add(tally)
            })
    }

    fn comparison_mask(&self, state: TagState) -> TagState {
        let mut mask = state;
        for group_mask in &self.group_masks {
            if state.masked_byte(*group_mask) != 0 {
                mask.set_mask(*group_mask, true);
            }
        }
        mask
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the tally counts tags in one definition, capped at \
                  TagDefinition::MAX_BYTES * 8 = 96 tags"
    )]
    fn priority_tallies(&self) -> Vec<TagPriorityCount> {
        let mut priorities = self.tag_priorities.clone();
        priorities.sort_unstable();
        priorities.dedup();
        priorities
            .into_iter()
            .map(|priority| TagPriorityCount {
                priority,
                count: self
                    .tag_priorities
                    .iter()
                    .filter(|candidate| **candidate == priority)
                    .count() as u32,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TagPriorityCount {
    pub priority: i32,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagDefinitionBuilder {
    group_count: usize,
    tags: Vec<TagDefinitionTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TagDefinitionTag {
    group: Option<TagGroupId>,
    priority: i32,
}

impl TagDefinitionBuilder {
    /// Declares one mutually-exclusive tag group and returns its id.
    ///
    /// # Panics
    ///
    /// Panics once more than `i32::MAX` groups have been declared, which no
    /// authored controller definition approaches.
    #[must_use]
    pub fn add_group(&mut self) -> TagGroupId {
        let group = TagGroupId::new(self.group_count).expect("Mannequin tag-group id exhausted");
        self.group_count += 1;
        group
    }

    /// Declares one tag, optionally inside a previously declared group.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTagGroupId`] when `group` was not produced by
    /// [`Self::add_group`] on this builder.
    ///
    /// # Panics
    ///
    /// Panics once more than `i32::MAX` tags have been declared, which no
    /// authored controller definition approaches.
    pub fn add_tag(
        &mut self,
        group: Option<TagGroupId>,
        priority: i32,
    ) -> Result<TagId, InvalidTagGroupId> {
        if let Some(group) = group
            && group.index() >= self.group_count
        {
            return Err(InvalidTagGroupId(group));
        }
        let tag = TagId::new(self.tags.len()).expect("Mannequin tag id exhausted");
        self.tags.push(TagDefinitionTag { group, priority });
        Ok(tag)
    }

    /// Packs every declared tag into Cry's byte-local mask layout.
    ///
    /// # Errors
    ///
    /// Returns [`TagCapacityError`] when the declared tags and groups do not
    /// fit in [`TagDefinition::MAX_BYTES`] bytes.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "byte indices come from `0..TagDefinition::MAX_BYTES` (12) and the group \
                  mask is built from at most eight bits, so both casts are exact"
    )]
    pub fn build(self) -> Result<TagDefinition, TagCapacityError> {
        let mut tag_masks = vec![TagMask::default(); self.tags.len()];
        let mut group_masks = vec![TagMask::default(); self.group_count];
        let mut tags_mapped = 0;
        let mut num_bits = 0;

        for current_byte in 0..TagDefinition::MAX_BYTES {
            if tags_mapped == self.tags.len() {
                break;
            }
            let mut current_bit = 0_u32;

            for (group_index, group_mask) in group_masks.iter_mut().enumerate() {
                if current_bit >= u8::BITS || group_mask.mask != 0 {
                    continue;
                }
                let tags_in_group = self
                    .tags
                    .iter()
                    .filter(|tag| tag.group.is_some_and(|group| group.index() == group_index))
                    .count();
                if tags_in_group == 0 {
                    continue;
                }
                let required_bits = usize::BITS - tags_in_group.leading_zeros();
                if required_bits > u8::BITS - current_bit {
                    continue;
                }

                group_mask.byte = current_byte as u8;
                group_mask.mask = (((1_u16 << required_bits) - 1) as u8) << current_bit;
                let mut encoded_value = 0_u8;
                for (index, tag) in self.tags.iter().enumerate() {
                    if tag.group.is_some_and(|group| group.index() == group_index) {
                        encoded_value += 1;
                        tag_masks[index] = TagMask {
                            byte: current_byte as u8,
                            mask: encoded_value << current_bit,
                        };
                        tags_mapped += 1;
                    }
                }
                current_bit += required_bits;
            }

            for (index, tag) in self.tags.iter().enumerate() {
                if current_bit >= u8::BITS {
                    break;
                }
                if tag_masks[index].mask == 0 && tag.group.is_none() {
                    tag_masks[index] = TagMask {
                        byte: current_byte as u8,
                        mask: 1 << current_bit,
                    };
                    current_bit += 1;
                    tags_mapped += 1;
                }
            }
            num_bits += if tags_mapped < self.tags.len() {
                u8::BITS
            } else {
                current_bit
            };
        }

        if tags_mapped != self.tags.len() {
            return Err(TagCapacityError {
                tags: self.tags.len(),
                mapped: tags_mapped,
            });
        }

        Ok(TagDefinition {
            tag_masks,
            tag_groups: self.tags.iter().map(|tag| tag.group).collect(),
            tag_priorities: self.tags.iter().map(|tag| tag.priority).collect(),
            group_masks,
            num_bits,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("tag definition needs more than 96 bits: mapped {mapped} of {tags} tags")]
pub struct TagCapacityError {
    pub tags: usize,
    pub mapped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("tag id {0:?} is not in this definition")]
pub struct InvalidTagId(pub TagId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("tag group id {0:?} is not in this definition")]
pub struct InvalidTagGroupId(pub TagGroupId);

/// Runtime fragment table compiled from a Mannequin action definition.
///
/// Fragment IDs are dense indices rather than packed tag-state values. Each
/// fragment may still own a packed sub-tag definition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct FragmentDefinition {
    fragment_tags: Vec<Option<TagDefinition>>,
}

impl FragmentDefinition {
    #[must_use]
    pub const fn new(fragment_tags: Vec<Option<TagDefinition>>) -> Self {
        Self { fragment_tags }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.fragment_tags.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fragment_tags.is_empty()
    }

    #[must_use]
    pub fn tag_definition(&self, fragment: FragmentId) -> Option<&TagDefinition> {
        self.fragment_tags
            .get(fragment.index())
            .and_then(Option::as_ref)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
pub struct FragmentTagState {
    pub global_tags: TagState,
    pub fragment_tags: TagState,
}

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ScopeMask: u32 {
        const ALL = u32::MAX;
    }

    /// Current `IAction::EFlags` values plus Azoth-internal runtime flags.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct ActionFlags: u32 {
        const BLEND_OUT = 1 << 0;
        const NO_AUTO_BLEND_OUT = 1 << 1;
        const INTERRUPTABLE = 1 << 2;
        // Bit 3 is reserved and has an empty name in the shipping enum table.
        const INSTALLING = 1 << 4;
        const STARTED = 1 << 5;
        const REQUEUED = 1 << 6;
        const TRUMP_SELF = 1 << 7;
        const TRANSITIONING = 1 << 8;
        const PLAYING_FRAGMENT = 1 << 9;
        const TRANSITIONING_OUT = 1 << 10;
        const TRANSITION_PENDING = 1 << 11;
        const FRAGMENT_IS_ONE_SHOT = 1 << 12;
        const STOPPING = 1 << 13;
        const TRANS_OCCURRED_BEFORE_UPDATE = 1 << 14;
        const FREEZE_LAST_FRAME_ON_END = 1 << 15;
        const SCOPELESS = 1 << 16;
        const LAST_PENDING = 1 << 17;
        const CLAMP_FRAGMENT_TIME_TO_DURATION = 1 << 18;
        const AUTO_REQUEUED = 1 << 19;
        const WAS_RANDOM = 1 << 20;
        const JUST_INSTALLED = 1 << 21;
        /// Internal one-shot latch consumed when a fragment would otherwise
        /// take the time-warp auto-reinstall path.
        const SKIP_TIMEWARP_AUTO_REINSTALL_ONCE = 1 << 22;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ResumeFlags: u32 {
        const RESTART_ANIMATIONS = 1 << 0;
        const RESTORE_LOOPING_ANIMATION_TIME = 1 << 1;
        const RESTORE_NON_LOOPING_ANIMATION_TIME = 1 << 2;
        const DEFAULT = Self::RESTART_ANIMATIONS.bits()
            | Self::RESTORE_LOOPING_ANIMATION_TIME.bits()
            | Self::RESTORE_NON_LOOPING_ANIMATION_TIME.bits();
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct ControllerFlags: u32 {
        const PAUSED_UPDATE = 1 << 0;
        const DEBUG_DRAW = 1 << 1;
        const DUMP_STATE = 1 << 2;
        const IS_IN_UPDATE = 1 << 3;
        const NO_TRANSITIONS = 1 << 4;
        const ENTER_PROCEDURAL_ON_INSTALL = 1 << 5;
        const RESOLVE_QUEUE_BEFORE_UPDATE = 1 << 6;
    }

    /// Lumberyard fragment-definition flags plus Azoth's time-warp reinstall
    /// extension.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct FragmentFlags: u32 {
        const PERSISTENT = 1 << 0;
        const AUTO_REINSTALL = 1 << 1;
        const TIMEWARP_AUTO_REINSTALL = 1 << 2;
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct TransitionFlags: u32 {
        const CYCLIC = 1 << 0;
        const CYCLE_LOCKED = 1 << 1;
        const OUTRO = 1 << 2;
    }
}

impl ActionFlags {
    const PLAYBACK_STATE: Self = Self::from_bits_retain(
        Self::TRANSITIONING.bits() | Self::PLAYING_FRAGMENT.bits() | Self::TRANSITIONING_OUT.bits(),
    );

    /// Action flags for persistent queue entries.
    #[must_use]
    pub const fn for_persistence(persistent: bool) -> Self {
        if persistent {
            Self::from_bits_retain(Self::NO_AUTO_BLEND_OUT.bits() | Self::INTERRUPTABLE.bits())
        } else {
            Self::empty()
        }
    }
}

impl ScopeMask {
    #[must_use]
    pub fn from_scope(scope: ScopeId) -> Option<Self> {
        (scope.index() < u32::BITS as usize).then(|| Self::from_bits_retain(1_u32 << scope.index()))
    }

    #[must_use]
    pub fn contains_scope(self, scope: ScopeId) -> bool {
        Self::from_scope(scope).is_some_and(|flag| self.contains(flag))
    }

    #[must_use]
    pub fn least_significant_scope(self) -> Option<ScopeId> {
        (!self.is_empty())
            .then(|| ScopeId::new(self.bits().trailing_zeros() as usize))
            .flatten()
    }
}

impl Default for ResumeFlags {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ActionStatus {
    #[default]
    None,
    Pending,
    Installed,
    Exiting,
    Finished,
}

/// Why an action was failed by the controller.
///
/// The discriminants match Cry's `EActionFailure` enum.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionFailure {
    QueueFull,
    InvalidContext,
}

/// How controller-owned state is flushed from an action.
///
/// `NormalLeaveAnimations` differs while scopes are flushed but has the same
/// action lifetime as `Normal`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ActionEndMethod {
    #[default]
    Normal,
    NormalLeaveAnimations,
    Failure,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityComparison {
    Lower,
    Equal,
    Higher,
}

impl From<std::cmp::Ordering> for PriorityComparison {
    fn from(value: std::cmp::Ordering) -> Self {
        match value {
            std::cmp::Ordering::Less => Self::Lower,
            std::cmp::Ordering::Equal => Self::Equal,
            std::cmp::Ordering::Greater => Self::Higher,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize, Default)]
pub enum OptionIndex {
    #[default]
    Random,
    Index(u32),
}

impl OptionIndex {
    pub const RANDOM_NATIVE_VALUE: u32 = 0xffff_fffe;
    pub const INVALID_NATIVE_VALUE: u32 = u32::MAX;

    #[must_use]
    pub const fn native_value(self) -> u32 {
        match self {
            Self::Random => Self::RANDOM_NATIVE_VALUE,
            Self::Index(index) => index,
        }
    }
}

impl TryFrom<u32> for OptionIndex {
    type Error = InvalidOptionIndex;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            Self::RANDOM_NATIVE_VALUE => Ok(Self::Random),
            Self::INVALID_NATIVE_VALUE => Err(InvalidOptionIndex),
            value => Ok(Self::Index(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid Mannequin option index")]
pub struct InvalidOptionIndex;

/// One controller parameter keyed by Mannequin's lowercase name CRC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MannequinParameter {
    pub name: Crc32,
    pub value: Isometry3d,
}

impl MannequinParameter {
    #[must_use]
    pub fn new(name: impl Into<Crc32>, value: impl Into<Isometry3d>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

impl Default for MannequinParameter {
    fn default() -> Self {
        Self {
            name: Crc32::ZERO,
            value: Isometry3d::IDENTITY,
        }
    }
}

/// Small contiguous parameter table used by action controllers and scopes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MannequinParameters {
    entries: SmallVec<[MannequinParameter; 8]>,
}

impl MannequinParameters {
    #[must_use]
    pub fn get(&self, name: impl Into<Crc32>) -> Option<&Isometry3d> {
        let name = name.into();
        self.entries
            .iter()
            .find(|parameter| parameter.name == name)
            .map(|parameter| &parameter.value)
    }

    #[must_use]
    pub fn get_parameter(&self, name: impl Into<Crc32>) -> Option<&MannequinParameter> {
        let name = name.into();
        self.entries.iter().find(|parameter| parameter.name == name)
    }

    pub fn set(&mut self, name: impl Into<Crc32>, value: impl Into<Isometry3d>) {
        self.set_parameter(MannequinParameter::new(name, value));
    }

    pub fn set_parameter(&mut self, parameter: MannequinParameter) {
        if let Some(current) = self
            .entries
            .iter_mut()
            .find(|current| current.name == parameter.name)
        {
            *current = parameter;
        } else {
            self.entries.push(parameter);
        }
    }

    pub fn remove(&mut self, name: impl Into<Crc32>) -> Option<MannequinParameter> {
        let name = name.into();
        let index = self
            .entries
            .iter()
            .position(|parameter| parameter.name == name)?;
        Some(self.entries.remove(index))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterates the stored parameters in insertion order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &MannequinParameter> {
        self.entries.iter()
    }
}

/// Read-only parameter capability used to compose controller ownership chains.
pub trait MannequinParameterSource {
    fn parameter(&self, name: Crc32) -> Option<&Isometry3d>;
}

impl MannequinParameterSource for MannequinParameters {
    fn parameter(&self, name: Crc32) -> Option<&Isometry3d> {
        self.get(name)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FragmentRequestId(u32);

impl FragmentRequestId {
    pub const INVALID_NATIVE_VALUE: u32 = u32::MAX;

    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        if value == 0 || value == Self::INVALID_NATIVE_VALUE {
            None
        } else {
            Some(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentRequestIdAllocator {
    next: u32,
}

impl Default for FragmentRequestIdAllocator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl FragmentRequestIdAllocator {
    /// Hands out the next request id, wrapping back to one on exhaustion.
    ///
    /// # Panics
    ///
    /// Never in practice: one is always a valid [`FragmentRequestId`], so the
    /// wrap-around path cannot fail.
    pub fn allocate(&mut self) -> FragmentRequestId {
        let id = FragmentRequestId::new(self.next).unwrap_or_else(|| {
            self.next = 1;
            FragmentRequestId::new(1).expect("one is a valid request id")
        });
        self.next = self.next.wrapping_add(1);
        id
    }
}

/// Runtime state common to Cry `IAction` implementations.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub fragment_id: FragmentId,
    pub fragment_tags: TagState,
    pub priority: i32,
    pub forced_scope_mask: ScopeMask,
    pub installed_scope_mask: ScopeMask,
    pub root_scope: Option<ScopeId>,
    pub sub_context: Option<SubContextId>,
    pub user_token: u32,
    pub option_index: OptionIndex,
    pub speed_bias: f32,
    pub animation_weight: f32,
    /// Optional offset added to every selected clip's key time.
    pub fragment_start_time_offset: Option<f32>,
    /// Optional blend duration applied to every selected clip.
    pub fragment_blend_time: Option<f32>,
    pub status: ActionStatus,
    pub flags: ActionFlags,
    pub active_time: f32,
    pub queue_time: Option<f32>,
}

impl Action {
    pub const MAX_QUEUE_SIZE: usize = 10;

    #[must_use]
    pub const fn builder(fragment_id: FragmentId) -> ActionBuilder {
        ActionBuilder::new(fragment_id)
    }

    #[must_use]
    pub fn can_blend_out(&self, comparison: PriorityComparison) -> bool {
        if comparison == PriorityComparison::Higher {
            return true;
        }

        (self.flags.contains(ActionFlags::FRAGMENT_IS_ONE_SHOT)
            && !self.flags.contains(ActionFlags::NO_AUTO_BLEND_OUT))
            || self.flags.contains(ActionFlags::BLEND_OUT)
            || matches!(self.status, ActionStatus::Finished | ActionStatus::Exiting)
    }

    /// Reset transient action state before queueing.
    pub fn initialise(&mut self, queue_time: Option<f32>) {
        self.queue_time = queue_time;
        self.status = ActionStatus::Pending;
        self.root_scope = None;
        self.flags.remove(
            ActionFlags::STARTED
                | ActionFlags::STOPPING
                | ActionFlags::BLEND_OUT
                | ActionFlags::REQUEUED
                | ActionFlags::AUTO_REQUEUED,
        );
        self.active_time = 0.0;
    }

    pub fn install(&mut self) {
        if self.status != ActionStatus::Finished {
            self.status = ActionStatus::Installed;
        }
        self.flags.remove(ActionFlags::PLAYBACK_STATE);
    }

    pub fn enter(&mut self) {
        self.flags.insert(ActionFlags::STARTED);
    }

    pub fn exit(&mut self) {
        self.status = ActionStatus::None;
        self.flags.remove(ActionFlags::STARTED);
        self.restore_random_option();
    }

    pub fn fail(&mut self) {
        self.exit();
    }

    pub fn update_pending(&mut self, delta_time: f32) -> ActionStatus {
        let previous_active_time = self.active_time;
        self.active_time += delta_time;
        if let Some(queue_time) = self.queue_time {
            if self.active_time >= queue_time {
                self.flags.insert(ActionFlags::LAST_PENDING);
            }
            if previous_active_time > 0.0 && self.active_time > queue_time {
                self.status = ActionStatus::Finished;
            }
        }
        self.status
    }

    pub fn update(&mut self, delta_time: f32) -> ActionStatus {
        self.active_time += delta_time;
        self.status
    }

    /// Apply the controller's shipped once-per-frame update gate.
    pub fn update_from_controller(&mut self, delta_time: f32) -> ActionStatus {
        if self
            .flags
            .contains(ActionFlags::TRANS_OCCURRED_BEFORE_UPDATE)
        {
            self.flags.remove(ActionFlags::TRANS_OCCURRED_BEFORE_UPDATE);
            return self.status;
        }

        let status = self.update(delta_time * self.speed_bias);
        self.flags
            .remove(ActionFlags::TRANSITION_PENDING | ActionFlags::JUST_INSTALLED);
        status
    }

    pub fn stop(&mut self) {
        self.flags
            .insert(ActionFlags::BLEND_OUT | ActionFlags::STOPPING);
    }

    pub fn force_finish(&mut self) {
        self.status = ActionStatus::Finished;
        self.flags.remove(ActionFlags::INTERRUPTABLE);
    }

    pub fn begin_installing(&mut self) {
        self.flags.insert(ActionFlags::INSTALLING);
        self.flags
            .remove(ActionFlags::PLAYING_FRAGMENT | ActionFlags::TRANSITIONING);
    }

    pub fn end_installing(&mut self) {
        self.flags.remove(ActionFlags::INSTALLING);
    }

    pub fn transition_started(&mut self) {
        self.flags.remove(ActionFlags::PLAYBACK_STATE);
        self.flags.insert(ActionFlags::TRANSITIONING);
    }

    pub fn fragment_started(&mut self) {
        self.flags.remove(ActionFlags::PLAYBACK_STATE);
        self.flags.insert(ActionFlags::PLAYING_FRAGMENT);
    }

    pub fn transition_out_started(&mut self) {
        self.status = ActionStatus::Exiting;
        self.flags.remove(ActionFlags::PLAYBACK_STATE);
        self.flags.insert(ActionFlags::TRANSITIONING_OUT);
    }

    fn restore_random_option(&mut self) {
        if self.flags.contains(ActionFlags::WAS_RANDOM) {
            self.flags.remove(ActionFlags::WAS_RANDOM);
            self.option_index = OptionIndex::Random;
        }
    }
}

/// `IAction`'s overridable lifecycle surface, separated from the controller and
/// animation backend so richer action types compose without controller wrappers.
pub trait MannequinAction: AsRef<Action> + AsMut<Action> {
    /// `IAction::ComparePriority`. Cry calls this only when integer priorities
    /// are equal, allowing domain actions to break ties without replacing the
    /// controller's base ordering rules.
    fn compare_priority(&self, _current: &Self) -> PriorityComparison {
        PriorityComparison::Equal
    }

    /// `IAction::DoComparePriority`, including `TrumpSelf` and integer priority.
    fn do_compare_priority(&self, current: &Self, same_action: bool) -> PriorityComparison {
        let candidate_state = self.as_ref();
        let current_state = current.as_ref();
        if same_action && candidate_state.flags.contains(ActionFlags::TRUMP_SELF) {
            PriorityComparison::Higher
        } else {
            match candidate_state.priority.cmp(&current_state.priority) {
                std::cmp::Ordering::Equal => self.compare_priority(current),
                ordering => ordering.into(),
            }
        }
    }

    /// `IAction::OnRequestBlendOut`.
    fn on_request_blend_out(&mut self, _comparison: PriorityComparison) {}

    fn on_initialise(&mut self) {}

    fn on_cancel_install(&mut self) {}

    fn on_exit(&mut self) {}

    fn on_failure(&mut self, _failure: ActionFailure) {}

    /// `IAction::OnActionEvent`, called by Cry's `ActionEvent` procedural clip.
    fn on_action_event(&mut self, _event: Crc32) {}

    /// `IAction::OnSequenceFinished`, emitted by the scope that drained an
    /// animation FIFO layer.
    fn on_sequence_finished(&mut self, _layer: u32, _scope: ScopeId) {}
}

impl MannequinAction for Action {}

impl AsRef<Self> for Action {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsMut<Self> for Action {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionBuilder(Action);

impl ActionBuilder {
    #[must_use]
    pub const fn new(fragment_id: FragmentId) -> Self {
        Self(Action {
            fragment_id,
            fragment_tags: TagState::EMPTY,
            priority: 0,
            forced_scope_mask: ScopeMask::empty(),
            installed_scope_mask: ScopeMask::empty(),
            root_scope: None,
            sub_context: None,
            user_token: 0,
            option_index: OptionIndex::Random,
            speed_bias: 1.0,
            animation_weight: 1.0,
            fragment_start_time_offset: None,
            fragment_blend_time: None,
            status: ActionStatus::None,
            flags: ActionFlags::empty(),
            active_time: 0.0,
            queue_time: None,
        })
    }

    #[must_use]
    pub const fn priority(mut self, priority: i32) -> Self {
        self.0.priority = priority;
        self
    }

    #[must_use]
    pub const fn fragment_tags(mut self, tags: TagState) -> Self {
        self.0.fragment_tags = tags;
        self
    }

    #[must_use]
    pub fn persistent(mut self, persistent: bool) -> Self {
        self.0
            .flags
            .remove(ActionFlags::NO_AUTO_BLEND_OUT | ActionFlags::INTERRUPTABLE);
        self.0
            .flags
            .insert(ActionFlags::for_persistence(persistent));
        self
    }

    #[must_use]
    pub fn flags(mut self, flags: ActionFlags) -> Self {
        self.0.flags.insert(flags);
        self
    }

    #[must_use]
    pub const fn forced_scopes(mut self, scopes: ScopeMask) -> Self {
        self.0.forced_scope_mask = scopes;
        self
    }

    #[must_use]
    pub const fn queue_time(mut self, queue_time: Option<f32>) -> Self {
        self.0.queue_time = queue_time;
        self
    }

    #[must_use]
    pub const fn speed_bias(mut self, speed_bias: f32) -> Self {
        self.0.speed_bias = speed_bias;
        self
    }

    #[must_use]
    pub const fn animation_weight(mut self, animation_weight: f32) -> Self {
        self.0.animation_weight = animation_weight;
        self
    }

    #[must_use]
    pub const fn fragment_start_time_offset(mut self, start_time: Option<f32>) -> Self {
        self.0.fragment_start_time_offset = start_time;
        self
    }

    #[must_use]
    pub const fn fragment_blend_time(mut self, smooth_time: Option<f32>) -> Self {
        self.0.fragment_blend_time = smooth_time;
        self
    }

    #[must_use]
    pub const fn build(self) -> Action {
        self.0
    }
}

/// Stable handle used by scopes and queues while an action remains arena-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionHandle {
    slot: u32,
    generation: u32,
}

impl ActionHandle {
    /// Packs the stable generational identity without exposing the arena's
    /// representation as separate public fields.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        (self.generation as u64) << 32 | self.slot as u64
    }

    /// Reconstructs a handle identity. Arena membership is still validated by
    /// the arena operation that consumes the handle.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the low half of the packed identity is the slot by construction, so the \
                  truncation is the intended unpacking"
    )]
    pub const fn from_bits(bits: u64) -> Self {
        Self {
            slot: bits as u32,
            generation: (bits >> 32) as u32,
        }
    }
}

impl From<ActionHandle> for u64 {
    fn from(value: ActionHandle) -> Self {
        value.to_bits()
    }
}

impl From<u64> for ActionHandle {
    fn from(value: u64) -> Self {
        Self::from_bits(value)
    }
}

#[derive(Debug)]
struct ActionSlot<A> {
    generation: u32,
    action: Option<A>,
}

/// Generational storage keeps queued and installed actions at one stable identity.
#[derive(Debug)]
pub struct ActionArena<A> {
    slots: Vec<ActionSlot<A>>,
    free: Vec<u32>,
}

impl<A> Default for ActionArena<A> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<A> ActionArena<A> {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free: Vec::with_capacity(capacity),
        }
    }

    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "slot indices are allocated through `u32::try_from` in `insert`, so every \
                  index in the arena already fits in a u32"
    )]
    #[expect(
        clippy::double_must_use,
        reason = "the iterator this returns is already #[must_use]"
    )]
    pub fn iter(&self) -> impl Iterator<Item = (ActionHandle, &A)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.action.as_ref().map(|action| {
                (
                    ActionHandle {
                        slot: index as u32,
                        generation: slot.generation,
                    },
                    action,
                )
            })
        })
    }

    /// Stores `action` in a free or freshly allocated slot.
    ///
    /// # Panics
    ///
    /// Panics once the arena holds more than `u32::MAX` slots, which no
    /// controller approaches.
    pub fn insert(&mut self, action: A) -> ActionHandle {
        if let Some(slot) = self.free.pop() {
            let entry = &mut self.slots[slot as usize];
            debug_assert!(entry.action.is_none());
            entry.action = Some(action);
            ActionHandle {
                slot,
                generation: entry.generation,
            }
        } else {
            let slot = u32::try_from(self.slots.len()).expect("Mannequin action arena exhausted");
            self.slots.push(ActionSlot {
                generation: 0,
                action: Some(action),
            });
            ActionHandle {
                slot,
                generation: 0,
            }
        }
    }

    #[must_use]
    pub fn get(&self, handle: ActionHandle) -> Option<&A> {
        self.slots
            .get(handle.slot as usize)
            .filter(|slot| slot.generation == handle.generation)
            .and_then(|slot| slot.action.as_ref())
    }

    pub fn get_mut(&mut self, handle: ActionHandle) -> Option<&mut A> {
        self.slots
            .get_mut(handle.slot as usize)
            .filter(|slot| slot.generation == handle.generation)
            .and_then(|slot| slot.action.as_mut())
    }

    pub fn remove(&mut self, handle: ActionHandle) -> Option<A> {
        let slot = self.slots.get_mut(handle.slot as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        let action = slot.action.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(handle.slot);
        Some(action)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("stale or foreign Mannequin action handle")]
pub struct InvalidActionHandle;

/// Cry's stable, descending-priority pending queue.
#[derive(Debug, Default)]
pub struct ActionQueue {
    handles: SmallVec<[ActionHandle; 16]>,
}

impl ActionQueue {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            handles: SmallVec::with_capacity(capacity),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// Iterates the queued handles in priority order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = ActionHandle> + '_ {
        self.handles.iter().copied()
    }

    #[must_use]
    pub fn contains(&self, handle: ActionHandle) -> bool {
        self.handles.contains(&handle)
    }

    /// Initialises `handle` and places it on the pending queue.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` is not live in `arena`.
    pub fn queue<A>(
        &mut self,
        arena: &mut ActionArena<A>,
        handle: ActionHandle,
        queue_time: Option<f32>,
    ) -> Result<(), InvalidActionHandle>
    where
        A: MannequinAction,
    {
        let action = arena.get_mut(handle).ok_or(InvalidActionHandle)?;
        action.as_mut().initialise(queue_time);
        action.on_initialise();
        self.push_onto_queue(arena, handle)
    }

    /// Marks an installed action as re-queued and pushes it back on the queue.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidActionHandle`] when `handle` is not live in `arena`.
    pub fn requeue<A>(
        &mut self,
        arena: &mut ActionArena<A>,
        handle: ActionHandle,
    ) -> Result<(), InvalidActionHandle>
    where
        A: MannequinAction,
    {
        let action = arena.get_mut(handle).ok_or(InvalidActionHandle)?.as_mut();
        debug_assert_eq!(action.status, ActionStatus::Installed);
        action.flags.insert(ActionFlags::REQUEUED);
        self.push_onto_queue(arena, handle)
    }

    pub fn remove(&mut self, handle: ActionHandle) -> bool {
        let Some(index) = self.handles.iter().position(|queued| *queued == handle) else {
            return false;
        };
        self.handles.remove(index);
        true
    }

    pub fn clear<A>(&mut self, arena: &mut ActionArena<A>)
    where
        A: AsMut<Action>,
    {
        for handle in self.handles.drain(..) {
            if let Some(action) = arena.get_mut(handle) {
                action.as_mut().status = ActionStatus::None;
            }
        }
    }

    /// Match Cry's queue cap: discard lowest-priority non-interruptable actions first.
    pub fn prune<A>(&mut self, arena: &mut ActionArena<A>) -> Vec<ActionHandle>
    where
        A: MannequinAction,
    {
        let mut removed = Vec::new();
        self.prune_into(arena, &mut removed);
        removed
    }

    pub fn prune_into<A>(
        &mut self,
        arena: &mut ActionArena<A>,
        removed: &mut impl Extend<ActionHandle>,
    ) where
        A: MannequinAction,
    {
        while self.handles.len() > Action::MAX_QUEUE_SIZE {
            let Some(index) = self.handles.iter().rposition(|handle| {
                arena.get(*handle).is_some_and(|action| {
                    !action.as_ref().flags.contains(ActionFlags::INTERRUPTABLE)
                })
            }) else {
                break;
            };
            let handle = self.handles.remove(index);
            if let Some(action) = arena.get_mut(handle) {
                action.on_failure(ActionFailure::QueueFull);
                action.as_mut().fail();
            }
            removed.extend(std::iter::once(handle));
        }
    }

    pub(super) fn push_onto_queue<A>(
        &mut self,
        arena: &ActionArena<A>,
        handle: ActionHandle,
    ) -> Result<(), InvalidActionHandle>
    where
        A: MannequinAction,
    {
        let candidate = arena.get(handle).ok_or(InvalidActionHandle)?;
        let candidate_requeued = candidate.as_ref().flags.contains(ActionFlags::REQUEUED);
        let mut insertion = self.handles.len();

        for (index, current_handle) in self.handles.iter().copied().enumerate() {
            let current = arena.get(current_handle).ok_or(InvalidActionHandle)?;
            let comparison = candidate.do_compare_priority(current, handle == current_handle);
            let insert_here = comparison == PriorityComparison::Higher
                || (comparison == PriorityComparison::Equal
                    && candidate_requeued
                    && !current.as_ref().flags.contains(ActionFlags::REQUEUED));
            if insert_here {
                insertion = index;
                break;
            }
        }

        self.handles.insert(insertion, handle);
        Ok(())
    }
}

/// Name/id lookup required by the component-facing controller API.
pub trait MannequinDefinition {
    fn fragment_id(&self, name: &str) -> Option<FragmentId>;
    fn global_tag_id(&self, name: &str) -> Option<TagId>;
    fn fragment_tag_id(&self, fragment: FragmentId, name: &str) -> Option<TagId>;
    fn tag_group_id(&self, name: &str) -> Option<TagGroupId>;
    fn scope_id(&self, name: &str) -> Option<ScopeId>;
    fn scope_context_id(&self, name: &str) -> Option<ScopeContextId>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_component_request_uses_persistence_flags() {
        assert_eq!(
            ActionFlags::for_persistence(true),
            ActionFlags::NO_AUTO_BLEND_OUT | ActionFlags::INTERRUPTABLE
        );
        assert_eq!(ActionFlags::for_persistence(false), ActionFlags::empty());
        assert_eq!(ActionFlags::SCOPELESS.bits(), 1 << 16);
        assert_eq!(ActionFlags::LAST_PENDING.bits(), 1 << 17);
        assert_eq!(ActionFlags::AUTO_REQUEUED.bits(), 1 << 19);
        assert_eq!(ActionFlags::WAS_RANDOM.bits(), 1 << 20);
        assert_eq!(ActionFlags::JUST_INSTALLED.bits(), 1 << 21);
        assert_eq!(
            ActionFlags::SKIP_TIMEWARP_AUTO_REINSTALL_ONCE.bits(),
            1 << 22
        );
    }

    #[test]
    fn action_status_and_flags_follow_cry_lifecycle() {
        let fragment = FragmentId::new(3).unwrap();
        let mut action = Action::builder(fragment).persistent(true).build();

        action.begin_installing();
        assert!(action.flags.contains(ActionFlags::INSTALLING));
        action.end_installing();
        action.install();
        action.enter();
        action.fragment_started();
        assert_eq!(action.status, ActionStatus::Installed);
        assert!(action.flags.contains(ActionFlags::PLAYING_FRAGMENT));

        action.stop();
        assert!(action.flags.contains(ActionFlags::BLEND_OUT));
        action.force_finish();
        assert_eq!(action.status, ActionStatus::Finished);
        assert!(!action.flags.contains(ActionFlags::INTERRUPTABLE));
    }

    #[test]
    fn pending_timeout_marks_last_pending_before_finishing() {
        let fragment = FragmentId::new(0).unwrap();
        let mut action = Action::builder(fragment).queue_time(Some(0.1)).build();
        action.initialise(Some(0.1));

        assert_eq!(action.update_pending(0.1), ActionStatus::Pending);
        assert!(action.flags.contains(ActionFlags::LAST_PENDING));
        assert_eq!(action.update_pending(0.01), ActionStatus::Finished);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the test asserts that exactly one tick is skipped, so the accumulated \
                  active time must match 0.0 and 0.5 bit-for-bit"
    )]
    fn transition_before_update_skips_one_action_tick() {
        let fragment = FragmentId::new(0).unwrap();
        let mut action = Action::builder(fragment)
            .speed_bias(2.0)
            .flags(
                ActionFlags::TRANS_OCCURRED_BEFORE_UPDATE
                    | ActionFlags::TRANSITION_PENDING
                    | ActionFlags::JUST_INSTALLED,
            )
            .build();

        action.update_from_controller(0.25);
        assert_eq!(action.active_time, 0.0);
        assert!(
            !action
                .flags
                .contains(ActionFlags::TRANS_OCCURRED_BEFORE_UPDATE)
        );
        assert!(action.flags.contains(ActionFlags::JUST_INSTALLED));

        action.update_from_controller(0.25);
        assert_eq!(action.active_time, 0.5);
        assert!(
            !action
                .flags
                .intersects(ActionFlags::TRANSITION_PENDING | ActionFlags::JUST_INSTALLED)
        );
    }

    #[test]
    fn tag_definition_packs_ungrouped_tags_into_exactly_96_bits() {
        let mut builder = TagDefinition::builder();
        for _ in 0..96 {
            builder.add_tag(None, 0).unwrap();
        }
        let definition = builder.build().unwrap();
        let mut tags = TagState::EMPTY;
        let last = TagId::new(95).unwrap();
        definition.set(&mut tags, last, true).unwrap();
        assert!(definition.is_set(tags, last));
        assert_eq!(definition.num_bits(), 96);

        let mut overflow = TagDefinition::builder();
        for _ in 0..97 {
            overflow.add_tag(None, 0).unwrap();
        }
        assert!(overflow.build().is_err());
    }

    #[test]
    fn grouped_tags_share_a_byte_local_encoded_value() {
        let mut builder = TagDefinition::builder();
        let stance = builder.add_group();
        let idle = builder.add_tag(Some(stance), 1).unwrap();
        let combat = builder.add_tag(Some(stance), 2).unwrap();
        let swimming = builder.add_tag(Some(stance), 3).unwrap();
        let grounded = builder.add_tag(None, 0).unwrap();
        let definition = builder.build().unwrap();

        assert_eq!(definition.group_mask(stance).unwrap().mask(), 0b11);
        assert_eq!(definition.tag_mask(idle).unwrap().mask(), 0b01);
        assert_eq!(definition.tag_mask(combat).unwrap().mask(), 0b10);
        assert_eq!(definition.tag_mask(swimming).unwrap().mask(), 0b11);
        assert_eq!(definition.tag_mask(grounded).unwrap().mask(), 0b100);

        let mut state = TagState::EMPTY;
        definition.set(&mut state, idle, true).unwrap();
        assert!(definition.is_set(state, idle));
        definition.set(&mut state, combat, true).unwrap();
        assert!(!definition.is_set(state, idle));
        assert!(definition.is_set(state, combat));
        definition.set(&mut state, grounded, true).unwrap();
        assert!(definition.is_set(state, grounded));
    }

    #[test]
    fn containment_compares_the_full_group_value() {
        let mut builder = TagDefinition::builder();
        let stance = builder.add_group();
        let idle = builder.add_tag(Some(stance), 0).unwrap();
        let combat = builder.add_tag(Some(stance), 0).unwrap();
        let grounded = builder.add_tag(None, 0).unwrap();
        let definition = builder.build().unwrap();

        let mut idle_grounded = TagState::EMPTY;
        definition.set(&mut idle_grounded, idle, true).unwrap();
        definition.set(&mut idle_grounded, grounded, true).unwrap();
        let mut combat_grounded = TagState::EMPTY;
        definition.set(&mut combat_grounded, combat, true).unwrap();
        definition
            .set(&mut combat_grounded, grounded, true)
            .unwrap();
        let mut grounded_only = TagState::EMPTY;
        definition.set(&mut grounded_only, grounded, true).unwrap();

        assert!(definition.contains(idle_grounded, idle_grounded));
        assert!(definition.contains(idle_grounded, grounded_only));
        assert!(!definition.contains(idle_grounded, combat_grounded));
    }

    #[test]
    fn queue_matches_cry_priority_and_requeue_order() {
        let fragment = FragmentId::new(0).unwrap();
        let mut arena = ActionArena::default();
        let low = arena.insert(Action::builder(fragment).priority(1).build());
        let high = arena.insert(Action::builder(fragment).priority(3).build());
        let equal = arena.insert(Action::builder(fragment).priority(3).build());
        let requeued = arena.insert(Action::builder(fragment).priority(3).build());
        arena.get_mut(requeued).unwrap().status = ActionStatus::Installed;

        let mut queue = ActionQueue::default();
        queue.queue(&mut arena, low, None).unwrap();
        queue.queue(&mut arena, high, None).unwrap();
        queue.queue(&mut arena, equal, None).unwrap();
        queue.requeue(&mut arena, requeued).unwrap();

        assert_eq!(
            queue.iter().collect::<Vec<_>>(),
            vec![requeued, high, equal, low]
        );
    }

    #[test]
    fn stale_action_handles_cannot_alias_reused_slots() {
        let fragment = FragmentId::new(0).unwrap();
        let mut arena = ActionArena::default();
        let stale = arena.insert(Action::builder(fragment).build());
        arena.remove(stale).unwrap();
        let current = arena.insert(Action::builder(fragment).build());

        assert!(arena.get(stale).is_none());
        assert!(arena.get(current).is_some());
        assert_ne!(stale, current);
    }
}
