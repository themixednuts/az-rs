//! Typed Mannequin fragment and transition selection databases.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::playback::AnimationFlags;

use super::{
    AnimationClip, ClipType, Fragment, FragmentData, FragmentId, FragmentSequenceFlags,
    FragmentTagState, ProceduralEntry, TagDefinition, TagState, TransitionFlags,
};

/// Definition surface required to compile and query a Mannequin database.
pub trait MannequinDatabaseDefinition {
    fn global_tag_definition(&self) -> &TagDefinition;

    fn fragment_tag_definition(&self, fragment: FragmentId) -> Option<&TagDefinition>;
}

/// Read-only animation-product information used while assembling a query.
///
/// Cry obtains these values from `IAnimationSet`. Keeping the capability as a
/// trait lets Bevy animation assets, blend spaces, and test fixtures expose the
/// same information without coupling the database to a renderer.
pub trait AnimationMetadata<K> {
    fn duration(&self, animation: &K) -> Option<f32>;

    fn is_variable_length(&self, _animation: &K) -> bool {
        false
    }
}

impl<K, F> AnimationMetadata<K> for F
where
    F: Fn(&K) -> Option<f32>,
{
    fn duration(&self, animation: &K) -> Option<f32> {
        self(animation)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FragmentOptionIndex(u32);

impl FragmentOptionIndex {
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    fn select(self, option_count: usize) -> Option<usize> {
        let option_count = u32::try_from(option_count).ok()?;
        (option_count != 0).then(|| (self.0 % option_count) as usize)
    }
}

impl From<u32> for FragmentOptionIndex {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentQuery {
    pub fragment: Option<FragmentId>,
    pub tags: FragmentTagState,
    pub required_global_tags: TagState,
    pub option: FragmentOptionIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentSelection {
    pub tags: FragmentTagState,
    pub tag_set_index: usize,
    pub option_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaggedFragmentOptions<T> {
    pub tags: FragmentTagState,
    pub options: Vec<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentDatabaseEntry<T> {
    pub tag_sets: Vec<TaggedFragmentOptions<T>>,
}

impl<T> Default for FragmentDatabaseEntry<T> {
    fn default() -> Self {
        Self {
            tag_sets: Vec::new(),
        }
    }
}

impl<T> FragmentDatabaseEntry<T> {
    pub fn push(&mut self, tags: FragmentTagState, value: T) {
        if let Some(entry) = self.tag_sets.iter_mut().find(|entry| entry.tags == tags) {
            entry.options.push(value);
        } else {
            self.tag_sets.push(TaggedFragmentOptions {
                tags,
                options: vec![value],
            });
        }
    }

    /// Cry sorts by descending combined tag priority and preserves authoring
    /// order for equal scores.
    pub fn sort(&mut self, global: &TagDefinition, fragment: Option<&TagDefinition>) {
        let combined = fragment.map(|fragment| global.combined_priority_tallies(fragment));
        self.tag_sets.sort_by_key(|entry| {
            let global_score = combined.as_deref().map_or_else(
                || global.rate(entry.tags.global_tags),
                |tallies| global.rate_with_tallies(entry.tags.global_tags, tallies),
            );
            let fragment_score = fragment.map_or(0, |definition| {
                combined.as_deref().map_or_else(
                    || definition.rate(entry.tags.fragment_tags),
                    |tallies| definition.rate_with_tallies(entry.tags.fragment_tags, tallies),
                )
            });
            std::cmp::Reverse(global_score.saturating_add(fragment_score))
        });
    }

    fn best_match<'a>(
        &'a self,
        query: FragmentQuery,
        global: &TagDefinition,
        fragment: Option<&TagDefinition>,
    ) -> Option<(usize, &'a TaggedFragmentOptions<T>)> {
        self.tag_sets.iter().enumerate().find(|(_, entry)| {
            global.contains_required(entry.tags.global_tags, query.required_global_tags)
                && global.contains(query.tags.global_tags, entry.tags.global_tags)
                && fragment.is_none_or(|definition| {
                    definition.contains(query.tags.fragment_tags, entry.tags.fragment_tags)
                })
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentDatabase<T> {
    fragments: Vec<FragmentDatabaseEntry<T>>,
}

impl<T> FragmentDatabase<T> {
    #[must_use]
    pub fn new(fragment_count: usize) -> Self {
        Self {
            fragments: (0..fragment_count)
                .map(|_| FragmentDatabaseEntry::default())
                .collect(),
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.fragments.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    #[must_use]
    pub fn fragment(&self, fragment: FragmentId) -> Option<&FragmentDatabaseEntry<T>> {
        self.fragments.get(fragment.index())
    }

    pub fn fragment_mut(&mut self, fragment: FragmentId) -> Option<&mut FragmentDatabaseEntry<T>> {
        self.fragments.get_mut(fragment.index())
    }

    /// Iterates every fragment entry paired with its [`FragmentId`].
    ///
    /// # Panics
    ///
    /// The returned iterator panics if an entry's position is not a valid
    /// [`FragmentId`], which cannot happen for a database built by
    /// [`FragmentDatabase::new`].
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (FragmentId, &FragmentDatabaseEntry<T>)> {
        self.fragments.iter().enumerate().map(|(index, entry)| {
            (
                FragmentId::new(index).expect("validated fragment database index"),
                entry,
            )
        })
    }

    /// Appends one authored option for `fragment` under `tags`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFragmentId`] when `fragment` is outside the range this
    /// database was created with.
    pub fn push(
        &mut self,
        fragment: FragmentId,
        tags: FragmentTagState,
        value: T,
    ) -> Result<(), InvalidFragmentId> {
        self.fragment_mut(fragment)
            .ok_or(InvalidFragmentId(fragment))?
            .push(tags, value);
        Ok(())
    }

    /// Sorts every fragment's tag sets into Mannequin's selection order.
    ///
    /// # Panics
    ///
    /// Panics if an entry's position is not a valid [`FragmentId`], which
    /// cannot happen for a database built by [`FragmentDatabase::new`].
    pub fn sort(&mut self, definition: &impl MannequinDatabaseDefinition) {
        for (index, entry) in self.fragments.iter_mut().enumerate() {
            let fragment = FragmentId::new(index).expect("validated fragment database index");
            entry.sort(
                definition.global_tag_definition(),
                definition.fragment_tag_definition(fragment),
            );
        }
    }

    #[must_use]
    pub fn best_entry<'a>(
        &'a self,
        query: FragmentQuery,
        definition: &impl MannequinDatabaseDefinition,
    ) -> Option<(&'a T, FragmentSelection)> {
        let fragment = query.fragment?;
        let entry = self.fragment(fragment)?;
        let (tag_set_index, tag_set) = entry.best_match(
            query,
            definition.global_tag_definition(),
            definition.fragment_tag_definition(fragment),
        )?;
        let option_index = query.option.select(tag_set.options.len())?;
        Some((
            &tag_set.options[option_index],
            FragmentSelection {
                tags: tag_set.tags,
                tag_set_index,
                option_index,
            },
        ))
    }

    #[must_use]
    pub fn matching_option_count(
        &self,
        query: FragmentQuery,
        definition: &impl MannequinDatabaseDefinition,
    ) -> usize {
        let Some(fragment) = query.fragment else {
            return 0;
        };
        self.fragment(fragment)
            .and_then(|entry| {
                entry.best_match(
                    query,
                    definition.global_tag_definition(),
                    definition.fragment_tag_definition(fragment),
                )
            })
            .map_or(0, |(_, entry)| entry.options.len())
    }
}

/// Cry's complete animation database: authored fragments plus transition
/// fragments. A query selects and assembles both into hot runtime clip data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationDatabase<K, P> {
    pub fragments: FragmentDatabase<Fragment<K, P>>,
    pub transitions: TransitionDatabase<Fragment<K, P>>,
}

impl<K, P> AnimationDatabase<K, P> {
    #[must_use]
    pub fn new(fragment_count: usize) -> Self {
        Self {
            fragments: FragmentDatabase::new(fragment_count),
            transitions: TransitionDatabase::default(),
        }
    }

    pub fn sort(&mut self, definition: &impl MannequinDatabaseDefinition) {
        self.fragments.sort(definition);
        self.transitions.sort(definition);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseQueryResult<K, P> {
    pub data: FragmentData<K, P>,
    pub selection: Option<FragmentSelection>,
}

impl<K, P> AnimationDatabase<K, P>
where
    K: Clone,
    P: Clone,
{
    /// Port of `CAnimationDatabase::Query` and `AppendLayers`.
    ///
    /// The result contains at most two transition parts followed by the
    /// selected fragment. Layer entries are merged exactly once here; the
    /// scope sequencer consumes only this assembled representation.
    pub fn query(
        &self,
        blend_query: BlendQuery,
        option: FragmentOptionIndex,
        definition: &impl MannequinDatabaseDefinition,
        metadata: &impl AnimationMetadata<K>,
    ) -> DatabaseQueryResult<K, P> {
        let blends = if blend_query.flags.contains(BlendQueryFlags::NO_TRANSITIONS) {
            BlendSelections {
                primary: None,
                secondary: None,
            }
        } else {
            self.transitions.find_best_blends(blend_query, definition)
        };

        let selected = blend_query
            .flags
            .contains(BlendQueryFlags::TO_INSTALLED)
            .then(|| {
                self.fragments.best_entry(
                    FragmentQuery {
                        fragment: blend_query.fragment_to,
                        tags: blend_query.tag_state_to,
                        required_global_tags: blend_query.additional_tags,
                        option,
                    },
                    definition,
                )
            })
            .flatten();

        let mut data = FragmentData {
            is_one_shot: true,
            ..FragmentData::default()
        };
        let mut time_tally = 0.0;
        let mut time_offset = 0.0;
        let mut part = 0_u8;

        for selection in [blends.primary, blends.secondary].into_iter().flatten() {
            Self::append_transition(
                &mut data,
                selection.blend,
                part,
                &mut time_offset,
                &mut time_tally,
                metadata,
            );
            part += 1;
        }

        let selection = selected.map(|(fragment, selection)| {
            data.blend_out_duration = fragment.blend_out_duration;
            let duration = append_layers(
                &mut data,
                fragment,
                LayerAppendContext {
                    part,
                    start_time: time_tally,
                    start_offset: time_offset,
                    is_blend: false,
                },
                metadata,
            );
            data.durations[usize::from(part)] = duration;
            data.part_types[usize::from(part)] = ClipType::Normal;
            data.sequence_flags.insert(FragmentSequenceFlags::FRAGMENT);
            selection
        });

        DatabaseQueryResult { data, selection }
    }

    fn append_transition(
        data: &mut FragmentData<K, P>,
        blend: &FragmentBlend<Fragment<K, P>>,
        part: u8,
        time_offset: &mut f32,
        time_tally: &mut f32,
        metadata: &impl AnimationMetadata<K>,
    ) {
        let duration = append_layers(
            data,
            &blend.fragment,
            LayerAppendContext {
                part,
                start_time: *time_tally,
                start_offset: 0.0,
                is_blend: true,
            },
            metadata,
        );
        *time_offset = blend.enter_time;
        *time_tally += duration;

        let (clip_type, sequence_flag) = if blend.flags.contains(TransitionFlags::OUTRO) {
            (
                ClipType::TransitionOutro,
                FragmentSequenceFlags::TRANSITION_OUTRO,
            )
        } else {
            (ClipType::Transition, FragmentSequenceFlags::TRANSITION)
        };
        let index = usize::from(part);
        data.part_types[index] = clip_type;
        data.durations[index] += duration;
        data.sequence_flags.insert(sequence_flag);
    }
}

/// Inputs that stay constant while one fragment part is merged into a
/// [`FragmentData`] sequence.
#[derive(Debug, Clone, Copy)]
struct LayerAppendContext {
    part: u8,
    start_time: f32,
    start_offset: f32,
    is_blend: bool,
}

/// Where one authored layer lands in its destination layer, plus whether the
/// running total is still being accumulated for it.
#[derive(Debug, Clone, Copy)]
struct LayerPlacement {
    layer_index: usize,
    start_index: usize,
    install_len: usize,
    should_override: bool,
    calculate_time: bool,
}

fn append_layers<K, P>(
    data: &mut FragmentData<K, P>,
    fragment: &Fragment<K, P>,
    context: LayerAppendContext,
    metadata: &impl AnimationMetadata<K>,
) -> f32
where
    K: Clone,
    P: Clone,
{
    let mut total_time = 0.0;
    let mut calculate_time = true;

    if data.animation_layers.len() < fragment.animation_layers.len() {
        data.animation_layers
            .resize_with(fragment.animation_layers.len(), Vec::new);
    }
    if data.procedural_layers.len() < fragment.procedural_layers.len() {
        data.procedural_layers
            .resize_with(fragment.procedural_layers.len(), Vec::new);
    }

    append_animation_layers(
        data,
        fragment,
        context,
        &mut total_time,
        &mut calculate_time,
        metadata,
    );
    patch_trailing_animation_layers(data, fragment, context);
    append_procedural_layers(
        data,
        fragment,
        context,
        &mut total_time,
        &mut calculate_time,
    );
    patch_trailing_procedural_layers(data, fragment, context);

    total_time - context.start_time
}

/// Merges every authored animation layer into `data`, advancing the shared
/// running total and clearing the `calculate_time` latch exactly once per
/// layer, the way Cry's fragment assembly does.
fn append_animation_layers<K, P>(
    data: &mut FragmentData<K, P>,
    fragment: &Fragment<K, P>,
    context: LayerAppendContext,
    total_time: &mut f32,
    calculate_time: &mut bool,
    metadata: &impl AnimationMetadata<K>,
) where
    K: Clone,
{
    for (layer_index, source_layer) in fragment.animation_layers.iter().enumerate() {
        let destination = &mut data.animation_layers[layer_index];
        let old_len = destination.len();
        let had_entry = old_len != 0;
        let has_new_entry = !source_layer.is_empty();
        let should_override = had_entry
            && has_new_entry
            && (destination[old_len - 1].blend.exit_time >= context.start_time
                || destination[old_len - 1].blend.terminal);
        let start_index = old_len - usize::from(should_override);
        let install_len = if context.is_blend {
            source_layer.len().max(1)
        } else {
            source_layer.len()
        };
        destination.resize_with(start_index + install_len, AnimationClip::default);

        let placement = LayerPlacement {
            layer_index,
            start_index,
            install_len,
            should_override,
            calculate_time: *calculate_time,
        };
        let last_duration =
            append_animation_clips(data, source_layer, placement, context, total_time, metadata);

        if *calculate_time && !context.is_blend {
            *total_time += last_duration;
        }
        *calculate_time = false;
    }
}

/// Writes one authored animation layer's clips into their destination slots and
/// returns the reference length of the final clip.
#[expect(
    clippy::useless_let_if_seq,
    reason = "`animation_duration` and `variable_length` are both written by the same \
              let-chain, so folding either into an `if` expression would evaluate the \
              metadata lookups twice"
)]
fn append_animation_clips<K, P>(
    data: &mut FragmentData<K, P>,
    source_layer: &[AnimationClip<K>],
    placement: LayerPlacement,
    context: LayerAppendContext,
    total_time: &mut f32,
    metadata: &impl AnimationMetadata<K>,
) -> f32
where
    K: Clone,
{
    let destination = &mut data.animation_layers[placement.layer_index];
    let mut last_duration = 0.0;
    let mut layer_total_time = 0.0;
    for offset in 0..placement.install_len {
        let first = offset == 0;
        let source = source_layer.get(offset).cloned().unwrap_or_else(|| {
            let mut empty = AnimationClip::default();
            empty.blend.exit_time = 0.0;
            empty
        });
        let clip = &mut destination[placement.start_index + offset];
        clip.animation = source.animation;
        clip.part = context.part;

        if placement.should_override && first {
            if context.is_blend {
                let old_exit_time = clip.blend.exit_time;
                clip.blend = source.blend;
                clip.blend.exit_time = old_exit_time;
                clip.blend_part = context.part;
            }
        } else {
            clip.blend = source.blend;
            let mut blend_part = context.part;
            if first {
                clip.blend.exit_time = clip.blend.exit_time.max(0.0) + context.start_time;
                if !context.is_blend {
                    blend_part = clip.blend_part;
                }
            }
            clip.blend_part = blend_part;
        }
        clip.animation.flags |= clip.blend.flags;

        let mut animation_duration = 0.0;
        let mut variable_length = false;
        if !clip
            .animation
            .flags
            .contains(AnimationFlags::LOOP_ANIMATION)
            && clip.animation.playback_speed > 0.0
            && let Some(animation) = clip.animation.animation.as_ref()
            && let Some(duration) = metadata.duration(animation)
        {
            animation_duration = (duration - clip.blend.start_time) / clip.animation.playback_speed;
            variable_length = metadata.is_variable_length(animation);
        }
        clip.reference_length = animation_duration;
        clip.variable_length = variable_length;

        if clip.blend.exit_time < 0.0 {
            clip.blend.exit_time = last_duration;
        }
        if placement.calculate_time {
            *total_time += clip.blend.exit_time;
        }

        if !context.is_blend {
            let previous_start_time = layer_total_time;
            layer_total_time += clip.blend.exit_time;
            let animation_start_time = layer_total_time;
            if placement.layer_index == 0 {
                data.is_one_shot = !clip
                    .animation
                    .flags
                    .contains(AnimationFlags::LOOP_ANIMATION);
            }
            if context.start_offset > animation_start_time {
                let animation_start_offset = context.start_offset - animation_start_time;
                clip.blend.start_time += animation_start_offset / animation_duration.max(0.001);
                clip.blend.start_time = clip.blend.start_time.clamp(0.0, 1.0);
                clip.blend.exit_time = 0.0;
            } else if context.start_offset > previous_start_time {
                clip.blend.exit_time =
                    (clip.blend.exit_time - (context.start_offset - previous_start_time)).max(0.0);
            }
        }
        last_duration = animation_duration;
    }
    last_duration
}

/// Terminates animation layers this fragment does not author so the previous
/// part's clips stop on this part's boundary.
fn patch_trailing_animation_layers<K, P>(
    data: &mut FragmentData<K, P>,
    fragment: &Fragment<K, P>,
    context: LayerAppendContext,
) {
    for destination in data
        .animation_layers
        .iter_mut()
        .skip(fragment.animation_layers.len())
    {
        let Some(last) = destination.last_mut() else {
            continue;
        };
        if last.blend.exit_time >= context.start_time || last.blend.terminal {
            last.part = context.part;
        } else {
            let previous_part = last.part;
            let mut empty = AnimationClip::default();
            empty.blend.exit_time = context.start_time;
            empty.blend_part = previous_part;
            empty.part = context.part;
            destination.push(empty);
        }
    }
}

/// Merges every authored procedural layer into `data`, mirroring
/// [`append_animation_layers`] without any animation-duration lookups.
fn append_procedural_layers<K, P>(
    data: &mut FragmentData<K, P>,
    fragment: &Fragment<K, P>,
    context: LayerAppendContext,
    total_time: &mut f32,
    calculate_time: &mut bool,
) where
    P: Clone,
{
    for (layer_index, source_layer) in fragment.procedural_layers.iter().enumerate() {
        let destination = &mut data.procedural_layers[layer_index];
        let old_len = destination.len();
        let had_entry = old_len != 0;
        let has_new_entry = !source_layer.is_empty();
        let should_override = had_entry
            && has_new_entry
            && (destination[old_len - 1].blend.exit_time >= context.start_time
                || destination[old_len - 1].blend.terminal);
        let start_index = old_len - usize::from(should_override);
        let install_len = if context.is_blend {
            source_layer.len().max(1)
        } else {
            source_layer.len()
        };
        destination.resize_with(start_index + install_len, ProceduralEntry::default);

        let mut layer_total_time = 0.0;
        for offset in 0..install_len {
            let first = offset == 0;
            let source = source_layer.get(offset).cloned().unwrap_or_else(|| {
                let mut empty = ProceduralEntry::default();
                empty.blend.exit_time = 0.0;
                empty
            });
            let clip = &mut destination[start_index + offset];
            clip.parameters = source.parameters;
            clip.part = context.part;

            if should_override && first {
                if context.is_blend {
                    let old_exit_time = clip.blend.exit_time;
                    clip.blend = source.blend;
                    clip.blend.exit_time = old_exit_time;
                    clip.blend_part = context.part;
                }
            } else {
                clip.blend = source.blend;
                let mut blend_part = context.part;
                if first {
                    clip.blend.exit_time = clip.blend.exit_time.max(0.0) + context.start_time;
                    if !context.is_blend {
                        blend_part = clip.blend_part;
                    }
                }
                clip.blend_part = blend_part;
            }

            if *calculate_time {
                *total_time += clip.blend.exit_time;
            }
            if !context.is_blend {
                let previous_start_time = layer_total_time;
                layer_total_time += clip.blend.exit_time;
                let layer_start_time = layer_total_time;
                if context.start_offset > layer_start_time {
                    clip.blend.exit_time = 0.0;
                } else if context.start_offset > previous_start_time {
                    clip.blend.exit_time = (clip.blend.exit_time
                        - (context.start_offset - previous_start_time))
                        .max(0.0);
                }
            }
        }
        *calculate_time = false;
    }
}

/// Terminates procedural layers this fragment does not author so the previous
/// part's clips stop on this part's boundary.
fn patch_trailing_procedural_layers<K, P>(
    data: &mut FragmentData<K, P>,
    fragment: &Fragment<K, P>,
    context: LayerAppendContext,
) {
    for destination in data
        .procedural_layers
        .iter_mut()
        .skip(fragment.procedural_layers.len())
    {
        let Some(last) = destination.last_mut() else {
            continue;
        };
        if last.blend.exit_time >= context.start_time || last.blend.terminal {
            last.part = context.part;
        } else {
            let previous_part = last.part;
            let mut empty = ProceduralEntry::default();
            empty.blend.exit_time = context.start_time;
            empty.blend_part = previous_part;
            empty.part = context.part;
            destination.push(empty);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("fragment id {0:?} is not in this database")]
pub struct InvalidFragmentId(pub FragmentId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TransitionKey {
    pub from: Option<FragmentId>,
    pub to: Option<FragmentId>,
}

impl TransitionKey {
    #[must_use]
    pub const fn new(from: Option<FragmentId>, to: Option<FragmentId>) -> Self {
        Self { from, to }
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FragmentBlendUid(Uuid);

impl FragmentBlendUid {
    /// Cry constructs a new random UUID when an ADB transition is loaded; the
    /// editable ADB XML deliberately does not persist this handle.
    #[must_use]
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn new(value: Uuid) -> Option<Self> {
        if value.is_nil() {
            None
        } else {
            Some(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> Uuid {
        self.0
    }
}

impl From<FragmentBlendUid> for Uuid {
    fn from(value: FragmentBlendUid) -> Self {
        value.get()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FragmentBlend<T> {
    pub select_time: f32,
    pub start_time: f32,
    pub enter_time: f32,
    pub fragment: T,
    pub flags: TransitionFlags,
    #[serde(skip, default = "FragmentBlendUid::random")]
    pub uid: FragmentBlendUid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FragmentBlendVariant<T> {
    pub tags_from: FragmentTagState,
    pub tags_to: FragmentTagState,
    pub blends: Vec<FragmentBlend<T>>,
}

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct BlendQueryFlags: u32 {
        const FROM_INSTALLED = 1 << 0;
        const TO_INSTALLED = 1 << 1;
        const HIGHER_PRIORITY = 1 << 2;
        const NO_TRANSITIONS = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendQuery {
    pub fragment_from: Option<FragmentId>,
    pub fragment_to: Option<FragmentId>,
    pub tag_state_from: FragmentTagState,
    pub tag_state_to: FragmentTagState,
    pub additional_tags: TagState,
    pub fragment_time: f32,
    pub previous_normalized_time: f32,
    pub normalized_time: f32,
    pub flags: BlendQueryFlags,
    pub forced_blend: Option<FragmentBlendUid>,
}

#[derive(Debug)]
pub struct BlendSelection<'a, T> {
    pub key: TransitionKey,
    pub tags_from: FragmentTagState,
    pub tags_to: FragmentTagState,
    pub blend_index: usize,
    pub blend: &'a FragmentBlend<T>,
}

#[derive(Debug)]
pub struct BlendSelections<'a, T> {
    pub primary: Option<BlendSelection<'a, T>>,
    pub secondary: Option<BlendSelection<'a, T>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct TransitionDatabase<T> {
    #[serde(with = "transition_entries")]
    entries: Vec<TransitionEntry<T>>,
}

#[derive(Debug, Clone, PartialEq)]
struct TransitionEntry<T> {
    key: TransitionKey,
    variants: Vec<FragmentBlendVariant<T>>,
}

mod transition_entries {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

    use super::{FragmentBlendVariant, TransitionEntry, TransitionKey};

    pub fn serialize<T, S>(entries: &[TransitionEntry<T>], serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for entry in entries {
            map.serialize_entry(&entry.key, &entry.variants)?;
        }
        map.end()
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Vec<TransitionEntry<T>>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let entries =
            BTreeMap::<TransitionKey, Vec<FragmentBlendVariant<T>>>::deserialize(deserializer)?;
        Ok(entries
            .into_iter()
            .map(|(key, variants)| TransitionEntry { key, variants })
            .collect())
    }
}

impl<T> Default for TransitionDatabase<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> TransitionDatabase<T> {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn variants(&self, key: TransitionKey) -> Option<&[FragmentBlendVariant<T>]> {
        self.entry(key).map(|entry| entry.variants.as_slice())
    }

    /// Iterates every transition key paired with its authored blend variants.
    #[must_use]
    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = (TransitionKey, &[FragmentBlendVariant<T>])> {
        self.entries
            .iter()
            .map(|entry| (entry.key, entry.variants.as_slice()))
    }

    /// Inserts `blend` under `key` for the given from/to tag pair, creating the
    /// entry and variant when they do not exist yet.
    ///
    /// # Panics
    ///
    /// Never in practice: the entry index used to look the variant list back up
    /// is the one just returned or inserted by the binary search.
    pub fn push(
        &mut self,
        key: TransitionKey,
        tags_from: FragmentTagState,
        tags_to: FragmentTagState,
        blend: FragmentBlend<T>,
    ) {
        let entry_index = match self.entries.binary_search_by_key(&key, |entry| entry.key) {
            Ok(index) => index,
            Err(index) => {
                self.entries.insert(
                    index,
                    TransitionEntry {
                        key,
                        variants: Vec::new(),
                    },
                );
                index
            }
        };
        let variants = &mut self.entries[entry_index].variants;
        let variant = if let Some(index) = variants
            .iter()
            .position(|variant| variant.tags_from == tags_from && variant.tags_to == tags_to)
        {
            &mut variants[index]
        } else {
            variants.push(FragmentBlendVariant {
                tags_from,
                tags_to,
                blends: Vec::new(),
            });
            variants.last_mut().expect("variant was just appended")
        };
        variant.blends.push(blend);
        // Shipping operator< compares selectTime. Cookers reject non-finite
        // authored times; preserving order for an invalid NaN is deterministic.
        variant.blends.sort_by(|left, right| {
            left.select_time
                .partial_cmp(&right.select_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Stable-sort transition variants exactly like
    /// `CAnimationDatabase::SCompareBlendVariantFunctor`.
    pub fn sort(&mut self, definition: &impl MannequinDatabaseDefinition) {
        for entry in &mut self.entries {
            let from = entry
                .key
                .from
                .and_then(|fragment| definition.fragment_tag_definition(fragment));
            let to = entry
                .key
                .to
                .and_then(|fragment| definition.fragment_tag_definition(fragment));
            entry.variants.sort_by_key(|variant| {
                std::cmp::Reverse(Self::variant_priority(
                    variant,
                    definition.global_tag_definition(),
                    from,
                    to,
                ))
            });
        }
    }

    #[must_use]
    pub fn find_best_blends<'a>(
        &'a self,
        query: BlendQuery,
        definition: &impl MannequinDatabaseDefinition,
    ) -> BlendSelections<'a, T> {
        let exact = TransitionKey::new(query.fragment_from, query.fragment_to);
        let exit = TransitionKey::new(query.fragment_from, None);
        let entry = TransitionKey::new(None, query.fragment_to);
        let wildcard = TransitionKey::new(None, None);

        let exact_viable = query
            .flags
            .intersects(BlendQueryFlags::FROM_INSTALLED | BlendQueryFlags::TO_INSTALLED)
            && query.fragment_from.is_some()
            && query.fragment_to.is_some();
        let exit_viable = query.flags.contains(BlendQueryFlags::FROM_INSTALLED)
            && query.fragment_from.is_some()
            && !query.flags.contains(BlendQueryFlags::HIGHER_PRIORITY);
        let entry_viable =
            query.flags.contains(BlendQueryFlags::TO_INSTALLED) && query.fragment_to.is_some();
        let wildcard_viable = query.flags.contains(BlendQueryFlags::TO_INSTALLED);

        let candidates = [
            (exact, exact_viable),
            (exit, exit_viable),
            (entry, entry_viable),
            (wildcard, wildcard_viable),
        ];
        for (index, (key, viable)) in candidates.into_iter().enumerate() {
            if !viable {
                continue;
            }
            let Some(primary_variant) = self.find_best_variant(key, query, definition) else {
                continue;
            };
            let primary = Self::find_best_blend_in_variant(key, primary_variant, query);
            let secondary = if index == 1 && entry_viable {
                self.find_best_variant(entry, query, definition)
                    .and_then(|variant| Self::find_best_blend_in_variant(entry, variant, query))
            } else {
                None
            };
            return BlendSelections { primary, secondary };
        }

        BlendSelections {
            primary: None,
            secondary: None,
        }
    }

    fn variant_priority(
        variant: &FragmentBlendVariant<T>,
        global: &TagDefinition,
        from: Option<&TagDefinition>,
        to: Option<&TagDefinition>,
    ) -> u32 {
        global
            .rate(variant.tags_from.global_tags)
            .saturating_add(from.map_or(0, |definition| {
                definition.rate(variant.tags_from.fragment_tags)
            }))
            .saturating_add(global.rate(variant.tags_to.global_tags))
            .saturating_add(to.map_or(0, |definition| {
                definition.rate(variant.tags_to.fragment_tags)
            }))
    }

    fn find_best_variant(
        &self,
        key: TransitionKey,
        query: BlendQuery,
        definition: &impl MannequinDatabaseDefinition,
    ) -> Option<&FragmentBlendVariant<T>> {
        let from = key
            .from
            .and_then(|fragment| definition.fragment_tag_definition(fragment));
        let to = key
            .to
            .and_then(|fragment| definition.fragment_tag_definition(fragment));
        let global = definition.global_tag_definition();

        self.entry(key)?.variants.iter().find(|variant| {
            global.contains(
                query.tag_state_from.global_tags,
                variant.tags_from.global_tags,
            ) && global.contains(query.tag_state_to.global_tags, variant.tags_to.global_tags)
                && global.contains_required(variant.tags_from.global_tags, query.additional_tags)
                && global.contains_required(variant.tags_to.global_tags, query.additional_tags)
                && from.is_none_or(|definition| {
                    definition.contains(
                        query.tag_state_from.fragment_tags,
                        variant.tags_from.fragment_tags,
                    )
                })
                && to.is_none_or(|definition| {
                    definition.contains(
                        query.tag_state_to.fragment_tags,
                        variant.tags_to.fragment_tags,
                    )
                })
        })
    }

    #[inline]
    fn entry(&self, key: TransitionKey) -> Option<&TransitionEntry<T>> {
        self.entries
            .binary_search_by_key(&key, |entry| entry.key)
            .ok()
            .map(|index| &self.entries[index])
    }

    fn find_best_blend_in_variant(
        key: TransitionKey,
        variant: &FragmentBlendVariant<T>,
        query: BlendQuery,
    ) -> Option<BlendSelection<'_, T>> {
        let forced_index = query
            .forced_blend
            .and_then(|uid| variant.blends.iter().position(|blend| blend.uid == uid));
        let blend_index = forced_index.or_else(|| {
            let mut selected = None;
            for (index, blend) in variant.blends.iter().enumerate() {
                let source_time = if blend.flags.contains(TransitionFlags::CYCLE_LOCKED) {
                    query.previous_normalized_time
                } else if blend.flags.contains(TransitionFlags::CYCLIC) {
                    query.normalized_time
                } else {
                    query.fragment_time
                };
                if selected.is_none() || source_time >= blend.select_time {
                    selected = Some(index);
                }
            }
            selected
        })?;

        Some(BlendSelection {
            key,
            tags_from: variant.tags_from,
            tags_to: variant.tags_to,
            blend_index,
            blend: &variant.blends[blend_index],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Definition {
        global: TagDefinition,
        fragment: TagDefinition,
    }

    impl MannequinDatabaseDefinition for Definition {
        fn global_tag_definition(&self) -> &TagDefinition {
            &self.global
        }

        fn fragment_tag_definition(&self, _fragment: FragmentId) -> Option<&TagDefinition> {
            Some(&self.fragment)
        }
    }

    #[test]
    fn selection_uses_priority_then_stable_authoring_order() {
        let mut global = TagDefinition::builder();
        let locomotion = global.add_group();
        let idle = global.add_tag(Some(locomotion), 0).unwrap();
        let run = global.add_tag(Some(locomotion), 0).unwrap();
        let injured = global.add_tag(None, 10).unwrap();
        let global = global.build().unwrap();

        let mut fragment = TagDefinition::builder();
        let left = fragment.add_tag(None, 1).unwrap();
        let fragment = fragment.build().unwrap();
        let definition = Definition { global, fragment };

        let mut database = FragmentDatabase::new(1);
        let fragment_id = FragmentId::new(0).unwrap();
        let mut default = FragmentTagState::default();
        definition
            .global
            .set(&mut default.global_tags, idle, true)
            .unwrap();
        database.push(fragment_id, default, "idle").unwrap();

        let mut specific = default;
        definition
            .global
            .set(&mut specific.global_tags, injured, true)
            .unwrap();
        definition
            .fragment
            .set(&mut specific.fragment_tags, left, true)
            .unwrap();
        database
            .push(fragment_id, specific, "injured-left")
            .unwrap();
        database.sort(&definition);

        let mut query_tags = specific;
        definition
            .global
            .set(&mut query_tags.global_tags, run, true)
            .unwrap();
        let query = FragmentQuery {
            fragment: Some(fragment_id),
            tags: specific,
            required_global_tags: TagState::EMPTY,
            option: FragmentOptionIndex::new(0),
        };
        let (selected, selection) = database.best_entry(query, &definition).unwrap();
        assert_eq!(*selected, "injured-left");
        assert_eq!(selection.tags, specific);

        assert!(definition.global.is_set(query_tags.global_tags, run));
    }

    #[test]
    fn option_index_wraps_like_shipping_get_best_entry() {
        let global = TagDefinition::builder().build().unwrap();
        let fragment = TagDefinition::builder().build().unwrap();
        let definition = Definition { global, fragment };
        let fragment_id = FragmentId::new(0).unwrap();
        let mut database = FragmentDatabase::new(1);
        database
            .push(fragment_id, FragmentTagState::default(), 10)
            .unwrap();
        database
            .push(fragment_id, FragmentTagState::default(), 20)
            .unwrap();

        let (selected, selection) = database
            .best_entry(
                FragmentQuery {
                    fragment: Some(fragment_id),
                    option: FragmentOptionIndex::new(3),
                    ..FragmentQuery::default()
                },
                &definition,
            )
            .unwrap();
        assert_eq!(*selected, 20);
        assert_eq!(selection.option_index, 1);
    }

    fn blend(
        uid: u128,
        select_time: f32,
        flags: TransitionFlags,
        value: &'static str,
    ) -> FragmentBlend<&'static str> {
        FragmentBlend {
            select_time,
            start_time: 0.0,
            enter_time: 0.0,
            fragment: value,
            flags,
            uid: FragmentBlendUid::new(Uuid::from_u128(uid)).unwrap(),
        }
    }

    #[test]
    fn transition_fallback_can_pair_exit_and_entry_blends() {
        let global = TagDefinition::builder().build().unwrap();
        let fragment = TagDefinition::builder().build().unwrap();
        let definition = Definition { global, fragment };
        let from = FragmentId::new(0).unwrap();
        let to = FragmentId::new(1).unwrap();
        let mut database = TransitionDatabase::default();
        database.push(
            TransitionKey::new(Some(from), None),
            FragmentTagState::default(),
            FragmentTagState::default(),
            blend(1, 0.0, TransitionFlags::OUTRO, "exit"),
        );
        database.push(
            TransitionKey::new(None, Some(to)),
            FragmentTagState::default(),
            FragmentTagState::default(),
            blend(2, 0.0, TransitionFlags::empty(), "entry"),
        );

        let selections = database.find_best_blends(
            BlendQuery {
                fragment_from: Some(from),
                fragment_to: Some(to),
                flags: BlendQueryFlags::FROM_INSTALLED | BlendQueryFlags::TO_INSTALLED,
                ..BlendQuery::default()
            },
            &definition,
        );
        assert_eq!(selections.primary.unwrap().blend.fragment, "exit");
        assert_eq!(selections.secondary.unwrap().blend.fragment, "entry");
    }

    #[test]
    fn transition_time_and_forced_uid_match_shipping_selection() {
        let global = TagDefinition::builder().build().unwrap();
        let fragment = TagDefinition::builder().build().unwrap();
        let definition = Definition { global, fragment };
        let from = FragmentId::new(0).unwrap();
        let to = FragmentId::new(1).unwrap();
        let key = TransitionKey::new(Some(from), Some(to));
        let forced = FragmentBlendUid::new(Uuid::from_u128(12)).unwrap();
        let mut database = TransitionDatabase::default();
        database.push(
            key,
            FragmentTagState::default(),
            FragmentTagState::default(),
            blend(11, 0.2, TransitionFlags::empty(), "early"),
        );
        database.push(
            key,
            FragmentTagState::default(),
            FragmentTagState::default(),
            FragmentBlend {
                uid: forced,
                ..blend(12, 0.8, TransitionFlags::CYCLE_LOCKED, "cycle-locked")
            },
        );

        let query = BlendQuery {
            fragment_from: Some(from),
            fragment_to: Some(to),
            fragment_time: 0.3,
            previous_normalized_time: 0.9,
            normalized_time: 0.1,
            flags: BlendQueryFlags::FROM_INSTALLED | BlendQueryFlags::TO_INSTALLED,
            ..BlendQuery::default()
        };
        let selected = database
            .find_best_blends(query, &definition)
            .primary
            .unwrap();
        assert_eq!(selected.blend.fragment, "cycle-locked");

        let forced_query = BlendQuery {
            previous_normalized_time: 0.0,
            forced_blend: Some(forced),
            ..query
        };
        let forced_selection = database
            .find_best_blends(forced_query, &definition)
            .primary
            .unwrap();
        assert_eq!(forced_selection.blend.uid, forced);
    }
}
