use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

/// Assert a bit-exact `f32`.
///
/// These transforms must round-trip authored values through RON without
/// rounding, so an epsilon comparison would stop testing what matters.
#[track_caller]
fn assert_exact_f32(actual: f32, expected: f32) {
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "expected {expected} bit-exactly, got {actual}"
    );
}

use super::*;
use crate::source_schemas;

mod material_override;
mod particle_library;

#[test]
fn transforms_post_effect_group_to_authoring_source() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new(
            "Libs/PostEffectGroups/Spyglass_Overlay.xml",
            br#"<PostEffectGroup priority="1" hold="1" fadeDistance=".5">
<!-- disabled note -->
<Effect name="Global">
    <Param name="User_Brightness" floatValue=".8"/>
    <Param name="OverlayColor" colorValue="0.5,0.75,1,1"/>
</Effect>
<Effect name="ScreenFader">
    <Param name="Texture" textureValue="Textures\VFX\Misc\Screen_Fade_01.tif"/>
</Effect>
<BlendIn curve="linear">
    <Key time="0" Value="0"/>
    <Key time=".35" Value="1"/>
</BlendIn>
<BlendOut curve="smooth">
    <Key time="0" value="1"/>
    <Key time="1" value="0"/>
</BlendOut>
</PostEffectGroup>"#,
        ))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(artifact.path, "posteffects/spyglass_overlay.posteffect.ron");
    assert_eq!(artifact.schema, source_schemas::POST_EFFECT_GROUP);

    let source: PostEffectGroupSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(
        source.source_path,
        "libs/posteffectgroups/spyglass_overlay.xml"
    );
    assert_eq!(source.priority, 1);
    assert!(source.hold);
    assert_eq!(source.fade_distance, Some(0.5));
    assert_eq!(source.comments, ["disabled note"]);
    assert_eq!(source.effects.len(), 2);
    assert_eq!(source.effects[0].name, "Global");
    assert_eq!(
        source.effects[0].params[0].value,
        PostEffectParamValueSource::Float(PostEffectFloatParamValueSource { value: 0.8 })
    );
    assert_eq!(
        source.effects[0].params[1].value,
        PostEffectParamValueSource::Color(PostEffectColorParamValueSource {
            value: ColorRgbaSource {
                r: 0.5,
                g: 0.75,
                b: 1.0,
                a: 1.0,
            },
        })
    );
    assert_eq!(
        source.effects[1].params[0].value,
        PostEffectParamValueSource::Texture(PostEffectTextureParamValueSource {
            path: r"Textures\VFX\Misc\Screen_Fade_01.tif".to_string(),
        })
    );
    let blend_in = source.blend_in.expect("blend in");
    assert_eq!(blend_in.curve, PostEffectBlendCurve::Linear);
    assert_exact_f32(blend_in.keys[1].time, 0.35);
    assert_exact_f32(blend_in.keys[1].value, 1.0);
    let blend_out = source.blend_out.expect("blend out");
    assert_eq!(blend_out.curve, PostEffectBlendCurve::Smooth);
    assert_eq!(blend_out.keys.len(), 2);
}

#[test]
fn recovers_extra_legacy_effect_end_tag() {
    let source = PostEffectGroupSource::from_legacy(
        "libs/posteffectgroups/aoe_silence.xml",
        br#"<PostEffectGroup priority="1" hold="0">
<Effect name="Global"><Param name="Amount" floatValue="1"/></Effect>
</Effect>
<Effect name="ScreenFader"><Param name="Texture" stringValue="Textures\fade.tif"/></Effect>
</PostEffectGroup>"#,
    )
    .unwrap();

    assert_eq!(source.effects.len(), 2);
    assert_eq!(source.effects[1].name, "ScreenFader");
}

#[test]
fn recovers_stray_legacy_comment_close_marker() {
    let source = PostEffectGroupSource::from_legacy(
        "libs/posteffectgroups/aenaga_massiveaoe_tell.xml",
        br#"<PostEffectGroup priority="1" hold="1">
<BlendIn curve="linear"><Key time="0" Value="0"/><Key time="3" Value="1"/></BlendIn>-->
<BlendOut curve="linear"><Key time="0" Value="1"/><Key time="0.1" Value="0"/></BlendOut>
</PostEffectGroup>"#,
    )
    .unwrap();

    assert_eq!(source.blend_in.unwrap().keys.len(), 2);
    assert_eq!(source.blend_out.unwrap().keys.len(), 2);
}

#[test]
fn xml_source_paths_claim_only_promoted_xml_families() {
    assert_eq!(
        post_effect_group_source_path("libs/posteffectgroups/default.xml").as_deref(),
        Some("posteffects/default.posteffect.ron")
    );
    assert_eq!(
        xml_source_path("libs/posteffectgroups/blackout.xml").as_deref(),
        Some("posteffects/blackout.posteffect.ron")
    );
    assert!(is_legacy_xml_source("libs/posteffectgroups/default.xml"));
    assert_eq!(
        level_info_source_path("levels/ftue_v2/levelinfo.xml").as_deref(),
        Some("levels/ftue_v2/levelinfo.levelinfo.ron")
    );
    assert!(is_legacy_xml_source("levels/ftue_v2/levelinfo.xml"));
    assert!(!is_legacy_xml_source("levels/foo/leveldata.xml"));
    assert!(!is_legacy_xml_source(
        "libs/gameaudio/wwise/atl_controls.xml"
    ));
    assert!(xml_source_path("default.xml").is_none());
}

#[test]
fn transforms_complete_level_info_without_terrain() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new(
            "Levels/FTUE_V2/levelinfo.xml",
            br#"<LevelInfo SandboxVersion="0.0.0.0" Name="@devassets@/Levels/FTUE_V2">
<Missions><Mission Name="Mission0" Description=""/></Missions>
</LevelInfo>"#,
        ))
        .unwrap();

    let artifact = output.artifact().expect("level-info authoring artifact");
    assert_eq!(artifact.path, "levels/ftue_v2/levelinfo.levelinfo.ron");
    assert_eq!(artifact.schema, source_schemas::LEVEL_INFO);
    let source: LevelInfoSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(source.source_path, "levels/ftue_v2/levelinfo.xml");
    assert_eq!(source.sandbox_version, "0.0.0.0");
    assert_eq!(source.name, "@devassets@/Levels/FTUE_V2");
    assert_eq!(source.heightmap_size, None);
    assert_eq!(source.terrain, None);
    assert_eq!(
        source.missions,
        vec![LevelMissionSource {
            name: "Mission0".into(),
            description: String::new(),
        }]
    );
}

#[test]
fn transforms_complete_level_info_with_terrain() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new(
            "levels/main_menu/levelinfo.xml",
            br#"<LevelInfo SandboxVersion="1.12.0.1" Name="@devassets@/Levels/Main_Menu" HeightmapSize="400">
<TerrainInfo HeightmapSize="1024" UnitSize="2" SectorSize="64" SectorsTableSize="32" HeightmapZRatio="0" OceanWaterLevel="25"/>
<Missions><Mission Name="Mission0" Description=""/></Missions>
</LevelInfo>"#,
        ))
        .unwrap();

    let artifact = output.artifact().expect("level-info authoring artifact");
    let source: LevelInfoSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(source.heightmap_size, Some(400));
    assert_eq!(
        source.terrain,
        Some(LevelTerrainInfoSource {
            heightmap_size: 1024,
            unit_size: 2,
            sector_size: 64,
            sectors_table_size: 32,
            heightmap_z_ratio: 0.0,
            ocean_water_level: 25.0,
        })
    );
}

#[test]
fn transforms_time_of_day_to_authoring_source() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new(
            "Libs/TimeOfDay/Frontend/timeofday_frontend_aurora_a.xml",
            br#"<TimeOfDay Time="0" TimeStart="0" TimeEnd="23.983334" TimeAnimSpeed="0.1">
<Variable Name="Sun intensity" Value="4500">
  <Spline Keys="0:4500:36,0.5:100000:18,"/>
</Variable>
<Variable Name="Sun color" Color="0.49693301,0.730461,1">
  <Spline Keys="0:(0.496933:0.730461:1):36,0.25:(1:0.715694:0.401978):65572,"/>
</Variable>
</TimeOfDay>"#,
        ))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(
        artifact.path,
        "timeofday/frontend/timeofday_frontend_aurora_a.timeofday.ron"
    );
    assert_eq!(artifact.schema, source_schemas::TIME_OF_DAY);

    let source: TimeOfDayProfileSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(
        source.source_path,
        "libs/timeofday/frontend/timeofday_frontend_aurora_a.xml"
    );
    assert_exact_f32(source.time, 0.0);
    assert_exact_f32(source.start_time, 0.0);
    assert_exact_f32(source.end_time, 23.983_334);
    assert_exact_f32(source.animation_speed, 0.1);
    assert_eq!(source.variables.len(), 2);

    assert_eq!(source.variables[0].name, "Sun intensity");
    assert_eq!(
        source.variables[0].value,
        TimeOfDayValueSource::Float(TimeOfDayFloatValueSource { value: 4500.0 })
    );
    assert_eq!(source.variables[0].spline.keys.len(), 2);
    assert_eq!(
        source.variables[0].spline.keys[0].value,
        TimeOfDayValueSource::Float(TimeOfDayFloatValueSource { value: 4500.0 })
    );
    assert_eq!(
        source.variables[0].spline.keys[0].flags.in_tangent,
        SplineTangentSource::Linear
    );
    assert_eq!(
        source.variables[0].spline.keys[0].flags.out_tangent,
        SplineTangentSource::Linear
    );
    assert!(!source.variables[0].spline.keys[0].flags.unified);
    assert_eq!(
        source.variables[0].spline.keys[0].flags.selected_dimensions,
        0
    );
    assert_eq!(source.variables[0].spline.keys[0].flags.unknown_bits, 0);

    assert_eq!(source.variables[1].name, "Sun color");
    assert_eq!(
        source.variables[1].value,
        TimeOfDayValueSource::Color(TimeOfDayColorValueSource {
            value: ColorRgbSource {
                r: 0.496_933,
                g: 0.730_461,
                b: 1.0,
            },
        })
    );
    assert_eq!(
        source.variables[1].spline.keys[1].flags.selected_dimensions,
        1
    );
}

#[test]
fn xml_source_paths_claim_time_of_day_profiles() {
    assert_eq!(
        time_of_day_source_path("libs/timeofday/frontend/timeofday_frontend_aurora_a.xml")
            .as_deref(),
        Some("timeofday/frontend/timeofday_frontend_aurora_a.timeofday.ron")
    );
    assert_eq!(
        xml_source_path("engineassets/levelforsliceediting/leveldata/timeofday.xml").as_deref(),
        Some("timeofday/engineassets/levelforsliceediting/leveldata/timeofday.timeofday.ron")
    );
    assert!(is_legacy_xml_source(
        "libs/timeofday/frontend/timeofday_frontend_aurora_a.xml"
    ));
    assert!(is_legacy_xml_source(
        "engineassets/levelforsliceediting/leveldata/timeofday.xml"
    ));
    assert!(time_of_day_source_path("libs/posteffectgroups/default.xml").is_none());
}

#[test]
fn transforms_material_effects_fx_library_to_authoring_source() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new(
            "Libs/MaterialEffects/FXLibs/collisions.xml",
            br#"<FXLib type="collisions">
<Effect name="default" delay="0.25">
  <Audio trigger="default">
    <Switch name="SurfaceType" state="dirt"/>
  </Audio>
  <Particle>
    <Name direction="normal" minscale="1" maxscale="2">Particles.Dust</Name>
  </Particle>
</Effect>
</FXLib>"#,
        ))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(
        artifact.path,
        "materials/effects/fxlibs/collisions.materialeffects.ron"
    );
    assert_eq!(artifact.schema, source_schemas::MATERIAL_EFFECTS);

    let source: MaterialEffectsSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    let MaterialEffectsSource::Library(library) = source else {
        panic!("expected material effects library source");
    };
    assert_eq!(
        library.source_path,
        "libs/materialeffects/fxlibs/collisions.xml"
    );
    assert_eq!(library.kind, "collisions");
    assert_eq!(library.effects.len(), 1);

    let effect = &library.effects[0];
    assert_eq!(effect.name, "default");
    assert_eq!(effect.delay, Some(0.25));
    assert_eq!(effect.resources.len(), 2);

    let MaterialEffectResourceSource::Audio(audio) = &effect.resources[0] else {
        panic!("expected audio resource");
    };
    assert_eq!(audio.trigger, "default");
    assert_eq!(audio.switches[0].name, "SurfaceType");
    assert_eq!(audio.switches[0].state, "dirt");

    let MaterialEffectResourceSource::Particle(particle) = &effect.resources[1] else {
        panic!("expected particle resource");
    };
    assert_eq!(particle.names.len(), 1);
    assert_eq!(particle.names[0].path, "Particles.Dust");
    assert_eq!(particle.names[0].direction.as_deref(), Some("normal"));
    assert_eq!(particle.names[0].min_scale, Some(1.0));
    assert_eq!(particle.names[0].max_scale, Some(2.0));
}

#[test]
fn transforms_material_effects_spreadsheet_to_authoring_source() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new("Libs/MaterialEffects/materialeffects.xml", br#"<Workbook xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
<Worksheet ss:Name="MFX">
  <Table>
    <Row>
      <Cell/>
      <Cell ss:Formula="=surface_col"><Data ss:Type="String">mat_metal</Data></Cell>
      <Cell><Data ss:Type="String">mat_wood</Data></Cell>
    </Row>
    <Row>
      <Cell><Data ss:Type="String">mat_metal</Data></Cell>
      <Cell ss:Formula="=metal_metal"><Data ss:Type="String">collisions:metal_metal</Data></Cell>
      <Cell><Data ss:Type="String" rel_version="Territory_FirstLight">collisions:metal_wood</Data></Cell>
    </Row>
    <Row>
      <Cell><Data ss:Type="String">Jump_Player</Data></Cell>
      <Cell><Data ss:Type="String">jump_player:metal</Data></Cell>
      <Cell><Data ss:Type="String">jump_player:wood</Data></Cell>
    </Row>
  </Table>
</Worksheet>
</Workbook>"#))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(
        artifact.path,
        "materials/effects/materialeffects.materialeffects.ron"
    );
    assert_eq!(artifact.schema, source_schemas::MATERIAL_EFFECTS);

    let source: MaterialEffectsSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    let MaterialEffectsSource::InteractionIndex(index) = source else {
        panic!("expected material effects interaction index source");
    };
    assert_eq!(
        index.source_path,
        "libs/materialeffects/materialeffects.xml"
    );
    assert_eq!(index.worksheet, "MFX");
    assert_eq!(index.columns.len(), 2);
    assert_eq!(index.columns[0].index, 2);
    assert_eq!(index.columns[0].name, "mat_metal");
    assert_eq!(
        index.columns[0].metadata.formula.as_deref(),
        Some("=surface_col")
    );
    assert_eq!(index.rows.len(), 2);

    let surface_row = &index.rows[0];
    assert_eq!(
        surface_row.kind,
        MaterialEffectsInteractionRowKindSource::Surface
    );
    assert_eq!(surface_row.name, "mat_metal");
    assert_eq!(surface_row.entries.len(), 2);
    assert_eq!(surface_row.entries[0].column, "mat_metal");
    assert_eq!(surface_row.entries[0].reference.library, "collisions");
    assert_eq!(surface_row.entries[0].reference.effect, "metal_metal");
    assert_eq!(
        surface_row.entries[1].metadata.rel_version.as_deref(),
        Some("Territory_FirstLight")
    );

    let custom_row = &index.rows[1];
    assert_eq!(
        custom_row.kind,
        MaterialEffectsInteractionRowKindSource::Custom
    );
    assert_eq!(custom_row.name, "Jump_Player");
    assert_eq!(custom_row.entries[0].reference.library, "jump_player");
    assert_eq!(custom_row.entries[0].reference.effect, "metal");
}

#[derive(Default)]
struct MaterialEffectsCorpusCounts {
    files: usize,
    fx_libraries: usize,
    index_files: usize,
    effects: usize,
    audio: usize,
    switches: usize,
    particles: usize,
    empty_particles: usize,
    particle_names: usize,
    decals: usize,
    force_feedback: usize,
    random: usize,
    index_columns: usize,
    index_rows: usize,
    index_entries: usize,
    formulas: usize,
    rel_versions: usize,
    defined_effects: std::collections::BTreeSet<String>,
    referenced_effects: Vec<String>,
}

#[test]
#[ignore = "requires AZOTH_RELEASE_SOURCE pointing at a local release corpus"]
fn transforms_configured_material_effects_corpus() {
    let release_source =
        std::env::var("AZOTH_RELEASE_SOURCE").expect("AZOTH_RELEASE_SOURCE must be set");
    let cache_root = std::path::Path::new(&release_source);

    let mut counts = MaterialEffectsCorpusCounts::default();
    for source_path in known_material_effects_paths(cache_root) {
        let bytes = std::fs::read(cache_root.join(&source_path)).unwrap();
        let output = XmlSourceTransform
            .transform(LegacySourceInput::new(&source_path, &bytes))
            .unwrap();
        let source: MaterialEffectsSource =
            ron::de::from_bytes(&output.artifact().unwrap().bytes).unwrap();

        match source {
            MaterialEffectsSource::Library(library) => {
                count_material_effects_library(&library, &source_path, &mut counts);
            }
            MaterialEffectsSource::InteractionIndex(index) => {
                count_material_effects_index(&index, &source_path, &mut counts);
            }
        }
        counts.files += 1;
    }

    let missing_refs = counts
        .referenced_effects
        .iter()
        .filter(|reference| !counts.defined_effects.contains(reference.as_str()))
        .count();
    let unique_missing_refs = counts
        .referenced_effects
        .iter()
        .filter(|reference| !counts.defined_effects.contains(reference.as_str()))
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    assert_eq!(counts.files, 167);
    assert_eq!(counts.fx_libraries, 166);
    assert_eq!(counts.index_files, 1);
    assert_eq!(counts.effects, 4576);
    assert_eq!(counts.audio, 4576);
    assert_eq!(counts.switches, 3937);
    assert_eq!(counts.particles, 4402);
    assert_eq!(counts.empty_particles, 4158);
    assert_eq!(counts.particle_names, 244);
    assert_eq!(counts.decals, 0);
    assert_eq!(counts.force_feedback, 0);
    assert_eq!(counts.random, 0);
    assert_eq!(counts.index_columns, 26);
    assert_eq!(counts.index_rows, 203);
    assert_eq!(counts.index_entries, 5278);
    assert_eq!(counts.formulas, 5227);
    assert_eq!(counts.rel_versions, 270);
    assert_eq!(missing_refs, 932);
    assert_eq!(unique_missing_refs, 930);
}

fn count_material_effects_library(
    library: &MaterialEffectsLibrarySource,
    source_path: &str,
    counts: &mut MaterialEffectsCorpusCounts,
) {
    assert_eq!(library.source_path, source_path);
    let library_name = source_path
        .rsplit('/')
        .next()
        .unwrap()
        .strip_suffix(".xml")
        .unwrap()
        .to_ascii_lowercase();
    for effect in &library.effects {
        counts.defined_effects.insert(format!(
            "{library_name}:{}",
            effect.name.to_ascii_lowercase()
        ));
        counts.effects += 1;
        for resource in &effect.resources {
            count_material_effect_resource(resource, counts);
        }
    }
    counts.fx_libraries += 1;
}

fn count_material_effects_index(
    index: &MaterialEffectsInteractionIndexSource,
    source_path: &str,
    counts: &mut MaterialEffectsCorpusCounts,
) {
    assert_eq!(index.source_path, source_path);
    counts.index_columns += index.columns.len();
    counts.index_rows += index.rows.len();
    counts.formulas += index
        .columns
        .iter()
        .filter(|column| column.metadata.formula.is_some())
        .count();
    counts.rel_versions += index
        .columns
        .iter()
        .filter(|column| column.metadata.rel_version.is_some())
        .count();
    for row in &index.rows {
        if row.metadata.formula.is_some() {
            counts.formulas += 1;
        }
        if row.metadata.rel_version.is_some() {
            counts.rel_versions += 1;
        }
        counts.index_entries += row.entries.len();
        for entry in &row.entries {
            if entry.metadata.formula.is_some() {
                counts.formulas += 1;
            }
            if entry.metadata.rel_version.is_some() {
                counts.rel_versions += 1;
            }
            counts.referenced_effects.push(format!(
                "{}:{}",
                entry.reference.library.to_ascii_lowercase(),
                entry.reference.effect.to_ascii_lowercase()
            ));
        }
    }
    counts.index_files += 1;
}

fn count_material_effect_resource(
    resource: &MaterialEffectResourceSource,
    counts: &mut MaterialEffectsCorpusCounts,
) {
    match resource {
        MaterialEffectResourceSource::Audio(source) => {
            counts.audio += 1;
            counts.switches += source.switches.len();
        }
        MaterialEffectResourceSource::Particle(source) => {
            counts.particles += 1;
            if source.names.is_empty() {
                counts.empty_particles += 1;
            }
            counts.particle_names += source.names.len();
        }
        MaterialEffectResourceSource::Decal(_) => counts.decals += 1,
        MaterialEffectResourceSource::ForceFeedback(_) => counts.force_feedback += 1,
        MaterialEffectResourceSource::Random(source) => {
            counts.random += 1;
            for child in &source.resources {
                count_material_effect_resource(child, counts);
            }
        }
    }
}

fn known_material_effects_paths(cache_root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_material_effects_paths(
        cache_root,
        &cache_root.join("libs/materialeffects"),
        &mut paths,
    );
    paths.sort();
    paths
}

fn collect_material_effects_paths(
    cache_root: &std::path::Path,
    current: &std::path::Path,
    paths: &mut Vec<String>,
) {
    for entry in std::fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_material_effects_paths(cache_root, &path, paths);
        } else {
            let relative = path
                .strip_prefix(cache_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if material_effects_source_path(&relative).is_some() {
                paths.push(relative);
            }
        }
    }
}

#[test]
#[ignore = "requires AZOTH_RELEASE_SOURCE pointing at a local release corpus"]
fn transforms_configured_post_effect_group_corpus() {
    let release_source =
        std::env::var("AZOTH_RELEASE_SOURCE").expect("AZOTH_RELEASE_SOURCE must be set");
    let cache_root = std::path::Path::new(&release_source);

    let mut files = 0usize;
    let mut effects = 0usize;
    let mut params = 0usize;
    let mut blend_in = 0usize;
    let mut blend_out = 0usize;
    let mut keys = 0usize;

    for source_path in known_post_effect_group_paths(cache_root) {
        let bytes = std::fs::read(cache_root.join(&source_path)).unwrap();
        let output = XmlSourceTransform
            .transform(LegacySourceInput::new(&source_path, &bytes))
            .unwrap_or_else(|error| panic!("{source_path}: {error}"));
        let source: PostEffectGroupSource =
            ron::de::from_bytes(&output.artifact().unwrap().bytes).unwrap();

        assert_eq!(source.source_path, source_path);
        effects += source.effects.len();
        params += source
            .effects
            .iter()
            .map(|effect| effect.params.len())
            .sum::<usize>();
        if let Some(blend) = &source.blend_in {
            blend_in += 1;
            keys += blend.keys.len();
        }
        if let Some(blend) = &source.blend_out {
            blend_out += 1;
            keys += blend.keys.len();
        }
        files += 1;
    }

    assert_eq!(files, 31);
    assert_eq!(effects, 72);
    assert_eq!(params, 407);
    assert_eq!(blend_in, 21);
    assert_eq!(blend_out, 21);
    assert_eq!(keys, 84);
}

fn known_post_effect_group_paths(cache_root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    let root = cache_root.join("libs/posteffectgroups");
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let relative = entry
                .path()
                .strip_prefix(cache_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if post_effect_group_source_path(&relative).is_some() {
                paths.push(relative);
            }
        }
    }
    paths.sort();
    paths
}

#[test]
#[ignore = "requires AZOTH_RELEASE_SOURCE pointing at a local release corpus"]
fn transforms_configured_time_of_day_corpus() {
    let release_source =
        std::env::var("AZOTH_RELEASE_SOURCE").expect("AZOTH_RELEASE_SOURCE must be set");
    let cache_root = std::path::Path::new(&release_source);

    let mut files = 0usize;
    let mut variables = 0usize;
    let mut float_variables = 0usize;
    let mut color_variables = 0usize;
    let mut splines = 0usize;
    let mut keys = 0usize;
    let mut unknown_flag_bits = 0u32;

    for source_path in known_time_of_day_paths(cache_root) {
        let bytes = std::fs::read(cache_root.join(&source_path)).unwrap();
        let output = XmlSourceTransform
            .transform(LegacySourceInput::new(&source_path, &bytes))
            .unwrap();
        let source: TimeOfDayProfileSource =
            ron::de::from_bytes(&output.artifact().unwrap().bytes).unwrap();

        assert_eq!(source.source_path, source_path);
        for variable in &source.variables {
            variables += 1;
            match variable.value {
                TimeOfDayValueSource::Float(_) => float_variables += 1,
                TimeOfDayValueSource::Color(_) => color_variables += 1,
            }
            splines += 1;
            keys += variable.spline.keys.len();
            for key in &variable.spline.keys {
                unknown_flag_bits |= key.flags.unknown_bits;
            }
        }
        files += 1;
    }

    assert_eq!(files, 169);
    assert_eq!(variables, 21564);
    assert_eq!(float_variables, 18361);
    assert_eq!(color_variables, 3203);
    assert_eq!(splines, 21564);
    assert_eq!(keys, 166_765);
    assert_eq!(unknown_flag_bits, 0);
}

fn known_time_of_day_paths(cache_root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_time_of_day_paths(cache_root, cache_root, &mut paths);
    paths.sort();
    paths
}

fn collect_time_of_day_paths(
    cache_root: &std::path::Path,
    current: &std::path::Path,
    paths: &mut Vec<String>,
) {
    for entry in std::fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_time_of_day_paths(cache_root, &path, paths);
        } else {
            let relative = path
                .strip_prefix(cache_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if time_of_day_source_path(&relative).is_some() {
                paths.push(relative);
            }
        }
    }
}
