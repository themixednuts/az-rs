use az_asset::EngineTextureFormat;
use criterion::{Criterion, criterion_group, criterion_main};
use lmbr_central_assets::parse_material_asset;
use std::hint::black_box;

fn material_xml(texture_count: usize) -> Vec<u8> {
    use std::fmt::Write as _;

    let mut xml =
        String::from(r#"<Material Name="bench" Shader="Illum" Diffuse="0.5,0.5,0.5,1"><Textures>"#);
    for index in 0..texture_count {
        let _ = write!(
            xml,
            r#"<Texture Map="Diffuse" File="Objects/Bench/Texture_{index}.tif" Filter="3" IsTileU="1" IsTileV="1" TexType="1"><TexMod TileU="1" TileV="1"/></Texture>"#,
        );
    }
    xml.push_str("</Textures></Material>");
    xml.into_bytes()
}

fn bench_material_transform(c: &mut Criterion) {
    let bytes = material_xml(128);
    c.bench_function("material_transform_128_textures", |b| {
        b.iter(|| {
            parse_material_asset(
                black_box("Materials/Bench/Material.mtl"),
                black_box(&bytes),
                black_box(EngineTextureFormat::Dds),
            )
            .unwrap()
        });
    });
}

criterion_group!(benches, bench_material_transform);
criterion_main!(benches);
