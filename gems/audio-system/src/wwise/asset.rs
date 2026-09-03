//! Wwise Bevy assets and loaders.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;

use super::bank::{WwiseEventObject, WwiseHierarchyObject, WwiseSoundBank};
use super::config::{WWISE_MEDIA_ASSET_EXTENSIONS, WWISE_SOUND_BANK_ASSET_EXTENSIONS};
use super::error::WwiseAssetLoadError;
use super::media_file::WwiseMediaInfo;

/// A Wwise `.bnk` soundbank and its parsed metadata.
#[derive(Asset, TypePath, Debug, Clone, Default, PartialEq, Eq)]
pub struct WwiseSoundBankAsset {
    pub bank: WwiseSoundBank,
    pub bytes: Box<[u8]>,
}

impl WwiseSoundBankAsset {
    /// Construct a soundbank asset by copying a borrowed byte buffer.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::from_vec`] returns.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WwiseAssetLoadError> {
        Self::from_vec(bytes.to_vec())
    }

    /// Construct a soundbank asset from an owned byte buffer.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseAssetLoadError::SoundBank`] if `bytes` is not a well-formed
    /// Wwise `.bnk` container — no `BKHD` section, a section or `HIRC` object
    /// whose declared range runs past the end of the bank, or a malformed
    /// `DIDX`/`HIRC` record.
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, WwiseAssetLoadError> {
        let bank = WwiseSoundBank::parse(&bytes)?;
        Ok(Self {
            bank,
            bytes: bytes.into_boxed_slice(),
        })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Borrow the payload bytes of a hierarchy object inside this bank.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseAssetLoadError::SoundBank`] wrapping
    /// [`WwiseSoundBankParseError::HircObjectDataOutOfBounds`] if `object`'s
    /// data range does not lie inside this bank's bytes.
    ///
    /// [`WwiseSoundBankParseError::HircObjectDataOutOfBounds`]: super::error::WwiseSoundBankParseError::HircObjectDataOutOfBounds
    pub fn hierarchy_object_body(
        &self,
        object: WwiseHierarchyObject,
    ) -> Result<&[u8], WwiseAssetLoadError> {
        Ok(object.body(self.bytes())?)
    }

    /// Decode `object` as an Event object, or `None` if it is another kind.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseAssetLoadError::SoundBank`] if `object`'s data range runs
    /// past the end of the bank, or if its action list is truncated or declares
    /// a count that overflows the object payload.
    pub fn event_object(
        &self,
        object: WwiseHierarchyObject,
    ) -> Result<Option<WwiseEventObject<'_>>, WwiseAssetLoadError> {
        Ok(object.event(self.bytes())?)
    }
}

/// A Wwise `.wem` encoded media file.
#[derive(Asset, TypePath, Debug, Clone, Default, PartialEq, Eq)]
pub struct WwiseMediaAsset {
    pub info: WwiseMediaInfo,
    pub bytes: Box<[u8]>,
}

impl WwiseMediaAsset {
    /// Construct a media asset by copying a borrowed byte buffer.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::from_vec`] returns.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WwiseAssetLoadError> {
        Self::from_vec(bytes.to_vec())
    }

    /// Construct a media asset from an owned byte buffer.
    ///
    /// # Errors
    ///
    /// Returns [`WwiseAssetLoadError::Media`] if `bytes` is shorter than 12
    /// bytes, is missing the `RIFF`/`WAVE` magic, declares a RIFF size longer
    /// than the buffer, or contains a chunk whose payload runs past the
    /// declared end of the container.
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, WwiseAssetLoadError> {
        let info = WwiseMediaInfo::parse(&bytes)?;
        Ok(Self {
            info,
            bytes: bytes.into_boxed_slice(),
        })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

/// Bevy asset loader for Wwise `.bnk` soundbanks.
#[derive(Default, TypePath)]
pub struct WwiseSoundBankAssetLoader;

impl AssetLoader for WwiseSoundBankAssetLoader {
    type Asset = WwiseSoundBankAsset;
    type Settings = ();
    type Error = WwiseAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        WwiseSoundBankAsset::from_vec(bytes)
    }

    fn extensions(&self) -> &[&str] {
        WWISE_SOUND_BANK_ASSET_EXTENSIONS
    }
}

/// Bevy asset loader for Wwise `.wem` encoded media.
#[derive(Default, TypePath)]
pub struct WwiseMediaAssetLoader;

impl AssetLoader for WwiseMediaAssetLoader {
    type Asset = WwiseMediaAsset;
    type Settings = ();
    type Error = WwiseAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        WwiseMediaAsset::from_vec(bytes)
    }

    fn extensions(&self) -> &[&str] {
        WWISE_MEDIA_ASSET_EXTENSIONS
    }
}
