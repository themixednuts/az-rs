use std::io;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::prelude::TypePath;
use serde::{Deserialize, Serialize};

use crate::InputMapAsset;

pub const INPUT_MAP_PRODUCT_EXTENSION: &str = "inputmap.bin";

#[derive(Default, TypePath)]
pub struct InputMapAssetLoader;

impl AssetLoader for InputMapAssetLoader {
    type Asset = InputMapAsset;
    type Settings = ();
    type Error = InputMapFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        read_input_map_asset(&bytes)
    }

    fn extensions(&self) -> &[&str] {
        &[INPUT_MAP_PRODUCT_EXTENSION]
    }
}

/// Serialize an input-map asset to the `.inputmap.bin` postcard product.
///
/// # Errors
///
/// Returns [`InputMapFormatError::Codec`] if postcard cannot serialize the
/// product, or [`InputMapFormatError::Io`] if `writer` rejects the write.
pub fn write_input_map_asset(
    asset: &InputMapAsset,
    writer: &mut impl io::Write,
) -> Result<(), InputMapFormatError> {
    let mut input_map = asset
        .input_map
        .iter()
        .map(|(&crc, &index)| (crc, index))
        .collect::<Vec<_>>();
    input_map.sort_unstable_by_key(|&(crc, index)| (index, crc));
    writer.write_all(&postcard::to_allocvec(&InputMapProduct {
        name_map: asset.name_map.clone(),
        input_map,
    })?)?;
    Ok(())
}

/// Deserialize an input-map asset from a `.inputmap.bin` postcard product.
///
/// # Errors
///
/// Returns [`InputMapFormatError::Codec`] if `bytes` is not a valid postcard
/// encoding of the product, or [`InputMapFormatError::DuplicateCrc`] if the
/// product lists the same input CRC twice.
pub fn read_input_map_asset(bytes: &[u8]) -> Result<InputMapAsset, InputMapFormatError> {
    let product = postcard::from_bytes::<InputMapProduct>(bytes)?;
    let mut input_map = std::collections::HashMap::with_capacity(product.input_map.len());
    for (crc, index) in product.input_map {
        if input_map.insert(crc, index).is_some() {
            return Err(InputMapFormatError::DuplicateCrc { crc });
        }
    }
    Ok(InputMapAsset {
        name_map: product.name_map,
        input_map,
    })
}

#[derive(Serialize, Deserialize)]
struct InputMapProduct {
    name_map: Vec<String>,
    input_map: Vec<(u32, i32)>,
}

#[derive(Debug, thiserror::Error)]
pub enum InputMapFormatError {
    #[error("input-map I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("input-map codec failed: {0}")]
    Codec(#[from] postcard::Error),
    #[error("input-map product contains duplicate CRC {crc:#010x}")]
    DuplicateCrc { crc: u32 },
}

#[cfg(test)]
mod tests {
    use az_core::crc::Crc32;

    use super::*;
    use crate::InputMapLayout;

    #[test]
    fn derived_input_map_round_trips_as_a_runtime_product() {
        let interact_crc = Crc32::from_str_lower("ui_interact").value();
        let quickslot_crc = Crc32::from_str_lower("Use_Quickslot_1").value();
        let asset = InputMapAsset {
            name_map: vec!["ui_interact".to_owned(), "Use_Quickslot_1".to_owned()],
            input_map: std::collections::HashMap::from([(interact_crc, 0), (quickslot_crc, 1)]),
        };
        let mut product = Vec::new();
        write_input_map_asset(&asset, &mut product).expect("write input map");
        let decoded = read_input_map_asset(&product).expect("read input map");
        let layout = InputMapLayout::try_from(&decoded).expect("valid layout");

        assert_eq!(
            layout.name(layout.resolve("ui_interact").unwrap()),
            Some("ui_interact")
        );
        assert_eq!(
            decoded
                .input_map
                .get(&Crc32::from_str_lower("ui_interact").value()),
            Some(&0)
        );

        let mut repeated = Vec::new();
        write_input_map_asset(&asset, &mut repeated).expect("write input map again");
        assert_eq!(
            product, repeated,
            "input-map products must be deterministic"
        );
    }
}
