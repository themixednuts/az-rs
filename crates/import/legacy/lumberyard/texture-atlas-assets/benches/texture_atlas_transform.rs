use std::fmt::Write as _;

use az_asset::EngineTextureFormat;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use texture_atlas_assets::transform_texture_atlas_index;

fn atlas_xml(regions: usize) -> Vec<u8> {
    let mut xml = String::from(
        r#"<ObjectStream version="3"><Class name="TextureAtlasImpl" version="1" type="{2CA51C61-1B5F-4480-A257-F28D8944AA35}"><Class name="AZStd::unordered_map" field="Coordinate Pairs" type="{23D80BE7-76FE-5C35-B4EA-A4492B2B058C}">"#,
    );
    for index in 0..regions {
        let left = (index % 64) * 4;
        let top = (index / 64) * 4;
        write!(
            xml,
            r#"<Class name="AZStd::pair" field="element" type="{{657F8527-0117-54A0-91E7-6E67A7464BC5}}"><Class name="AZStd::string" field="value1" value="Icon{index}" type="{{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}}"/><Class name="AtlasCoordinates" field="value2" version="1" type="{{FC5D6A60-1056-4F6C-96F7-6A47912F8A35}}"><Class name="int" field="Left" value="{left}" type="{{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}}"/><Class name="int" field="Top" value="{top}" type="{{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}}"/><Class name="int" field="Width" value="4" type="{{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}}"/><Class name="int" field="Height" value="4" type="{{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}}"/></Class></Class>"#,
        )
        .expect("writing to a String cannot fail");
    }
    xml.push_str(
        r#"</Class><Class name="int" field="Width" value="256" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/><Class name="int" field="Height" value="256" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/></Class></ObjectStream>"#,
    );
    xml.into_bytes()
}

fn bench_texture_atlas_transform(c: &mut Criterion) {
    let bytes = atlas_xml(256);
    c.bench_function("texture_atlas_transform_256_regions", |b| {
        b.iter(|| {
            transform_texture_atlas_index(
                black_box(&bytes),
                black_box("LyShineUI/Images/TextureAtlas/Common.texatlasidx"),
                black_box(EngineTextureFormat::Dds),
            )
            .unwrap()
        });
    });
}

criterion_group!(benches, bench_texture_atlas_transform);
criterion_main!(benches);
