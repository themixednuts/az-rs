//! Fixed-tick historical input assets and ECS runtime.

mod codec;

pub use codec::{
    InputMapAssetLoader, InputMapFormatError, read_input_map_asset, write_input_map_asset,
};

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use az_asset::AssetRef;
use az_core::{
    AssetData, AssetTypeRegistration, AzAssetData, ComponentLoweringRegistration, ReflectAzRtti,
    ReflectAzTypeInfo, crc::Crc32, rtti::AzTypeRegistration,
};
use az_derive::{AzComponent, AzRtti};
#[cfg(feature = "runtime")]
use az_framework::asset::AssetCatalog;
use az_gem_contract::{Contribution, GemContext, contribution};
use bevy::{
    asset::{AssetApp, AssetLoadFailedEvent},
    ecs::reflect::ReflectComponent,
    prelude::{
        Added, App, Asset, AssetServer, Assets, Changed, Commands, Component, Entity, Handle,
        IntoScheduleConfigs, MessageReader, Or, Plugin, Query, Reflect, Res, Update,
    },
    reflect::{ReflectDeserialize, ReflectSerialize, std_traits::ReflectDefault},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::{Uuid, uuid};

pub const HISTORICAL_INPUT_SYSTEM_COMPONENT_TYPE_ID: Uuid =
    uuid!("548C4C30-FE22-499D-8192-89C555C67242");
pub const INPUT_MAP_ASSET_TYPE_ID: Uuid = uuid!("C6ECD1FF-7B27-40C7-8104-5E1775EB8D16");
pub const INPUT_MAP_ASSET_VERSION: u32 = 1;
pub const INPUT_MAP_ASSET_STABLE_NAME: &str = "azoth.historical_input.input_map";

/// Process-level Historical Input service component.
#[derive(Component, AzComponent, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
#[az_component(
    name = "HistoricalInputSystemComponent",
    HISTORICAL_INPUT_SYSTEM_COMPONENT_TYPE_ID
)]
#[reflect(Component, Default)]
pub struct HistoricalInputSystemComponent;

/// Reflected mapping from input-name CRCs to dense input indices.
#[derive(Asset, AzRtti, Debug, Default, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "InputMapAsset", INPUT_MAP_ASSET_TYPE_ID, AzAssetData, register)]
#[reflect(Default)]
pub struct InputMapAsset {
    #[serde(rename = "NameMap", default)]
    pub name_map: Vec<String>,
    #[serde(rename = "InputMap", default)]
    pub input_map: HashMap<u32, i32>,
}

/// Asset-backed configuration that installs history on an ECS entity.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct HistoricalInputContext {
    pub input_map: AssetRef<InputMapAsset>,
    pub history_capacity: u32,
}

/// Terminal failure to resolve or compile an entity's historical input map.
///
/// Runtime portrayal code consumes this component instead of waiting forever
/// for a [`HistoricalInputBuffer`] that cannot be constructed.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct HistoricalInputContextFailed {
    message: String,
}

impl HistoricalInputContextFailed {
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl HistoricalInputContext {
    #[inline]
    #[must_use]
    pub const fn new(input_map: AssetRef<InputMapAsset>) -> Self {
        Self {
            input_map,
            history_capacity: 1,
        }
    }

    #[inline]
    #[must_use]
    pub const fn history_capacity(mut self, history_capacity: u32) -> Self {
        self.history_capacity = if history_capacity == 0 {
            1
        } else {
            history_capacity
        };
        self
    }

    #[inline]
    #[must_use]
    fn normalized_capacity(&self) -> HistoryCapacity {
        let capacity = usize::try_from(self.history_capacity.max(1))
            .expect("u32 historical input capacity must fit usize");
        HistoryCapacity::new(NonZeroUsize::new(capacity).expect("capacity is non-zero"))
    }
}

impl AssetData for InputMapAsset {
    const STABLE_NAME: &'static str = INPUT_MAP_ASSET_STABLE_NAME;
}

/// Dense index into an [`InputMapLayout`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect)]
#[reflect(opaque)]
pub struct InputIndex(u32);

impl InputIndex {
    #[inline]
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<InputIndex> for u32 {
    #[inline]
    fn from(value: InputIndex) -> Self {
        value.get()
    }
}

impl From<InputIndex> for usize {
    #[inline]
    fn from(value: InputIndex) -> Self {
        value.index()
    }
}

impl TryFrom<i32> for InputIndex {
    type Error = InputIndexError;

    #[inline]
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        u32::try_from(value)
            .map(Self)
            .map_err(|_| InputIndexError(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("input index {0} is negative")]
pub struct InputIndexError(i32);

/// Immutable, shareable lookup data compiled from an [`InputMapAsset`].
#[derive(Debug)]
pub struct InputMapLayout {
    names: Box<[Box<str>]>,
    indices: HashMap<u32, InputIndex>,
}

impl InputMapLayout {
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn names(&self) -> &[Box<str>] {
        &self.names
    }

    #[inline]
    #[must_use]
    pub fn name(&self, index: InputIndex) -> Option<&str> {
        self.names.get(index.index()).map(AsRef::as_ref)
    }

    #[inline]
    #[must_use]
    pub fn resolve_crc(&self, input: Crc32) -> Option<InputIndex> {
        self.indices.get(&input.value()).copied()
    }

    #[inline]
    #[must_use]
    pub fn resolve(&self, input: impl AsRef<str>) -> Option<InputIndex> {
        self.resolve_crc(Crc32::from_str_lower(input.as_ref()))
    }
}

impl TryFrom<&InputMapAsset> for InputMapLayout {
    type Error = InputMapLayoutError;

    fn try_from(asset: &InputMapAsset) -> Result<Self, Self::Error> {
        let input_count = asset.name_map.len();
        let mut indices = HashMap::with_capacity(asset.input_map.len());
        for (&crc, &raw_index) in &asset.input_map {
            let index = InputIndex::try_from(raw_index).map_err(|_| {
                InputMapLayoutError::NegativeIndex {
                    crc,
                    index: raw_index,
                }
            })?;
            if index.index() >= input_count {
                return Err(InputMapLayoutError::IndexOutOfBounds {
                    crc,
                    index: index.get(),
                    input_count,
                });
            }
            indices.insert(crc, index);
        }
        Ok(Self {
            names: asset
                .name_map
                .iter()
                .map(|name| name.clone().into_boxed_str())
                .collect(),
            indices,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InputMapLayoutError {
    #[error("input CRC {crc:#010x} maps to negative index {index}")]
    NegativeIndex { crc: u32, index: i32 },
    #[error(
        "input CRC {crc:#010x} maps to index {index}, but the name map contains {input_count} inputs"
    )]
    IndexOutOfBounds {
        crc: u32,
        index: u32,
        input_count: usize,
    },
}

/// Number of stored ring slots. One slot is reserved as the scan sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryCapacity(NonZeroUsize);

impl HistoryCapacity {
    pub const CURRENT_ONLY: Self = Self(NonZeroUsize::MIN);

    #[inline]
    #[must_use]
    pub const fn new(capacity: NonZeroUsize) -> Self {
        Self(capacity)
    }

    #[inline]
    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

impl Default for HistoryCapacity {
    fn default() -> Self {
        Self::CURRENT_ONLY
    }
}

impl From<NonZeroUsize> for HistoryCapacity {
    #[inline]
    fn from(value: NonZeroUsize) -> Self {
        Self::new(value)
    }
}

/// Number of prior ticks included by a historical query.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Reflect,
    Serialize,
    Deserialize,
)]
#[reflect(opaque, Serialize, Deserialize)]
pub struct BackTicks(u32);

impl BackTicks {
    #[inline]
    #[must_use]
    pub const fn new(ticks: u32) -> Self {
        Self(ticks)
    }

    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Convert a float tick count, clamping below at `0` and saturating at
    /// [`u32::MAX`].
    #[inline]
    #[must_use]
    // `f32 as u32` is a saturating cast and is the only float-to-integer
    // conversion usable in a `const fn`; `u32::try_from` is not implemented for
    // `f32`, so the lints' suggested fix does not compile.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub const fn from_f32_truncating(ticks: f32) -> Self {
        Self(ticks.max(0.0) as u32)
    }
}

impl From<u32> for BackTicks {
    #[inline]
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

/// Two-byte fixed-tick input record.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputSample {
    state_ticks: u8,
    active: u8,
}

impl InputSample {
    pub const MAX_STATE_TICKS: u8 = 0xfe;

    #[inline]
    #[must_use]
    pub const fn state_ticks(self) -> u8 {
        self.state_ticks
    }

    #[inline]
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.active != 0
    }

    #[inline]
    #[must_use]
    pub const fn became_active(self) -> bool {
        self.state_ticks == 1 && self.active != 0
    }

    #[inline]
    #[must_use]
    pub const fn became_inactive(self) -> bool {
        self.state_ticks == 1 && self.active == 0
    }

    #[inline]
    fn update(&mut self, active: bool) {
        if self.is_active() != active {
            self.state_ticks = 0;
        }
        self.active = u8::from(active);
        self.state_ticks = self
            .state_ticks
            .saturating_add(1)
            .min(Self::MAX_STATE_TICKS);
    }
}

/// Allocation-free fixed-tick history for one ECS entity.
#[derive(Component, Debug)]
pub struct HistoricalInputBuffer {
    layout: Arc<InputMapLayout>,
    capacity: HistoryCapacity,
    cursor: usize,
    current: Box<[InputSample]>,
    history: Box<[InputSample]>,
    disabled: Box<[bool]>,
}

impl HistoricalInputBuffer {
    #[must_use]
    pub fn builder(layout: Arc<InputMapLayout>) -> HistoricalInputBufferBuilder {
        HistoricalInputBufferBuilder {
            layout,
            capacity: HistoryCapacity::default(),
        }
    }

    #[inline]
    #[must_use]
    pub const fn layout(&self) -> &Arc<InputMapLayout> {
        &self.layout
    }

    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> HistoryCapacity {
        self.capacity
    }

    #[inline]
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Ensure the ring can evaluate an authored native `BackTicks` window.
    ///
    /// Historical Input reserves one ring slot as the scan sentinel, so a
    /// non-zero `BackTicks` value requires the current sample, every requested
    /// prior sample, and the sentinel. Existing samples retain their
    /// ticks-back positions when the ring grows.
    ///
    /// # Panics
    ///
    /// Panics if `back_ticks` does not fit in a `usize`, which cannot happen on
    /// any target this gem builds for (`usize` is at least 32 bits wide).
    pub fn ensure_back_ticks(&mut self, back_ticks: BackTicks) {
        let requested = back_ticks.get();
        if requested == 0 {
            return;
        }
        let required = usize::try_from(requested)
            .expect("u32 back-tick count must fit usize")
            .saturating_add(2);
        let required = HistoryCapacity::new(
            NonZeroUsize::new(required).expect("back-tick history capacity is non-zero"),
        );
        self.ensure_history_capacity(required);
    }

    /// Grow the history ring without changing the meaning of any retained
    /// ticks-back sample. Shrink requests are ignored.
    ///
    /// # Panics
    ///
    /// Panics if `input_count * capacity` overflows `usize`, i.e. the requested
    /// ring is larger than the address space.
    pub fn ensure_history_capacity(&mut self, capacity: HistoryCapacity) {
        let old_capacity = self.capacity.get();
        let new_capacity = capacity.get();
        if new_capacity <= old_capacity {
            return;
        }

        let input_count = self.current.len();
        let history_len = input_count
            .checked_mul(new_capacity)
            .expect("historical input allocation size overflow");
        let mut history = vec![InputSample::default(); history_len];
        let retained_ticks = if old_capacity == 1 {
            1
        } else {
            old_capacity - 1
        };
        for input in 0..input_count {
            for ticks_back in 0..retained_ticks {
                let old_tick = (self.cursor + old_capacity - ticks_back) % old_capacity;
                let new_tick = (new_capacity - ticks_back) % new_capacity;
                history[input * new_capacity + new_tick] =
                    self.history[input * old_capacity + old_tick];
            }
        }
        self.capacity = capacity;
        self.cursor = 0;
        self.history = history.into_boxed_slice();
    }

    #[inline]
    #[must_use]
    pub fn resolve(&self, input: impl AsRef<str>) -> Option<InputIndex> {
        self.layout.resolve(input)
    }

    #[inline]
    #[must_use]
    pub fn resolve_crc(&self, input: Crc32) -> Option<InputIndex> {
        self.layout.resolve_crc(input)
    }

    #[must_use]
    pub fn sample(&self, input: InputIndex, ticks_back: u32) -> Option<InputSample> {
        self.validate_index(input)?;
        let tick = self.ring_index(ticks_back);
        self.history.get(self.history_offset(input, tick)).copied()
    }

    #[inline]
    #[must_use]
    pub fn current_sample(&self, input: InputIndex) -> Option<InputSample> {
        self.sample(input, 0)
    }

    #[inline]
    #[must_use]
    pub fn is_disabled(&self, input: InputIndex) -> bool {
        self.disabled.get(input.index()).copied().unwrap_or(true)
    }

    pub fn set_disabled(&mut self, input: InputIndex, disabled: bool) -> bool {
        let Some(slot) = self.disabled.get_mut(input.index()) else {
            return false;
        };
        *slot = disabled;
        true
    }

    /// Record an immediate state update in the current ring slot.
    pub fn set_active(&mut self, input: InputIndex, active: bool) -> bool {
        let Some(sample) = self.current.get_mut(input.index()) else {
            return false;
        };
        sample.update(active);
        let sample = *sample;
        let offset = self.history_offset(input, self.cursor);
        self.history[offset] = sample;
        true
    }

    /// Advance the ring and record a dense frame of input states.
    ///
    /// `active_inputs[index]` is the state for that dense input index. Missing
    /// trailing values are inactive.
    pub fn record_frame(&mut self, active_inputs: &[bool]) {
        self.advance_cursor();
        let input_count = self.current.len();
        for index in 0..input_count {
            self.current[index].update(active_inputs.get(index).copied().unwrap_or(false));
            self.history[index * self.capacity.get() + self.cursor] = self.current[index];
        }
    }

    /// Advance the ring from a packed little-endian bit set.
    pub fn record_frame_bits(&mut self, active_words: &[u32]) {
        self.advance_cursor();
        let capacity = self.capacity.get();
        for (index, sample) in self.current.iter_mut().enumerate() {
            let active = active_words
                .get(index / u32::BITS as usize)
                .is_some_and(|word| word & (1 << (index % u32::BITS as usize)) != 0);
            sample.update(active);
            self.history[index * capacity + self.cursor] = *sample;
        }
    }

    pub fn clear_history(&mut self, input: InputIndex) -> bool {
        if self.validate_index(input).is_none() {
            return false;
        }
        let start = input.index() * self.capacity.get();
        self.history[start..start + self.capacity.get()].fill(InputSample::default());
        true
    }

    /// Reset the live state for one input without rewriting prior ring slots.
    pub fn clear_current(&mut self, input: InputIndex) -> bool {
        let Some(sample) = self.current.get_mut(input.index()) else {
            return false;
        };
        *sample = InputSample::default();
        true
    }

    pub fn clear_all_history(&mut self) {
        self.history.fill(InputSample::default());
    }

    #[must_use]
    pub fn became_active(&self, input: InputIndex, back_ticks: BackTicks) -> bool {
        self.any_in_native_window(input, back_ticks, InputSample::became_active)
    }

    #[must_use]
    pub fn became_inactive(&self, input: InputIndex, back_ticks: BackTicks) -> bool {
        self.any_in_native_window(input, back_ticks, InputSample::became_inactive)
    }

    #[must_use]
    pub fn was_active(&self, input: InputIndex, back_ticks: BackTicks) -> bool {
        self.any_in_native_window(input, back_ticks, InputSample::is_active)
    }

    #[must_use]
    pub fn was_inactive(&self, input: InputIndex, back_ticks: BackTicks) -> bool {
        self.any_in_native_window(input, back_ticks, |sample| !sample.is_active())
    }

    #[must_use]
    pub fn active_state_ticks(&self, input: InputIndex) -> u8 {
        self.current_sample(input)
            .filter(|sample| sample.is_active())
            .map_or(0, InputSample::state_ticks)
    }

    #[must_use]
    pub fn is_disabled_and_active(&self, input: InputIndex) -> bool {
        self.is_disabled(input)
            && self
                .current_sample(input)
                .is_some_and(InputSample::is_active)
    }

    #[must_use]
    pub fn ticks_since_became_active(&self, input: InputIndex) -> Option<BackTicks> {
        if self.is_disabled(input) {
            return None;
        }
        if self.capacity.get() == 1
            || self
                .current_sample(input)
                .is_some_and(InputSample::became_active)
        {
            return Some(BackTicks::default());
        }
        let exclusive_sentinel =
            u32::try_from(self.capacity.get().saturating_sub(1)).unwrap_or(u32::MAX);
        (1..exclusive_sentinel).find_map(|ticks_back| {
            self.sample(input, ticks_back)
                .is_some_and(InputSample::became_active)
                .then_some(BackTicks::new(ticks_back))
        })
    }

    #[inline]
    fn validate_index(&self, input: InputIndex) -> Option<()> {
        (input.index() < self.current.len()).then_some(())
    }

    #[inline]
    const fn advance_cursor(&mut self) {
        self.cursor += 1;
        if self.cursor == self.capacity.get() {
            self.cursor = 0;
        }
    }

    #[inline]
    fn ring_index(&self, ticks_back: u32) -> usize {
        let maximum_offset = self.capacity.get() - 1;
        let offset = usize::try_from(ticks_back)
            .unwrap_or(usize::MAX)
            .min(maximum_offset);
        (self.cursor + self.capacity.get() - offset) % self.capacity.get()
    }

    #[inline]
    const fn history_offset(&self, input: InputIndex, tick: usize) -> usize {
        input.index() * self.capacity.get() + tick
    }

    fn any_in_native_window(
        &self,
        input: InputIndex,
        back_ticks: BackTicks,
        predicate: impl Fn(InputSample) -> bool,
    ) -> bool {
        if self.is_disabled(input) {
            return false;
        }
        if self.capacity.get() == 1 || back_ticks.get() == 0 {
            return self.current_sample(input).is_some_and(predicate);
        }

        let sentinel = u32::try_from(self.capacity.get() - 1).unwrap_or(u32::MAX);
        let exclusive_stop = back_ticks.get().saturating_add(1).min(sentinel);
        (0..exclusive_stop).any(|ticks_back| self.sample(input, ticks_back).is_some_and(&predicate))
    }
}

#[derive(Debug)]
pub struct HistoricalInputBufferBuilder {
    layout: Arc<InputMapLayout>,
    capacity: HistoryCapacity,
}

#[derive(Component, Debug, Clone)]
struct PendingHistoricalInputContext {
    input_map: Handle<InputMapAsset>,
    capacity: HistoryCapacity,
}

/// Query filter matching contexts that were just added or just edited.
#[cfg(feature = "runtime")]
type HistoricalInputContextTouched = Or<(
    Added<HistoricalInputContext>,
    Changed<HistoricalInputContext>,
)>;

#[cfg(feature = "runtime")]
fn request_historical_input_contexts(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    contexts: Query<(Entity, &HistoricalInputContext), HistoricalInputContextTouched>,
) {
    let (Some(catalog), Some(asset_server)) = (catalog, asset_server) else {
        return;
    };
    for (entity, context) in &contexts {
        match catalog.load_asset_ref(&context.input_map, &asset_server) {
            Ok(input_map) => {
                commands
                    .entity(entity)
                    .remove::<HistoricalInputBuffer>()
                    .remove::<HistoricalInputContextFailed>()
                    .insert(PendingHistoricalInputContext {
                        input_map,
                        capacity: context.normalized_capacity(),
                    });
            }
            Err(error) => {
                let message = error.to_string();
                commands
                    .entity(entity)
                    .remove::<HistoricalInputBuffer>()
                    .remove::<PendingHistoricalInputContext>()
                    .insert(HistoricalInputContextFailed {
                        message: message.clone(),
                    });
                tracing::error!(?entity, %error, "failed to resolve historical input map");
            }
        }
    }
}

#[cfg(feature = "runtime")]
fn fail_historical_input_asset_loads(
    mut commands: Commands,
    mut failures: MessageReader<AssetLoadFailedEvent<InputMapAsset>>,
    pending: Query<(Entity, &PendingHistoricalInputContext)>,
) {
    for failure in failures.read() {
        for (entity, pending) in &pending {
            if pending.input_map.id() != failure.id {
                continue;
            }
            let message = format!(
                "failed to load historical input map {}: {}",
                failure.path, failure.error
            );
            commands
                .entity(entity)
                .remove::<HistoricalInputBuffer>()
                .remove::<PendingHistoricalInputContext>()
                .insert(HistoricalInputContextFailed {
                    message: message.clone(),
                });
            tracing::error!(?entity, error = %message, "historical input asset load failed");
        }
    }
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing it here would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::needless_pass_by_value)]
#[cfg(feature = "runtime")]
fn build_historical_input_contexts(
    mut commands: Commands,
    assets: Res<Assets<InputMapAsset>>,
    pending: Query<(Entity, &PendingHistoricalInputContext)>,
) {
    for (entity, pending) in &pending {
        let Some(asset) = assets.get(&pending.input_map) else {
            continue;
        };
        match InputMapLayout::try_from(asset) {
            Ok(layout) => {
                let history = HistoricalInputBuffer::builder(Arc::new(layout))
                    .history_capacity(pending.capacity)
                    .build();
                commands
                    .entity(entity)
                    .insert(history)
                    .remove::<PendingHistoricalInputContext>()
                    .remove::<HistoricalInputContextFailed>();
            }
            Err(error) => {
                let message = error.to_string();
                commands
                    .entity(entity)
                    .remove::<PendingHistoricalInputContext>()
                    .insert(HistoricalInputContextFailed {
                        message: message.clone(),
                    });
                tracing::error!(?entity, %error, "invalid historical input map");
            }
        }
    }
}

impl HistoricalInputBufferBuilder {
    #[inline]
    #[must_use]
    pub fn history_capacity(mut self, capacity: impl Into<HistoryCapacity>) -> Self {
        self.capacity = capacity.into();
        self
    }

    /// Allocate the buffer described by this builder.
    ///
    /// # Panics
    ///
    /// Panics if `input_count * capacity` overflows `usize`, i.e. the requested
    /// ring is larger than the address space.
    #[must_use]
    pub fn build(self) -> HistoricalInputBuffer {
        let input_count = self.layout.len();
        let history_len = input_count
            .checked_mul(self.capacity.get())
            .expect("historical input allocation size overflow");
        HistoricalInputBuffer {
            layout: self.layout,
            capacity: self.capacity,
            cursor: 0,
            current: vec![InputSample::default(); input_count].into_boxed_slice(),
            history: vec![InputSample::default(); history_len].into_boxed_slice(),
            disabled: vec![false; input_count].into_boxed_slice(),
        }
    }
}

/// Register Historical Input reflected types and runtime assets.
#[cfg(feature = "runtime")]
pub struct HistoricalInputPlugin;

#[cfg(feature = "runtime")]
impl Plugin for HistoricalInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<InputMapAsset>()
            .init_asset_loader::<InputMapAssetLoader>()
            .register_type::<AzAssetData>()
            .register_type_data::<AzAssetData, ReflectAzTypeInfo>()
            .register_type_data::<AzAssetData, ReflectAzRtti>()
            .register_type::<InputMapAsset>()
            .register_type_data::<InputMapAsset, ReflectAzTypeInfo>()
            .register_type_data::<InputMapAsset, ReflectAzRtti>()
            .register_type::<HistoricalInputContext>()
            .register_type::<InputIndex>()
            .register_type::<HistoricalInputSystemComponent>()
            .register_type_data::<HistoricalInputSystemComponent, ReflectAzTypeInfo>()
            .register_type_data::<HistoricalInputSystemComponent, ReflectAzRtti>()
            .add_systems(
                Update,
                (
                    request_historical_input_contexts,
                    fail_historical_input_asset_loads,
                    build_historical_input_contexts,
                )
                    .chain(),
            );
    }
}

/// The asset types this gem owns.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [AssetTypeRegistration::for_asset::<InputMapAsset>().with_owner("az-gem-historical-input")]
}

/// The AZ types this gem owns.
///
/// Each named const is the item the derive emitted. Dropping one from this list
/// does not compile.
#[must_use]
pub const fn types() -> [AzTypeRegistration; 1] {
    [InputMapAsset::REGISTRATION]
}

/// The component lowerings this gem owns.
#[must_use]
pub const fn lowerings() -> [ComponentLoweringRegistration; 1] {
    [HistoricalInputSystemComponent::REGISTRATION]
}

/// The gem's `package` contribution.
///
/// Sealing is privacy: the generated `package_contribution` is the only way in.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<AssetTypeRegistration>()
            .register_many(asset_types());
        ctx.registrar::<AzTypeRegistration>().register_many(types());
        ctx.registrar::<ComponentLoweringRegistration>()
            .register_many(lowerings());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (InputIndex, HistoricalInputBuffer) {
        let asset = InputMapAsset {
            name_map: vec!["attack_primary".to_owned()],
            input_map: HashMap::from([(Crc32::from_str_lower("attack_primary").value(), 0)]),
        };
        let layout = Arc::new(InputMapLayout::try_from(&asset).unwrap());
        let input = layout.resolve("attack_primary").unwrap();
        let history = HistoricalInputBuffer::builder(layout)
            .history_capacity(NonZeroUsize::new(5).unwrap())
            .build();
        (input, history)
    }

    #[test]
    fn state_ticks_reset_on_edges_and_saturate() {
        let (input, mut history) = fixture();
        history.record_frame(&[false]);
        assert_eq!(history.current_sample(input).unwrap().state_ticks(), 1);
        history.record_frame(&[true]);
        assert!(history.current_sample(input).unwrap().became_active());
        for _ in 0..300 {
            history.record_frame(&[true]);
        }
        assert_eq!(
            history.current_sample(input).unwrap().state_ticks(),
            InputSample::MAX_STATE_TICKS
        );
        history.record_frame(&[false]);
        assert!(history.current_sample(input).unwrap().became_inactive());
    }

    #[test]
    fn edge_query_matches_exclusive_sentinel_ring_window() {
        let (input, mut history) = fixture();
        history.record_frame(&[true]);
        history.record_frame(&[true]);
        history.record_frame(&[true]);

        assert!(!history.became_active(input, BackTicks::new(1)));
        assert!(history.became_active(input, BackTicks::new(2)));
    }

    #[test]
    fn authored_back_ticks_grow_the_ring_and_preserve_samples() {
        let (input, mut history) = fixture();
        history.record_frame(&[true]);
        history.record_frame(&[false]);
        let current = history.sample(input, 0).unwrap();
        let previous = history.sample(input, 1).unwrap();

        history.ensure_back_ticks(BackTicks::new(8));

        assert_eq!(history.capacity().get(), 10);
        assert_eq!(history.sample(input, 0), Some(current));
        assert_eq!(history.sample(input, 1), Some(previous));
    }

    #[test]
    fn ticks_since_became_active_excludes_the_ring_sentinel() {
        let (input, fixture) = fixture();
        let mut history = HistoricalInputBuffer::builder(fixture.layout().clone())
            .history_capacity(NonZeroUsize::new(3).unwrap())
            .build();
        history.record_frame(&[true]);
        history.record_frame(&[false]);
        history.record_frame(&[false]);

        assert!(history.sample(input, 2).unwrap().became_active());
        assert_eq!(history.ticks_since_became_active(input), None);
    }

    #[test]
    fn disabled_inputs_never_match_queries() {
        let (input, mut history) = fixture();
        history.record_frame(&[true]);
        assert!(history.became_active(input, BackTicks::default()));
        assert!(history.set_disabled(input, true));
        assert!(!history.became_active(input, BackTicks::new(4)));
    }

    #[test]
    fn invalid_loaded_map_marks_context_failed() {
        let mut app = App::new();
        app.init_resource::<Assets<InputMapAsset>>()
            .add_systems(Update, build_historical_input_contexts);
        let input_map =
            app.world_mut()
                .resource_mut::<Assets<InputMapAsset>>()
                .add(InputMapAsset {
                    name_map: Vec::new(),
                    input_map: HashMap::from([(0x1234_5678, 0)]),
                });
        let entity = app
            .world_mut()
            .spawn(PendingHistoricalInputContext {
                input_map,
                capacity: HistoryCapacity::CURRENT_ONLY,
            })
            .id();

        app.update();

        let failed = app
            .world()
            .get::<HistoricalInputContextFailed>(entity)
            .expect("invalid input map must leave a terminal failure component");
        assert!(failed.message().contains("name map contains 0 inputs"));
        assert!(
            app.world()
                .get::<PendingHistoricalInputContext>(entity)
                .is_none()
        );
        assert!(app.world().get::<HistoricalInputBuffer>(entity).is_none());
    }
}
