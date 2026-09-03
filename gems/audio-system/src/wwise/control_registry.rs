//! Wwise ATL control registry.
//!
//! O3DE reference: `Gems/AudioSystem/Code/Source/Engine/ATL.h:153`.

use std::collections::HashMap;

use bevy::asset::{AssetEvent, AssetId};
use bevy::prelude::*;

use crate::{AudioControlId, INVALID_AUDIO_CONTROL_ID};

use super::{
    WwiseAudioControlsAsset, WwiseEnvironmentControl, WwisePreloadControl, WwiseRtpcControl,
    WwiseSwitchControl, WwiseSwitchStateControl, WwiseTriggerControl,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControlLocation {
    asset_id: AssetId<WwiseAudioControlsAsset>,
    index: usize,
}

impl ControlLocation {
    const fn new(asset_id: AssetId<WwiseAudioControlsAsset>, index: usize) -> Self {
        Self { asset_id, index }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SwitchStateLocation {
    asset_id: AssetId<WwiseAudioControlsAsset>,
    switch_index: usize,
    state_index: usize,
}

impl SwitchStateLocation {
    const fn new(
        asset_id: AssetId<WwiseAudioControlsAsset>,
        switch_index: usize,
        state_index: usize,
    ) -> Self {
        Self {
            asset_id,
            switch_index,
            state_index,
        }
    }
}

/// Runtime index for loaded Wwise ATL control metadata.
///
/// O3DE reference: `Gems/AudioSystem/Code/Source/Engine/ATL.cpp:190`.
#[derive(Resource, Debug, Clone, Default)]
pub struct WwiseControlRegistry {
    triggers: HashMap<AudioControlId, ControlLocation>,
    preloads: HashMap<AudioControlId, ControlLocation>,
    rtpcs: HashMap<AudioControlId, ControlLocation>,
    switches: HashMap<AudioControlId, ControlLocation>,
    switch_states: HashMap<(AudioControlId, AudioControlId), SwitchStateLocation>,
    environments: HashMap<AudioControlId, ControlLocation>,
}

impl WwiseControlRegistry {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
            && self.preloads.is_empty()
            && self.rtpcs.is_empty()
            && self.switches.is_empty()
            && self.switch_states.is_empty()
            && self.environments.is_empty()
    }

    #[must_use]
    pub fn trigger_count(&self) -> usize {
        self.triggers.len()
    }

    #[must_use]
    pub fn preload_count(&self) -> usize {
        self.preloads.len()
    }

    #[must_use]
    pub fn rtpc_count(&self) -> usize {
        self.rtpcs.len()
    }

    #[must_use]
    pub fn switch_count(&self) -> usize {
        self.switches.len()
    }

    #[must_use]
    pub fn switch_state_count(&self) -> usize {
        self.switch_states.len()
    }

    #[must_use]
    pub fn environment_count(&self) -> usize {
        self.environments.len()
    }

    pub fn clear(&mut self) {
        self.triggers.clear();
        self.preloads.clear();
        self.rtpcs.clear();
        self.switches.clear();
        self.switch_states.clear();
        self.environments.clear();
    }

    pub fn rebuild_from_assets<'a>(
        &mut self,
        assets: impl IntoIterator<
            Item = (
                AssetId<WwiseAudioControlsAsset>,
                &'a WwiseAudioControlsAsset,
            ),
        >,
    ) {
        self.clear();
        for (asset_id, asset) in assets {
            self.index_asset(asset_id, asset);
        }
    }

    pub fn index_asset(
        &mut self,
        asset_id: AssetId<WwiseAudioControlsAsset>,
        asset: &WwiseAudioControlsAsset,
    ) {
        for (index, trigger) in asset.triggers.iter().enumerate() {
            insert_control(&mut self.triggers, trigger.id, asset_id, index);
        }

        for (index, preload) in asset.preloads.iter().enumerate() {
            insert_control(&mut self.preloads, preload.id, asset_id, index);
        }

        for (index, rtpc) in asset.rtpcs.iter().enumerate() {
            insert_control(&mut self.rtpcs, rtpc.id, asset_id, index);
        }

        for (switch_index, switch) in asset.switches.iter().enumerate() {
            insert_control(&mut self.switches, switch.id, asset_id, switch_index);
            if !switch.id.is_valid() {
                continue;
            }
            for (state_index, state) in switch.states.iter().enumerate() {
                if state.id.is_valid() {
                    self.switch_states
                        .entry((switch.id, state.id))
                        .or_insert_with(|| {
                            SwitchStateLocation::new(asset_id, switch_index, state_index)
                        });
                }
            }
        }

        for (index, environment) in asset.environments.iter().enumerate() {
            insert_control(&mut self.environments, environment.id, asset_id, index);
        }
    }

    #[must_use]
    pub fn trigger_id(&self, name: &str) -> AudioControlId {
        resolved_control_id(name, &self.triggers)
    }

    #[must_use]
    pub fn preload_id(&self, name: &str) -> AudioControlId {
        resolved_control_id(name, &self.preloads)
    }

    #[must_use]
    pub fn rtpc_id(&self, name: &str) -> AudioControlId {
        resolved_control_id(name, &self.rtpcs)
    }

    #[must_use]
    pub fn switch_id(&self, name: &str) -> AudioControlId {
        resolved_control_id(name, &self.switches)
    }

    #[must_use]
    pub fn switch_state_id(&self, switch_id: AudioControlId, name: &str) -> AudioControlId {
        let state_id = AudioControlId::from_name(name);
        if state_id.is_valid() && self.switch_states.contains_key(&(switch_id, state_id)) {
            state_id
        } else {
            INVALID_AUDIO_CONTROL_ID
        }
    }

    #[must_use]
    pub fn environment_id(&self, name: &str) -> AudioControlId {
        resolved_control_id(name, &self.environments)
    }

    #[must_use]
    pub fn trigger<'a>(
        &self,
        id: AudioControlId,
        assets: &'a Assets<WwiseAudioControlsAsset>,
    ) -> Option<&'a WwiseTriggerControl> {
        let location = self.triggers.get(&id)?;
        assets
            .get(location.asset_id)
            .and_then(|asset| asset.triggers.get(location.index))
    }

    #[must_use]
    pub fn preload<'a>(
        &self,
        id: AudioControlId,
        assets: &'a Assets<WwiseAudioControlsAsset>,
    ) -> Option<&'a WwisePreloadControl> {
        let location = self.preloads.get(&id)?;
        assets
            .get(location.asset_id)
            .and_then(|asset| asset.preloads.get(location.index))
    }

    #[must_use]
    pub fn rtpc<'a>(
        &self,
        id: AudioControlId,
        assets: &'a Assets<WwiseAudioControlsAsset>,
    ) -> Option<&'a WwiseRtpcControl> {
        let location = self.rtpcs.get(&id)?;
        assets
            .get(location.asset_id)
            .and_then(|asset| asset.rtpcs.get(location.index))
    }

    #[must_use]
    pub fn switch<'a>(
        &self,
        id: AudioControlId,
        assets: &'a Assets<WwiseAudioControlsAsset>,
    ) -> Option<&'a WwiseSwitchControl> {
        let location = self.switches.get(&id)?;
        assets
            .get(location.asset_id)
            .and_then(|asset| asset.switches.get(location.index))
    }

    #[must_use]
    pub fn switch_state<'a>(
        &self,
        switch_id: AudioControlId,
        state_id: AudioControlId,
        assets: &'a Assets<WwiseAudioControlsAsset>,
    ) -> Option<&'a WwiseSwitchStateControl> {
        let location = self.switch_states.get(&(switch_id, state_id))?;
        assets.get(location.asset_id).and_then(|asset| {
            asset
                .switches
                .get(location.switch_index)
                .and_then(|switch| switch.states.get(location.state_index))
        })
    }

    #[must_use]
    pub fn environment<'a>(
        &self,
        id: AudioControlId,
        assets: &'a Assets<WwiseAudioControlsAsset>,
    ) -> Option<&'a WwiseEnvironmentControl> {
        let location = self.environments.get(&id)?;
        assets
            .get(location.asset_id)
            .and_then(|asset| asset.environments.get(location.index))
    }
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing it here would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::needless_pass_by_value)]
pub fn rebuild_wwise_control_registry_on_asset_events(
    mut registry: ResMut<WwiseControlRegistry>,
    assets: Res<Assets<WwiseAudioControlsAsset>>,
    mut events: MessageReader<AssetEvent<WwiseAudioControlsAsset>>,
) {
    let should_rebuild = events.read().any(|event| {
        matches!(
            event,
            AssetEvent::Added { .. }
                | AssetEvent::Modified { .. }
                | AssetEvent::LoadedWithDependencies { .. }
                | AssetEvent::Removed { .. }
        )
    });

    if should_rebuild || (registry.is_empty() && assets.iter().next().is_some()) {
        registry.rebuild_from_assets(assets.iter());
    }
}

fn insert_control(
    controls: &mut HashMap<AudioControlId, ControlLocation>,
    id: AudioControlId,
    asset_id: AssetId<WwiseAudioControlsAsset>,
    index: usize,
) {
    if id.is_valid() {
        controls
            .entry(id)
            .or_insert_with(|| ControlLocation::new(asset_id, index));
    }
}

fn resolved_control_id(
    name: &str,
    controls: &HashMap<AudioControlId, ControlLocation>,
) -> AudioControlId {
    let id = AudioControlId::from_name(name);
    if id.is_valid() && controls.contains_key(&id) {
        id
    } else {
        INVALID_AUDIO_CONTROL_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AudioSystemAssetPlugin, WwiseBankReference, WwiseEnvironmentImplementation, WwiseNamedId,
        WwiseRtpcReference, WwiseSwitchStateImplementation,
    };

    #[test]
    fn registry_indexes_controls_without_cloning_payloads() {
        let mut assets = Assets::<WwiseAudioControlsAsset>::default();
        let asset = sample_controls_asset("play_foo");
        let handle = assets.add(asset);
        let mut registry = WwiseControlRegistry::default();

        registry.rebuild_from_assets(assets.iter());

        assert_eq!(registry.trigger_count(), 1);
        assert_eq!(registry.preload_count(), 1);
        assert_eq!(registry.rtpc_count(), 1);
        assert_eq!(registry.switch_count(), 1);
        assert_eq!(registry.switch_state_count(), 1);
        assert_eq!(registry.environment_count(), 1);
        assert_eq!(
            registry.trigger_id("play_foo"),
            AudioControlId::from_name("play_foo")
        );
        assert_eq!(registry.trigger_id("missing"), INVALID_AUDIO_CONTROL_ID);

        let trigger = registry
            .trigger(AudioControlId::from_name("play_foo"), &assets)
            .unwrap();
        assert_eq!(trigger.name, "play_foo");
        assert_eq!(trigger.events[0].name, "Play_Foo");

        let switch_id = registry.switch_id("surface");
        let state_id = registry.switch_state_id(switch_id, "snow");
        let state = registry.switch_state(switch_id, state_id, &assets).unwrap();
        assert_eq!(state.name, "snow");
        assert_eq!(
            registry.switch_state_id(switch_id, "missing"),
            INVALID_AUDIO_CONTROL_ID
        );

        assert!(
            registry
                .preload(AudioControlId::from_name("preload_foo"), &assets)
                .is_some()
        );
        assert!(
            registry
                .rtpc(AudioControlId::from_name("volume"), &assets)
                .is_some()
        );
        assert!(registry.switch(switch_id, &assets).is_some());
        assert!(
            registry
                .environment(AudioControlId::from_name("cave"), &assets)
                .is_some()
        );
        assert_eq!(
            assets.get(&handle).unwrap().triggers[0].events[0].name,
            "Play_Foo"
        );
    }

    #[test]
    fn registry_keeps_first_loaded_duplicate_control() {
        let mut assets = Assets::<WwiseAudioControlsAsset>::default();
        assets.add(sample_controls_asset("play_foo"));
        assets.add(sample_controls_asset("play_foo"));
        let mut registry = WwiseControlRegistry::default();

        registry.rebuild_from_assets(assets.iter());

        assert_eq!(registry.trigger_count(), 1);
        assert_eq!(
            registry.trigger_id("play_foo"),
            AudioControlId::from_name("play_foo")
        );
    }

    #[test]
    fn asset_plugin_rebuilds_registry_when_controls_load() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins(AudioSystemAssetPlugin);

        let _handle = app
            .world_mut()
            .resource_mut::<Assets<WwiseAudioControlsAsset>>()
            .add(sample_controls_asset("play_foo"));

        app.update();

        let registry = app.world().resource::<WwiseControlRegistry>();
        assert_eq!(
            registry.trigger_id("play_foo"),
            AudioControlId::from_name("play_foo")
        );
    }

    fn sample_controls_asset(trigger_name: &str) -> WwiseAudioControlsAsset {
        WwiseAudioControlsAsset {
            triggers: vec![WwiseTriggerControl {
                name: trigger_name.to_string(),
                id: AudioControlId::from_name(trigger_name),
                events: vec![WwiseNamedId::new("Play_Foo")],
            }],
            preloads: vec![WwisePreloadControl {
                name: "preload_foo".to_string(),
                id: AudioControlId::from_name("preload_foo"),
                auto_load: true,
                banks: vec![WwiseBankReference {
                    path: "audio/sounds/wwise/foo.bnk".to_string(),
                    localized: false,
                }],
            }],
            rtpcs: vec![WwiseRtpcControl {
                name: "volume".to_string(),
                id: AudioControlId::from_name("volume"),
                rtpcs: vec![WwiseRtpcReference {
                    name: "Volume".to_string(),
                    id: crate::WwiseNameId::from_name("Volume"),
                    multiplier: 1.0,
                    shift: 0.0,
                }],
            }],
            switches: vec![WwiseSwitchControl {
                name: "surface".to_string(),
                id: AudioControlId::from_name("surface"),
                states: vec![WwiseSwitchStateControl {
                    name: "snow".to_string(),
                    id: AudioControlId::from_name("snow"),
                    implementations: vec![WwiseSwitchStateImplementation::Switch {
                        group: WwiseNamedId::new("Surface"),
                        value: WwiseNamedId::new("Snow"),
                    }],
                }],
            }],
            environments: vec![WwiseEnvironmentControl {
                name: "cave".to_string(),
                id: AudioControlId::from_name("cave"),
                implementations: vec![WwiseEnvironmentImplementation::AuxBus(WwiseNamedId::new(
                    "CaveAux",
                ))],
            }],
        }
    }
}
