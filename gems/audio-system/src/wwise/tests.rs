use bevy::prelude::*;

use super::*;
use crate::AudioControlId;
use crate::AudioSystemAssetPlugin;

#[test]
fn wwise_constants_match_lumberyard_source() {
    assert_eq!(WWISE_DEFAULT_BANKS_PATH, "sounds/wwise/");
    assert_eq!(WWISE_EXTERNAL_SOURCES_PATH, "external");
    assert_eq!(WWISE_CONFIG_FILE, "wwise_config.json");
    assert_eq!(WWISE_BANK_EXTENSION, ".bnk");
    assert_eq!(WWISE_MEDIA_EXTENSION, ".wem");
    assert_eq!(WWISE_INIT_BANK, "init.bnk");
    assert!(is_wwise_asset_name("foo.BNK"));
    assert!(is_wwise_asset_name("foo.wem"));
    assert!(!is_wwise_asset_name("foo.bin"));
}

#[test]
fn wwise_section_id_round_trips_tag_bytes() {
    let id = WwiseSectionId::from_tag(*b"BKHD");

    assert_eq!(id, WwiseSectionId::BKHD);
    assert_eq!(id.tag(), *b"BKHD");
    assert_eq!(id.tag_string(), "BKHD");
}

#[test]
fn wwise_bank_parser_reads_sections_media_and_hierarchy() {
    let bytes = sample_sound_bank_bytes();

    let bank = WwiseSoundBank::parse(&bytes).unwrap();

    assert_eq!(
        bank.header,
        Some(WwiseBankHeader {
            version: 123,
            bank_id: WwiseBankId(456),
            language_id: Some(789),
            feedback_in_bank: Some(0),
        })
    );
    assert!(bank.has_section(WwiseSectionId::BKHD));
    assert!(bank.has_section(WwiseSectionId::DIDX));
    assert!(bank.has_section(WwiseSectionId::DATA));
    assert!(bank.has_section(WwiseSectionId::HIRC));
    assert_eq!(
        bank.media,
        vec![WwiseMediaEntry {
            id: WwiseMediaId(100),
            offset: 4,
            size: 8,
        }]
    );
    assert_eq!(
        bank.hierarchy,
        vec![WwiseHierarchyObject {
            kind: WwiseHierarchyObjectKind::EVENT,
            object_id: WwiseObjectId(77),
            data_offset: 81,
            data_size: 17,
            event_action_count: Some(3),
        }]
    );
    let event = bank.hierarchy[0].event(&bytes).unwrap().unwrap();
    assert_eq!(event.object_id(), WwiseObjectId(77));
    assert_eq!(event.action_count(), 3);
    assert_eq!(
        event.action_ids().collect::<Vec<_>>(),
        vec![WwiseObjectId(10), WwiseObjectId(11), WwiseObjectId(12)]
    );
}

#[test]
fn wwise_sound_bank_asset_preserves_bytes_and_metadata() {
    let bytes = sample_sound_bank_bytes();

    let asset = WwiseSoundBankAsset::from_bytes(&bytes).unwrap();

    assert_eq!(asset.bytes(), bytes);
    assert_eq!(
        asset.bank.header.map(|header| header.bank_id),
        Some(WwiseBankId(456))
    );
    assert_eq!(asset.bank.media.len(), 1);

    assert_eq!(
        WwiseBankSummary::from_asset(&asset),
        WwiseBankSummary {
            bank_id: Some(456),
            sections: 4,
            media_entries: 1,
            hierarchy_objects: 1,
            event_objects: 1,
            event_actions: 3,
        }
    );
    let mut collection = WwiseCollectionSummary::default();
    collection.add_summary(WwiseAssetSummary::Bank(WwiseBankSummary::from_asset(
        &asset,
    )));
    assert_eq!(
        collection.to_string(),
        "  files: 1\n  banks: 1\n  media: 0\n  bank sections: 4\n  embedded media refs: 1\n  HIRC objects: 1\n  Event objects: 1\n  Event actions: 3\n"
    );

    let mut inspection = WwiseInspection::default();
    inspection.add_file_summary(
        inspect_wwise_asset_file("sounds/wwise/init.bnk", &bytes).expect("inspect bank"),
    );
    assert_eq!(
        inspection.report(20).to_string(),
        "sounds/wwise/init.bnk: bank id=456, 4 sections, 1 media refs, 1 HIRC objects, 1 events, 3 actions\n  files: 1\n  banks: 1\n  media: 0\n  bank sections: 4\n  embedded media refs: 1\n  HIRC objects: 1\n  Event objects: 1\n  Event actions: 3\n"
    );
}

#[test]
fn inspect_wwise_asset_files_aggregates_file_results() {
    let path = std::env::temp_dir().join(format!(
        "az-rs-audio-system-{}-init.bnk",
        std::process::id()
    ));
    std::fs::write(&path, sample_sound_bank_bytes()).expect("write bank");

    let inspection = inspect_wwise_asset_files([&path]).expect("inspect wwise files");

    assert_eq!(inspection.rows.len(), 1);
    assert_eq!(inspection.totals.files, 1);
    assert_eq!(inspection.totals.banks, 1);
    assert_eq!(inspection.totals.sections, 4);

    std::fs::remove_file(path).expect("remove bank");
}

#[test]
fn wwise_media_asset_preserves_encoded_bytes() {
    let bytes = sample_wem_bytes();

    let asset = WwiseMediaAsset::from_bytes(&bytes).unwrap();

    assert_eq!(asset.bytes(), bytes);
    assert!(asset.info.has_chunk(WwiseMediaChunkId::FMT));
    assert!(asset.info.has_chunk(WwiseMediaChunkId::DATA));
    assert_eq!(asset.info.chunks.len(), 2);
    assert_eq!(
        WwiseMediaSummary::from_asset(&asset),
        WwiseMediaSummary {
            bytes: bytes.len(),
            chunks: 2,
        }
    );
}

#[test]
fn wwise_media_parser_rejects_invalid_riff_magic() {
    let mut bytes = sample_wem_bytes();
    bytes[0..4].copy_from_slice(b"NOPE");

    assert_eq!(
        WwiseMediaAsset::from_bytes(&bytes).unwrap_err().to_string(),
        "failed to parse Wwise media: invalid Wwise media RIFF magic [78, 79, 80, 69]"
    );
}

#[test]
fn wwise_trigger_bank_map_reads_records() {
    let mut bytes = Vec::new();
    bytes.extend(123u32.to_le_bytes());
    bytes.extend(10u32.to_le_bytes());
    bytes.extend(20u32.to_le_bytes());
    bytes.extend(30u32.to_le_bytes());

    let map = WwiseTriggerBankMap::parse(&bytes).unwrap();
    let entries = map.entries().collect::<Vec<_>>();

    assert_eq!(map.len(), 1);
    assert_eq!(
        entries[0],
        WwiseTriggerBankMapEntry {
            bank_id: WwiseBankId(123),
            control_ids: [AudioControlId(10), AudioControlId(20), AudioControlId(30)]
        }
    );
}

#[test]
fn audio_system_asset_plugin_registers_wwise_assets() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(AudioSystemAssetPlugin);

    assert!(
        app.world()
            .contains_resource::<Assets<WwiseSoundBankAsset>>()
    );
    assert!(app.world().contains_resource::<Assets<WwiseMediaAsset>>());
}

#[test]
fn wwise_bank_parser_rejects_truncated_section() {
    let mut bytes = Vec::new();
    bytes.extend(*b"BKHD");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());

    assert_eq!(
        WwiseSoundBank::parse(&bytes),
        Err(WwiseSoundBankParseError::SectionOutOfBounds {
            section: WwiseSectionId::BKHD
        })
    );
}

#[test]
fn wwise_bank_parser_rejects_invalid_didx_size() {
    let mut bytes = Vec::new();
    push_section(
        &mut bytes,
        *b"BKHD",
        &[1u32.to_le_bytes(), 2u32.to_le_bytes()].concat(),
    );
    push_section(&mut bytes, *b"DIDX", &[0; 5]);

    assert_eq!(
        WwiseSoundBank::parse(&bytes),
        Err(WwiseSoundBankParseError::InvalidDidxSize { size: 5 })
    );
}

#[test]
fn wwise_bank_parser_rejects_invalid_hirc_object_size() {
    let mut bytes = Vec::new();
    push_section(
        &mut bytes,
        *b"BKHD",
        &[1u32.to_le_bytes(), 2u32.to_le_bytes()].concat(),
    );

    let mut hirc = Vec::new();
    hirc.extend(1u32.to_le_bytes());
    hirc.push(2);
    hirc.extend(3u32.to_le_bytes());
    push_section(&mut bytes, *b"HIRC", &hirc);

    assert_eq!(
        WwiseSoundBank::parse(&bytes),
        Err(WwiseSoundBankParseError::InvalidHircObjectSize { index: 0, size: 3 })
    );
}

#[test]
fn wwise_bank_parser_rejects_invalid_event_action_list() {
    let mut bytes = Vec::new();
    push_section(
        &mut bytes,
        *b"BKHD",
        &[1u32.to_le_bytes(), 2u32.to_le_bytes()].concat(),
    );

    let mut hirc = Vec::new();
    hirc.extend(1u32.to_le_bytes());
    hirc.push(WwiseHierarchyObjectKind::EVENT.as_u8());
    hirc.extend(5u32.to_le_bytes());
    hirc.extend(77u32.to_le_bytes());
    hirc.push(1);
    push_section(&mut bytes, *b"HIRC", &hirc);

    assert_eq!(
        WwiseSoundBank::parse(&bytes),
        Err(WwiseSoundBankParseError::HircEventActionListOutOfBounds {
            object_id: WwiseObjectId(77)
        })
    );
}

#[test]
fn wwise_bank_parser_rejects_media_past_data_section() {
    let mut bytes = Vec::new();
    push_section(
        &mut bytes,
        *b"BKHD",
        &[1u32.to_le_bytes(), 2u32.to_le_bytes()].concat(),
    );
    push_section(
        &mut bytes,
        *b"DIDX",
        &[100u32.to_le_bytes(), 8u32.to_le_bytes(), 8u32.to_le_bytes()].concat(),
    );
    push_section(&mut bytes, *b"DATA", &[0; 12]);

    assert_eq!(
        WwiseSoundBank::parse(&bytes),
        Err(WwiseSoundBankParseError::InvalidMediaRange {
            media_id: WwiseMediaId(100)
        })
    );
}

fn push_section(bytes: &mut Vec<u8>, tag: [u8; 4], payload: &[u8]) {
    let size = u32::try_from(payload.len()).expect("test section payload fits in u32");
    bytes.extend(tag);
    bytes.extend(size.to_le_bytes());
    bytes.extend(payload);
}

fn sample_sound_bank_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    push_section(
        &mut bytes,
        *b"BKHD",
        &[
            123u32.to_le_bytes(),
            456u32.to_le_bytes(),
            789u32.to_le_bytes(),
            0u32.to_le_bytes(),
        ]
        .concat(),
    );
    push_section(
        &mut bytes,
        *b"DIDX",
        &[100u32.to_le_bytes(), 4u32.to_le_bytes(), 8u32.to_le_bytes()].concat(),
    );
    push_section(&mut bytes, *b"DATA", &[0; 12]);

    let mut hirc = Vec::new();
    hirc.extend(1u32.to_le_bytes());
    hirc.push(WwiseHierarchyObjectKind::EVENT.as_u8());
    hirc.extend(17u32.to_le_bytes());
    hirc.extend(77u32.to_le_bytes());
    hirc.push(3);
    hirc.extend(10u32.to_le_bytes());
    hirc.extend(11u32.to_le_bytes());
    hirc.extend(12u32.to_le_bytes());
    push_section(&mut bytes, *b"HIRC", &hirc);
    bytes
}

fn sample_wem_bytes() -> Vec<u8> {
    let mut chunks = Vec::new();
    chunks.extend(*b"fmt ");
    chunks.extend(4u32.to_le_bytes());
    chunks.extend([0x42, 0, 0, 0]);
    chunks.extend(*b"data");
    chunks.extend(3u32.to_le_bytes());
    chunks.extend([1, 2, 3]);
    chunks.push(0);

    let riff_size = u32::try_from(chunks.len() + 4).expect("test RIFF payload fits in u32");
    let mut bytes = Vec::new();
    bytes.extend(*b"RIFF");
    bytes.extend(riff_size.to_le_bytes());
    bytes.extend(*b"WAVE");
    bytes.extend(chunks);
    bytes
}
