//! Wwise ATL metadata assets.

use std::io::{Cursor, Read, Write};
use std::string::FromUtf8Error;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::prelude::*;
use thiserror::Error;

use crate::AudioControlId;

use super::ids::WwiseNameId;

const CONTROLS_MAGIC: &[u8; 8] = b"AZWWCTL\0";
const VERSION: u32 = 1;

/// File extensions claimed by [`WwiseAudioControlsAssetLoader`].
///
/// Legacy ATL controls are commonly stored under `libs/gameaudio/wwise/` as
/// `atl_controls.xml`, `default_controls.xml`, and `preloaddata.xml`.
/// The on-disk product preserves the source extension; the
/// transformed metadata still uses our binary payload.
pub const WWISE_AUDIO_CONTROLS_ASSET_EXTENSIONS: &[&str] = &["xml"];

/// Audio Translation Layer controls backed by Wwise implementation data.
#[derive(Asset, TypePath, Debug, Clone, Default, PartialEq)]
pub struct WwiseAudioControlsAsset {
    pub triggers: Vec<WwiseTriggerControl>,
    pub preloads: Vec<WwisePreloadControl>,
    pub rtpcs: Vec<WwiseRtpcControl>,
    pub switches: Vec<WwiseSwitchControl>,
    pub environments: Vec<WwiseEnvironmentControl>,
}

/// Named Wwise identifier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WwiseNamedId {
    pub name: String,
    pub id: WwiseNameId,
}

impl WwiseNamedId {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: WwiseNameId::from_name(&name),
            name,
        }
    }
}

/// One ATL trigger and its Wwise events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WwiseTriggerControl {
    pub name: String,
    pub id: AudioControlId,
    pub events: Vec<WwiseNamedId>,
}

/// One ATL preload request and the Wwise banks it loads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WwisePreloadControl {
    pub name: String,
    pub id: AudioControlId,
    pub auto_load: bool,
    pub banks: Vec<WwiseBankReference>,
}

/// One Wwise bank referenced by a preload request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WwiseBankReference {
    pub path: String,
    pub localized: bool,
}

/// One ATL RTPC and its Wwise RTPC implementations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WwiseRtpcControl {
    pub name: String,
    pub id: AudioControlId,
    pub rtpcs: Vec<WwiseRtpcReference>,
}

/// One Wwise RTPC implementation.
#[derive(Debug, Clone, PartialEq)]
pub struct WwiseRtpcReference {
    pub name: String,
    pub id: WwiseNameId,
    pub multiplier: f32,
    pub shift: f32,
}

impl Default for WwiseRtpcReference {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: WwiseNameId::INVALID,
            multiplier: 1.0,
            shift: 0.0,
        }
    }
}

/// One ATL switch with state-specific Wwise implementations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WwiseSwitchControl {
    pub name: String,
    pub id: AudioControlId,
    pub states: Vec<WwiseSwitchStateControl>,
}

/// One ATL switch state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WwiseSwitchStateControl {
    pub name: String,
    pub id: AudioControlId,
    pub implementations: Vec<WwiseSwitchStateImplementation>,
}

/// Wwise implementation for an ATL switch state.
#[derive(Debug, Clone, PartialEq)]
pub enum WwiseSwitchStateImplementation {
    Switch {
        group: WwiseNamedId,
        value: WwiseNamedId,
    },
    State {
        group: WwiseNamedId,
        value: WwiseNamedId,
    },
    Rtpc {
        rtpc: WwiseNamedId,
        value: f32,
    },
}

/// One ATL environment and its Wwise implementations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WwiseEnvironmentControl {
    pub name: String,
    pub id: AudioControlId,
    pub implementations: Vec<WwiseEnvironmentImplementation>,
}

/// Wwise implementation for an ATL environment.
#[derive(Debug, Clone, PartialEq)]
pub enum WwiseEnvironmentImplementation {
    AuxBus(WwiseNamedId),
    Rtpc(WwiseRtpcReference),
}

/// Bevy loader for Wwise audio controls.
#[derive(Default, TypePath)]
pub struct WwiseAudioControlsAssetLoader;

impl AssetLoader for WwiseAudioControlsAssetLoader {
    type Asset = WwiseAudioControlsAsset;
    type Settings = ();
    type Error = WwiseMetadataFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        read_audio_controls_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        WWISE_AUDIO_CONTROLS_ASSET_EXTENSIONS
    }
}

/// Serialize an audio-controls asset in the `AZWWCTL` binary format.
///
/// # Errors
///
/// Returns [`WwiseMetadataFormatError::Io`] if `writer` rejects a write, or
/// [`WwiseMetadataFormatError::CountTooLarge`] if any control list is longer
/// than `u32::MAX` entries.
pub fn write_audio_controls_asset(
    asset: &WwiseAudioControlsAsset,
    mut writer: impl Write,
) -> Result<(), WwiseMetadataFormatError> {
    writer.write_all(CONTROLS_MAGIC)?;
    write_u32(&mut writer, VERSION)?;
    write_trigger_controls(&mut writer, &asset.triggers)?;
    write_preload_controls(&mut writer, &asset.preloads)?;
    write_rtpc_controls(&mut writer, &asset.rtpcs)?;
    write_switch_controls(&mut writer, &asset.switches)?;
    write_environment_controls(&mut writer, &asset.environments)?;
    Ok(())
}

/// Deserialize an audio-controls asset from an in-memory `AZWWCTL` buffer.
///
/// # Errors
///
/// Returns any error [`read_audio_controls_asset_from_reader`] returns.
pub fn read_audio_controls_asset(
    bytes: &[u8],
) -> Result<WwiseAudioControlsAsset, WwiseMetadataFormatError> {
    read_audio_controls_asset_from_reader(Cursor::new(bytes))
}

/// Deserialize an audio-controls asset from any [`Read`] source.
///
/// # Errors
///
/// Returns [`WwiseMetadataFormatError::BadMagic`] if the stream does not start
/// with `AZWWCTL\0`, [`WwiseMetadataFormatError::UnsupportedVersion`] if the
/// version word is not the one this build writes,
/// [`WwiseMetadataFormatError::Io`] if the stream ends mid-record,
/// [`WwiseMetadataFormatError::InvalidUtf8`] if a control name is not UTF-8,
/// and [`WwiseMetadataFormatError::InvalidData`] if an enum discriminant is out
/// of range.
pub fn read_audio_controls_asset_from_reader(
    mut reader: impl Read,
) -> Result<WwiseAudioControlsAsset, WwiseMetadataFormatError> {
    read_magic(&mut reader, *CONTROLS_MAGIC)?;
    read_version(&mut reader)?;
    Ok(WwiseAudioControlsAsset {
        triggers: read_trigger_controls(&mut reader)?,
        preloads: read_preload_controls(&mut reader)?,
        rtpcs: read_rtpc_controls(&mut reader)?,
        switches: read_switch_controls(&mut reader)?,
        environments: read_environment_controls(&mut reader)?,
    })
}

async fn read_audio_controls_asset_from_bevy_reader(
    reader: &mut dyn Reader,
) -> Result<WwiseAudioControlsAsset, WwiseMetadataFormatError> {
    read_bevy_magic(reader, CONTROLS_MAGIC).await?;
    read_bevy_version(reader).await?;
    Ok(WwiseAudioControlsAsset {
        triggers: read_bevy_trigger_controls(reader).await?,
        preloads: read_bevy_preload_controls(reader).await?,
        rtpcs: read_bevy_rtpc_controls(reader).await?,
        switches: read_bevy_switch_controls(reader).await?,
        environments: read_bevy_environment_controls(reader).await?,
    })
}

fn write_trigger_controls(
    writer: &mut impl Write,
    values: &[WwiseTriggerControl],
) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, values.len())?;
    for value in values {
        write_string(writer, &value.name)?;
        write_u64(writer, value.id.0)?;
        write_named_ids(writer, &value.events)?;
    }
    Ok(())
}

fn read_trigger_controls(
    reader: &mut impl Read,
) -> Result<Vec<WwiseTriggerControl>, WwiseMetadataFormatError> {
    let count = read_count(reader)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(WwiseTriggerControl {
            name: read_string(reader)?,
            id: AudioControlId(read_u64(reader)?),
            events: read_named_ids(reader)?,
        });
    }
    Ok(values)
}

async fn read_bevy_trigger_controls(
    reader: &mut dyn Reader,
) -> Result<Vec<WwiseTriggerControl>, WwiseMetadataFormatError> {
    let count = read_bevy_count(reader).await?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(WwiseTriggerControl {
            name: read_bevy_string(reader).await?,
            id: AudioControlId(read_bevy_u64(reader).await?),
            events: read_bevy_named_ids(reader).await?,
        });
    }
    Ok(values)
}

fn write_preload_controls(
    writer: &mut impl Write,
    values: &[WwisePreloadControl],
) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, values.len())?;
    for value in values {
        write_string(writer, &value.name)?;
        write_u64(writer, value.id.0)?;
        write_bool(writer, value.auto_load)?;
        write_len(writer, value.banks.len())?;
        for bank in &value.banks {
            write_string(writer, &bank.path)?;
            write_bool(writer, bank.localized)?;
        }
    }
    Ok(())
}

fn read_preload_controls(
    reader: &mut impl Read,
) -> Result<Vec<WwisePreloadControl>, WwiseMetadataFormatError> {
    let count = read_count(reader)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_string(reader)?;
        let id = AudioControlId(read_u64(reader)?);
        let auto_load = read_bool(reader)?;
        let bank_count = read_count(reader)?;
        let mut banks = Vec::with_capacity(bank_count);
        for _ in 0..bank_count {
            banks.push(WwiseBankReference {
                path: read_string(reader)?,
                localized: read_bool(reader)?,
            });
        }
        values.push(WwisePreloadControl {
            name,
            id,
            auto_load,
            banks,
        });
    }
    Ok(values)
}

async fn read_bevy_preload_controls(
    reader: &mut dyn Reader,
) -> Result<Vec<WwisePreloadControl>, WwiseMetadataFormatError> {
    let count = read_bevy_count(reader).await?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_bevy_string(reader).await?;
        let id = AudioControlId(read_bevy_u64(reader).await?);
        let auto_load = read_bevy_bool(reader).await?;
        let bank_count = read_bevy_count(reader).await?;
        let mut banks = Vec::with_capacity(bank_count);
        for _ in 0..bank_count {
            banks.push(WwiseBankReference {
                path: read_bevy_string(reader).await?,
                localized: read_bevy_bool(reader).await?,
            });
        }
        values.push(WwisePreloadControl {
            name,
            id,
            auto_load,
            banks,
        });
    }
    Ok(values)
}

fn write_rtpc_controls(
    writer: &mut impl Write,
    values: &[WwiseRtpcControl],
) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, values.len())?;
    for value in values {
        write_string(writer, &value.name)?;
        write_u64(writer, value.id.0)?;
        write_len(writer, value.rtpcs.len())?;
        for rtpc in &value.rtpcs {
            write_rtpc_reference(writer, rtpc)?;
        }
    }
    Ok(())
}

fn read_rtpc_controls(
    reader: &mut impl Read,
) -> Result<Vec<WwiseRtpcControl>, WwiseMetadataFormatError> {
    let count = read_count(reader)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_string(reader)?;
        let id = AudioControlId(read_u64(reader)?);
        let rtpc_count = read_count(reader)?;
        let mut rtpcs = Vec::with_capacity(rtpc_count);
        for _ in 0..rtpc_count {
            rtpcs.push(read_rtpc_reference(reader)?);
        }
        values.push(WwiseRtpcControl { name, id, rtpcs });
    }
    Ok(values)
}

async fn read_bevy_rtpc_controls(
    reader: &mut dyn Reader,
) -> Result<Vec<WwiseRtpcControl>, WwiseMetadataFormatError> {
    let count = read_bevy_count(reader).await?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_bevy_string(reader).await?;
        let id = AudioControlId(read_bevy_u64(reader).await?);
        let rtpc_count = read_bevy_count(reader).await?;
        let mut rtpcs = Vec::with_capacity(rtpc_count);
        for _ in 0..rtpc_count {
            rtpcs.push(read_bevy_rtpc_reference(reader).await?);
        }
        values.push(WwiseRtpcControl { name, id, rtpcs });
    }
    Ok(values)
}

fn write_switch_controls(
    writer: &mut impl Write,
    values: &[WwiseSwitchControl],
) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, values.len())?;
    for value in values {
        write_string(writer, &value.name)?;
        write_u64(writer, value.id.0)?;
        write_len(writer, value.states.len())?;
        for state in &value.states {
            write_string(writer, &state.name)?;
            write_u64(writer, state.id.0)?;
            write_len(writer, state.implementations.len())?;
            for implementation in &state.implementations {
                write_switch_implementation(writer, implementation)?;
            }
        }
    }
    Ok(())
}

fn read_switch_controls(
    reader: &mut impl Read,
) -> Result<Vec<WwiseSwitchControl>, WwiseMetadataFormatError> {
    let count = read_count(reader)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_string(reader)?;
        let id = AudioControlId(read_u64(reader)?);
        let state_count = read_count(reader)?;
        let mut states = Vec::with_capacity(state_count);
        for _ in 0..state_count {
            let state_name = read_string(reader)?;
            let state_id = AudioControlId(read_u64(reader)?);
            let implementation_count = read_count(reader)?;
            let mut implementations = Vec::with_capacity(implementation_count);
            for _ in 0..implementation_count {
                implementations.push(read_switch_implementation(reader)?);
            }
            states.push(WwiseSwitchStateControl {
                name: state_name,
                id: state_id,
                implementations,
            });
        }
        values.push(WwiseSwitchControl { name, id, states });
    }
    Ok(values)
}

async fn read_bevy_switch_controls(
    reader: &mut dyn Reader,
) -> Result<Vec<WwiseSwitchControl>, WwiseMetadataFormatError> {
    let count = read_bevy_count(reader).await?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_bevy_string(reader).await?;
        let id = AudioControlId(read_bevy_u64(reader).await?);
        let state_count = read_bevy_count(reader).await?;
        let mut states = Vec::with_capacity(state_count);
        for _ in 0..state_count {
            let state_name = read_bevy_string(reader).await?;
            let state_id = AudioControlId(read_bevy_u64(reader).await?);
            let implementation_count = read_bevy_count(reader).await?;
            let mut implementations = Vec::with_capacity(implementation_count);
            for _ in 0..implementation_count {
                implementations.push(read_bevy_switch_implementation(reader).await?);
            }
            states.push(WwiseSwitchStateControl {
                name: state_name,
                id: state_id,
                implementations,
            });
        }
        values.push(WwiseSwitchControl { name, id, states });
    }
    Ok(values)
}

fn write_environment_controls(
    writer: &mut impl Write,
    values: &[WwiseEnvironmentControl],
) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, values.len())?;
    for value in values {
        write_string(writer, &value.name)?;
        write_u64(writer, value.id.0)?;
        write_len(writer, value.implementations.len())?;
        for implementation in &value.implementations {
            write_environment_implementation(writer, implementation)?;
        }
    }
    Ok(())
}

fn read_environment_controls(
    reader: &mut impl Read,
) -> Result<Vec<WwiseEnvironmentControl>, WwiseMetadataFormatError> {
    let count = read_count(reader)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_string(reader)?;
        let id = AudioControlId(read_u64(reader)?);
        let implementation_count = read_count(reader)?;
        let mut implementations = Vec::with_capacity(implementation_count);
        for _ in 0..implementation_count {
            implementations.push(read_environment_implementation(reader)?);
        }
        values.push(WwiseEnvironmentControl {
            name,
            id,
            implementations,
        });
    }
    Ok(values)
}

async fn read_bevy_environment_controls(
    reader: &mut dyn Reader,
) -> Result<Vec<WwiseEnvironmentControl>, WwiseMetadataFormatError> {
    let count = read_bevy_count(reader).await?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_bevy_string(reader).await?;
        let id = AudioControlId(read_bevy_u64(reader).await?);
        let implementation_count = read_bevy_count(reader).await?;
        let mut implementations = Vec::with_capacity(implementation_count);
        for _ in 0..implementation_count {
            implementations.push(read_bevy_environment_implementation(reader).await?);
        }
        values.push(WwiseEnvironmentControl {
            name,
            id,
            implementations,
        });
    }
    Ok(values)
}

fn write_switch_implementation(
    writer: &mut impl Write,
    value: &WwiseSwitchStateImplementation,
) -> Result<(), WwiseMetadataFormatError> {
    match value {
        WwiseSwitchStateImplementation::Switch { group, value } => {
            write_u8(writer, 0)?;
            write_named_id(writer, group)?;
            write_named_id(writer, value)?;
        }
        WwiseSwitchStateImplementation::State { group, value } => {
            write_u8(writer, 1)?;
            write_named_id(writer, group)?;
            write_named_id(writer, value)?;
        }
        WwiseSwitchStateImplementation::Rtpc { rtpc, value } => {
            write_u8(writer, 2)?;
            write_named_id(writer, rtpc)?;
            write_f32(writer, *value)?;
        }
    }
    Ok(())
}

fn read_switch_implementation(
    reader: &mut impl Read,
) -> Result<WwiseSwitchStateImplementation, WwiseMetadataFormatError> {
    match read_u8(reader)? {
        0 => Ok(WwiseSwitchStateImplementation::Switch {
            group: read_named_id(reader)?,
            value: read_named_id(reader)?,
        }),
        1 => Ok(WwiseSwitchStateImplementation::State {
            group: read_named_id(reader)?,
            value: read_named_id(reader)?,
        }),
        2 => Ok(WwiseSwitchStateImplementation::Rtpc {
            rtpc: read_named_id(reader)?,
            value: read_f32(reader)?,
        }),
        tag => Err(WwiseMetadataFormatError::InvalidData {
            what: "switch implementation tag",
            value: tag,
        }),
    }
}

async fn read_bevy_switch_implementation(
    reader: &mut dyn Reader,
) -> Result<WwiseSwitchStateImplementation, WwiseMetadataFormatError> {
    match read_bevy_u8(reader).await? {
        0 => Ok(WwiseSwitchStateImplementation::Switch {
            group: read_bevy_named_id(reader).await?,
            value: read_bevy_named_id(reader).await?,
        }),
        1 => Ok(WwiseSwitchStateImplementation::State {
            group: read_bevy_named_id(reader).await?,
            value: read_bevy_named_id(reader).await?,
        }),
        2 => Ok(WwiseSwitchStateImplementation::Rtpc {
            rtpc: read_bevy_named_id(reader).await?,
            value: read_bevy_f32(reader).await?,
        }),
        tag => Err(WwiseMetadataFormatError::InvalidData {
            what: "switch implementation tag",
            value: tag,
        }),
    }
}

fn write_environment_implementation(
    writer: &mut impl Write,
    value: &WwiseEnvironmentImplementation,
) -> Result<(), WwiseMetadataFormatError> {
    match value {
        WwiseEnvironmentImplementation::AuxBus(aux_bus) => {
            write_u8(writer, 0)?;
            write_named_id(writer, aux_bus)?;
        }
        WwiseEnvironmentImplementation::Rtpc(rtpc) => {
            write_u8(writer, 1)?;
            write_rtpc_reference(writer, rtpc)?;
        }
    }
    Ok(())
}

fn read_environment_implementation(
    reader: &mut impl Read,
) -> Result<WwiseEnvironmentImplementation, WwiseMetadataFormatError> {
    match read_u8(reader)? {
        0 => Ok(WwiseEnvironmentImplementation::AuxBus(read_named_id(
            reader,
        )?)),
        1 => Ok(WwiseEnvironmentImplementation::Rtpc(read_rtpc_reference(
            reader,
        )?)),
        tag => Err(WwiseMetadataFormatError::InvalidData {
            what: "environment implementation tag",
            value: tag,
        }),
    }
}

async fn read_bevy_environment_implementation(
    reader: &mut dyn Reader,
) -> Result<WwiseEnvironmentImplementation, WwiseMetadataFormatError> {
    match read_bevy_u8(reader).await? {
        0 => Ok(WwiseEnvironmentImplementation::AuxBus(
            read_bevy_named_id(reader).await?,
        )),
        1 => Ok(WwiseEnvironmentImplementation::Rtpc(
            read_bevy_rtpc_reference(reader).await?,
        )),
        tag => Err(WwiseMetadataFormatError::InvalidData {
            what: "environment implementation tag",
            value: tag,
        }),
    }
}

fn write_rtpc_reference(
    writer: &mut impl Write,
    value: &WwiseRtpcReference,
) -> Result<(), WwiseMetadataFormatError> {
    write_string(writer, &value.name)?;
    write_u32(writer, value.id.0)?;
    write_f32(writer, value.multiplier)?;
    write_f32(writer, value.shift)
}

fn read_rtpc_reference(
    reader: &mut impl Read,
) -> Result<WwiseRtpcReference, WwiseMetadataFormatError> {
    Ok(WwiseRtpcReference {
        name: read_string(reader)?,
        id: WwiseNameId(read_u32(reader)?),
        multiplier: read_f32(reader)?,
        shift: read_f32(reader)?,
    })
}

async fn read_bevy_rtpc_reference(
    reader: &mut dyn Reader,
) -> Result<WwiseRtpcReference, WwiseMetadataFormatError> {
    Ok(WwiseRtpcReference {
        name: read_bevy_string(reader).await?,
        id: WwiseNameId(read_bevy_u32(reader).await?),
        multiplier: read_bevy_f32(reader).await?,
        shift: read_bevy_f32(reader).await?,
    })
}

fn write_named_ids(
    writer: &mut impl Write,
    values: &[WwiseNamedId],
) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, values.len())?;
    for value in values {
        write_named_id(writer, value)?;
    }
    Ok(())
}

fn read_named_ids(reader: &mut impl Read) -> Result<Vec<WwiseNamedId>, WwiseMetadataFormatError> {
    let count = read_count(reader)?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_named_id(reader)?);
    }
    Ok(values)
}

async fn read_bevy_named_ids(
    reader: &mut dyn Reader,
) -> Result<Vec<WwiseNamedId>, WwiseMetadataFormatError> {
    let count = read_bevy_count(reader).await?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_bevy_named_id(reader).await?);
    }
    Ok(values)
}

fn write_named_id(
    writer: &mut impl Write,
    value: &WwiseNamedId,
) -> Result<(), WwiseMetadataFormatError> {
    write_string(writer, &value.name)?;
    write_u32(writer, value.id.0)
}

fn read_named_id(reader: &mut impl Read) -> Result<WwiseNamedId, WwiseMetadataFormatError> {
    Ok(WwiseNamedId {
        name: read_string(reader)?,
        id: WwiseNameId(read_u32(reader)?),
    })
}

async fn read_bevy_named_id(
    reader: &mut dyn Reader,
) -> Result<WwiseNamedId, WwiseMetadataFormatError> {
    Ok(WwiseNamedId {
        name: read_bevy_string(reader).await?,
        id: WwiseNameId(read_bevy_u32(reader).await?),
    })
}

fn read_magic(reader: &mut impl Read, expected: [u8; 8]) -> Result<(), WwiseMetadataFormatError> {
    let mut found = [0; 8];
    reader.read_exact(&mut found)?;
    if found == expected {
        Ok(())
    } else {
        Err(WwiseMetadataFormatError::BadMagic { found })
    }
}

async fn read_bevy_magic(
    reader: &mut dyn Reader,
    expected: &[u8; 8],
) -> Result<(), WwiseMetadataFormatError> {
    let mut found = [0; 8];
    reader.read_exact(&mut found).await?;
    if &found == expected {
        Ok(())
    } else {
        Err(WwiseMetadataFormatError::BadMagic { found })
    }
}

fn read_version(reader: &mut impl Read) -> Result<(), WwiseMetadataFormatError> {
    let version = read_u32(reader)?;
    if version == VERSION {
        Ok(())
    } else {
        Err(WwiseMetadataFormatError::UnsupportedVersion(version))
    }
}

async fn read_bevy_version(reader: &mut dyn Reader) -> Result<(), WwiseMetadataFormatError> {
    let version = read_bevy_u32(reader).await?;
    if version == VERSION {
        Ok(())
    } else {
        Err(WwiseMetadataFormatError::UnsupportedVersion(version))
    }
}

fn write_string(writer: &mut impl Write, value: &str) -> Result<(), WwiseMetadataFormatError> {
    write_len(writer, value.len())?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String, WwiseMetadataFormatError> {
    let len = read_count(reader)?;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

async fn read_bevy_string(reader: &mut dyn Reader) -> Result<String, WwiseMetadataFormatError> {
    let len = read_bevy_count(reader).await?;
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).await?;
    Ok(String::from_utf8(bytes)?)
}

fn write_bool(writer: &mut impl Write, value: bool) -> Result<(), WwiseMetadataFormatError> {
    write_u8(writer, u8::from(value))
}

fn read_bool(reader: &mut impl Read) -> Result<bool, WwiseMetadataFormatError> {
    Ok(read_u8(reader)? != 0)
}

async fn read_bevy_bool(reader: &mut dyn Reader) -> Result<bool, WwiseMetadataFormatError> {
    Ok(read_bevy_u8(reader).await? != 0)
}

fn write_len(writer: &mut impl Write, value: usize) -> Result<(), WwiseMetadataFormatError> {
    let value = u32::try_from(value).map_err(|_| WwiseMetadataFormatError::CountTooLarge)?;
    write_u32(writer, value)
}

fn read_count(reader: &mut impl Read) -> Result<usize, WwiseMetadataFormatError> {
    Ok(read_u32(reader)? as usize)
}

async fn read_bevy_count(reader: &mut dyn Reader) -> Result<usize, WwiseMetadataFormatError> {
    Ok(read_bevy_u32(reader).await? as usize)
}

fn write_u8(writer: &mut impl Write, value: u8) -> Result<(), WwiseMetadataFormatError> {
    writer.write_all(&[value])?;
    Ok(())
}

fn read_u8(reader: &mut impl Read) -> Result<u8, WwiseMetadataFormatError> {
    let mut bytes = [0];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

async fn read_bevy_u8(reader: &mut dyn Reader) -> Result<u8, WwiseMetadataFormatError> {
    let mut bytes = [0];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes[0])
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), WwiseMetadataFormatError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn read_u32(reader: &mut impl Read) -> Result<u32, WwiseMetadataFormatError> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

async fn read_bevy_u32(reader: &mut dyn Reader) -> Result<u32, WwiseMetadataFormatError> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_u64(writer: &mut impl Write, value: u64) -> Result<(), WwiseMetadataFormatError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn read_u64(reader: &mut impl Read) -> Result<u64, WwiseMetadataFormatError> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

async fn read_bevy_u64(reader: &mut dyn Reader) -> Result<u64, WwiseMetadataFormatError> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes).await?;
    Ok(u64::from_le_bytes(bytes))
}

fn write_f32(writer: &mut impl Write, value: f32) -> Result<(), WwiseMetadataFormatError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn read_f32(reader: &mut impl Read) -> Result<f32, WwiseMetadataFormatError> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

async fn read_bevy_f32(reader: &mut dyn Reader) -> Result<f32, WwiseMetadataFormatError> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(f32::from_le_bytes(bytes))
}

#[derive(Debug, Error)]
pub enum WwiseMetadataFormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad Wwise metadata magic {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported Wwise metadata version {0}")]
    UnsupportedVersion(u32),
    #[error("Wwise metadata count exceeds u32")]
    CountTooLarge,
    #[error("Wwise metadata string is not UTF-8: {0}")]
    InvalidUtf8(#[from] FromUtf8Error),
    #[error("invalid Wwise metadata {what}: {value}")]
    InvalidData { what: &'static str, value: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wwise_name_id_uses_wwise_lowercase_fnv_hash() {
        assert_eq!(WwiseNameId::from_name("Play_Foo"), WwiseNameId(0xb255_9ee2));
        assert_eq!(
            WwiseNameId::from_name("Play_Foo"),
            WwiseNameId::from_name("play_foo")
        );
    }

    #[test]
    fn audio_controls_round_trip_binary() {
        let asset = WwiseAudioControlsAsset {
            triggers: vec![WwiseTriggerControl {
                name: "play_foo".to_string(),
                id: AudioControlId::from_name("play_foo"),
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
                    id: WwiseNameId::from_name("Volume"),
                    multiplier: 2.0,
                    shift: -1.0,
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
        };

        let mut bytes = Vec::new();
        write_audio_controls_asset(&asset, &mut bytes).unwrap();
        let decoded = read_audio_controls_asset(&bytes).unwrap();

        assert_eq!(decoded, asset);
    }
}
