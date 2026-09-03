use az_asset::EngineTextureFormat;
use az_objectstream::context::{ObjectStreamDialect, ObjectStreamReadContext};
use az_objectstream::lookup::LumberyardHashes;
use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lyshine_canvas::{UiCanvasTransformError, transform_ui_canvas_asset};

const UI_CANVAS_XML: &[u8] = br#"<ObjectStream version="3">
<Class name="AZ::Entity" type="{75651658-8663-478D-9090-2432DFCAFA44}">
  <Class name="AZ::EntityId" field="id" type="{6383F1D3-BB27-4E6B-A49A-6409B2059EAA}">
    <Class name="AZ::u64" field="id" value="1" type="{D6597933-47CD-4FC8-B911-63F3E2B0993A}"/>
  </Class>
  <Class name="AZStd::string" field="Name" value="CanvasEntity" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
  <Class name="AZStd::vector" field="Components" type="{A60E3E61-1FF6-4982-B6B8-9E4350C4C679}">
    <Class name="LyShine::UiCanvasComponent" type="{50B8CF6C-B19A-4D86-AFE9-96EFB820D422}">
      <Class name="AZ::EntityId" field="RootElement" type="{6383F1D3-BB27-4E6B-A49A-6409B2059EAA}">
        <Class name="AZ::u64" field="id" value="2" type="{D6597933-47CD-4FC8-B911-63F3E2B0993A}"/>
      </Class>
      <Class name="Vector2" field="CanvasSize" value="1920 1080" type="{3D80F623-C85C-4741-90D0-E4E66164E6BF}"/>
    </Class>
  </Class>
</Class>
<Class name="AZ::Entity" type="{75651658-8663-478D-9090-2432DFCAFA44}">
  <Class name="AZ::EntityId" field="id" type="{6383F1D3-BB27-4E6B-A49A-6409B2059EAA}">
    <Class name="AZ::u64" field="id" value="2" type="{D6597933-47CD-4FC8-B911-63F3E2B0993A}"/>
  </Class>
  <Class name="AZStd::string" field="Name" value="Root" type="{03AAAB3F-5C47-5A66-9EBC-D5FA4DB353C9}"/>
</Class>
</ObjectStream>"#;

fn bench_ui_canvas_transform(c: &mut Criterion) {
    let context =
        ObjectStreamReadContext::new(LumberyardHashes::new(), ObjectStreamDialect::default());
    let mut group = c.benchmark_group("ui_canvas_transform");
    group.throughput(Throughput::Bytes(UI_CANVAS_XML.len() as u64));
    group.bench_function("minimal_xml", |b| {
        b.iter(|| {
            transform_ui_canvas_asset(
                black_box(UI_CANVAS_XML),
                &context,
                EngineTextureFormat::Dds,
                |source_path| Ok::<_, UiCanvasTransformError>(source_path.to_string()),
            )
            .expect("transform UI canvas")
        });
    });
    group.finish();
}

criterion_group!(benches, bench_ui_canvas_transform);
criterion_main!(benches);
