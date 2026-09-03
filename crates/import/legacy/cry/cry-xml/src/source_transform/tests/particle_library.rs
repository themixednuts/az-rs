use az_asset_builder::{LegacySourceInput, LegacySourceTransform};

use super::*;

#[test]
fn transforms_particle_library_to_authoring_source() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new("Libs/Particles/cfx_ai_recolors.xml", br#"<ParticleLibrary Name="cfx_AI_Recolors" SandboxVersion="0.0.0.0" ParticleVersion="33">
<Folder Name="Magic"><Params Enabled="1"/></Folder>
<Particles Name="Laserbeam_beam">
  <Params EmitterShape="BEAM" Texture="textures/vfx/beam.dds" Material="materials/vfx/beam.mtl"/>
  <DynamicParams GlobalAlphaStrength="1,Random=0,EmitterStrength=,ParticleAge=(t=0,v=1,d=0,s=0,flags=0)"/>
  <DynamicParamsInterpolateOverride GlobalAlphaStrength_interpA="0" GlobalAlphaStrength_interpB="1"/>
  <Childs>
    <Particles Name="arcaneBlack">
      <Params Count="8" Geometry="objects/vfx/arcane.cgf"/>
    </Particles>
  </Childs>
  <LODs>
    <LevelOfDetail>
      <LodParticle Distance="20" Active="0">
        <Particle Name="Laserbeam_beamLod20">
          <Params Texture="textures/vfx/beam_lod.dds"/>
        </Particle>
      </LodParticle>
    </LevelOfDetail>
  </LODs>
</Particles>
</ParticleLibrary>"#))
        .unwrap();

    let artifact = output.artifact().expect("authoring source artifact");
    assert_eq!(artifact.path, "particles/cfx_ai_recolors.particle.ron");
    assert_eq!(artifact.schema, source_schemas::PARTICLE_LIBRARY);

    let source: ParticleLibrarySource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(source.source_path, "libs/particles/cfx_ai_recolors.xml");
    assert_eq!(source.name, "cfx_AI_Recolors");
    assert_eq!(source.sandbox_version.as_deref(), Some("0.0.0.0"));
    assert_eq!(source.particle_version.as_deref(), Some("33"));
    assert_eq!(source.folders.len(), 1);
    assert_eq!(source.folders[0].name, "Magic");
    assert_eq!(source.folders[0].params.entries[0].name, "Enabled");
    assert_eq!(source.folders[0].params.entries[0].value, "1");

    let effect = &source.effects[0];
    assert_eq!(effect.name, "Laserbeam_beam");
    assert_eq!(effect.params.entries[0].name, "EmitterShape");
    assert_eq!(effect.params.entries[0].value, "BEAM");
    assert_eq!(effect.params.entries[1].name, "Texture");
    assert_eq!(effect.params.entries[1].value, "textures/vfx/beam.dds");
    assert_eq!(effect.dynamic_params.entries[0].name, "GlobalAlphaStrength");
    assert_eq!(
        effect.dynamic_param_interpolation.entries[0].name,
        "GlobalAlphaStrength_interpA"
    );
    assert_eq!(effect.children[0].name, "arcaneBlack");
    assert_eq!(effect.children[0].params.entries[0].name, "Count");
    assert_eq!(effect.children[0].params.entries[1].name, "Geometry");

    let lod_particle = &effect.lods[0].levels[0].particles[0];
    assert_eq!(lod_particle.distance.as_deref(), Some("20"));
    assert_eq!(lod_particle.active.as_deref(), Some("0"));
    assert_eq!(lod_particle.effect.name, "Laserbeam_beamLod20");
    assert_eq!(lod_particle.effect.params.entries[0].name, "Texture");
}

#[test]
fn particle_library_source_paths_only_claim_particle_xml() {
    assert_eq!(
        particle_library_source_path("libs/particles/cfx_ai_recolors.xml").as_deref(),
        Some("particles/cfx_ai_recolors.particle.ron")
    );
    assert_eq!(
        xml_source_path("libs/particles/cfx_ai_recolors.xml").as_deref(),
        Some("particles/cfx_ai_recolors.particle.ron")
    );
    assert!(is_legacy_xml_source("libs/particles/cfx_ai_recolors.xml"));
    assert!(!is_legacy_xml_source("libs/particles/shared/noise.dds"));
    assert!(!is_legacy_xml_source("libs/particles/shared/material.mtl"));
    assert!(particle_library_source_path("libs/posteffectgroups/default.xml").is_none());
}

#[test]
#[ignore = "requires AZOTH_RELEASE_SOURCE pointing at a local release corpus"]
fn transforms_configured_particle_library_corpus() {
    let release_source =
        std::env::var("AZOTH_RELEASE_SOURCE").expect("AZOTH_RELEASE_SOURCE must be set");
    let cache_root = std::path::Path::new(&release_source);

    let mut stats = ParticleLibraryCorpusStats::default();
    let mut tracked_param_keys = std::collections::BTreeMap::<String, usize>::new();

    for source_path in known_particle_library_paths(cache_root) {
        let bytes = std::fs::read(cache_root.join(&source_path)).unwrap();
        let output = XmlSourceTransform
            .transform(LegacySourceInput::new(&source_path, &bytes))
            .unwrap_or_else(|error| panic!("{source_path}: {error}"));
        let artifact = output.artifact().expect("authoring source artifact");
        assert_eq!(artifact.schema, source_schemas::PARTICLE_LIBRARY);
        let source: ParticleLibrarySource = ron::de::from_bytes(&artifact.bytes).unwrap();

        assert_eq!(source.source_path, source_path);
        stats.files += 1;
        count_particle_library_source(&source, &mut stats, &mut tracked_param_keys);
    }

    assert_eq!(stats.files, 640);
    assert_eq!(stats.settings, 105);
    assert_eq!(stats.folders, 959);
    assert_eq!(stats.root_effects, 8330);
    assert_eq!(stats.child_effects, 38692);
    assert_eq!(stats.lod_effects, 37003);
    assert_eq!(stats.total_effects, 84025);
    assert_eq!(stats.lod_groups, 19662);
    assert_eq!(stats.lod_levels, 37003);
    assert_eq!(stats.lod_particles, 37003);
    assert_eq!(stats.params, 84950);
    assert_eq!(stats.dynamic_params, 4859);
    assert_eq!(stats.dynamic_param_interpolation, 4822);
    assert_eq!(stats.param_entries, 2_087_316);
    assert_eq!(stats.dynamic_param_entries, 13697);
    assert_eq!(stats.dynamic_param_interpolation_entries, 27288);
    assert_eq!(stats.extra_nodes, 0);
    assert_eq!(stats.comments, 0);
    assert_eq!(
        tracked_param_keys,
        [
            ("Geometry".to_string(), 11792),
            ("GeometryPieces".to_string(), 1372),
            ("Material".to_string(), 28637),
            ("NormalMap".to_string(), 399),
            ("Texture".to_string(), 76746),
            ("TextureTiling".to_string(), 43299),
        ]
        .into_iter()
        .collect()
    );
}

#[derive(Debug, Default)]
struct ParticleLibraryCorpusStats {
    files: usize,
    settings: usize,
    folders: usize,
    root_effects: usize,
    child_effects: usize,
    lod_effects: usize,
    total_effects: usize,
    lod_groups: usize,
    lod_levels: usize,
    lod_particles: usize,
    params: usize,
    dynamic_params: usize,
    dynamic_param_interpolation: usize,
    param_entries: usize,
    dynamic_param_entries: usize,
    dynamic_param_interpolation_entries: usize,
    extra_nodes: usize,
    comments: usize,
}

fn count_particle_library_source(
    source: &ParticleLibrarySource,
    stats: &mut ParticleLibraryCorpusStats,
    tracked_param_keys: &mut std::collections::BTreeMap<String, usize>,
) {
    stats.settings += source.settings.len();
    stats.folders += source.folders.len();
    stats.comments += source.comments.len();
    stats.extra_nodes += count_extra_nodes(&source.extra_nodes);
    count_param_bag(
        &source.params,
        &mut stats.params,
        &mut stats.param_entries,
        tracked_param_keys,
    );
    count_param_bag(
        &source.dynamic_params,
        &mut stats.dynamic_params,
        &mut stats.dynamic_param_entries,
        tracked_param_keys,
    );
    count_param_bag(
        &source.dynamic_param_interpolation,
        &mut stats.dynamic_param_interpolation,
        &mut stats.dynamic_param_interpolation_entries,
        tracked_param_keys,
    );

    for settings in &source.settings {
        stats.comments += settings.comments.len();
        stats.extra_nodes += count_extra_nodes(&settings.extra_nodes);
        count_param_bag(
            &settings.params,
            &mut stats.params,
            &mut stats.param_entries,
            tracked_param_keys,
        );
        count_param_bag(
            &settings.dynamic_params,
            &mut stats.dynamic_params,
            &mut stats.dynamic_param_entries,
            tracked_param_keys,
        );
        count_param_bag(
            &settings.dynamic_param_interpolation,
            &mut stats.dynamic_param_interpolation,
            &mut stats.dynamic_param_interpolation_entries,
            tracked_param_keys,
        );
    }
    for folder in &source.folders {
        stats.comments += folder.comments.len();
        stats.extra_nodes += count_extra_nodes(&folder.extra_nodes);
        count_param_bag(
            &folder.params,
            &mut stats.params,
            &mut stats.param_entries,
            tracked_param_keys,
        );
        count_param_bag(
            &folder.dynamic_params,
            &mut stats.dynamic_params,
            &mut stats.dynamic_param_entries,
            tracked_param_keys,
        );
        count_param_bag(
            &folder.dynamic_param_interpolation,
            &mut stats.dynamic_param_interpolation,
            &mut stats.dynamic_param_interpolation_entries,
            tracked_param_keys,
        );
        for child in &folder.children {
            count_particle_effect(child, ParticleEffectKind::Child, stats, tracked_param_keys);
        }
    }
    for effect in &source.effects {
        stats.root_effects += 1;
        count_particle_effect(effect, ParticleEffectKind::Root, stats, tracked_param_keys);
    }
}

fn count_particle_effect(
    effect: &ParticleEffectSource,
    kind: ParticleEffectKind,
    stats: &mut ParticleLibraryCorpusStats,
    tracked_param_keys: &mut std::collections::BTreeMap<String, usize>,
) {
    match kind {
        ParticleEffectKind::Root => {}
        ParticleEffectKind::Child => stats.child_effects += 1,
        ParticleEffectKind::Lod => stats.lod_effects += 1,
    }
    stats.total_effects += 1;
    stats.settings += effect.settings.len();
    stats.comments += effect.comments.len();
    stats.extra_nodes += count_extra_nodes(&effect.extra_nodes);
    count_param_bag(
        &effect.params,
        &mut stats.params,
        &mut stats.param_entries,
        tracked_param_keys,
    );
    count_param_bag(
        &effect.dynamic_params,
        &mut stats.dynamic_params,
        &mut stats.dynamic_param_entries,
        tracked_param_keys,
    );
    count_param_bag(
        &effect.dynamic_param_interpolation,
        &mut stats.dynamic_param_interpolation,
        &mut stats.dynamic_param_interpolation_entries,
        tracked_param_keys,
    );

    for settings in &effect.settings {
        stats.comments += settings.comments.len();
        stats.extra_nodes += count_extra_nodes(&settings.extra_nodes);
        count_param_bag(
            &settings.params,
            &mut stats.params,
            &mut stats.param_entries,
            tracked_param_keys,
        );
        count_param_bag(
            &settings.dynamic_params,
            &mut stats.dynamic_params,
            &mut stats.dynamic_param_entries,
            tracked_param_keys,
        );
        count_param_bag(
            &settings.dynamic_param_interpolation,
            &mut stats.dynamic_param_interpolation,
            &mut stats.dynamic_param_interpolation_entries,
            tracked_param_keys,
        );
    }
    for child in &effect.children {
        count_particle_effect(child, ParticleEffectKind::Child, stats, tracked_param_keys);
    }
    for lods in &effect.lods {
        stats.lod_groups += 1;
        stats.lod_levels += lods.levels.len();
        stats.comments += lods.comments.len();
        stats.extra_nodes += count_extra_nodes(&lods.extra_nodes);
        for level in &lods.levels {
            stats.comments += level.comments.len();
            stats.extra_nodes += count_extra_nodes(&level.extra_nodes);
            for particle in &level.particles {
                stats.lod_particles += 1;
                stats.comments += particle.comments.len();
                stats.extra_nodes += count_extra_nodes(&particle.extra_nodes);
                count_particle_effect(
                    &particle.effect,
                    ParticleEffectKind::Lod,
                    stats,
                    tracked_param_keys,
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ParticleEffectKind {
    Root,
    Child,
    Lod,
}

fn count_param_bag(
    bag: &ParticleParamBagSource,
    bags: &mut usize,
    entries: &mut usize,
    tracked_param_keys: &mut std::collections::BTreeMap<String, usize>,
) {
    if bag.entries.is_empty() {
        return;
    }

    *bags += 1;
    *entries += bag.entries.len();
    for entry in &bag.entries {
        if is_tracked_param_name(&entry.name) && !entry.value.trim().is_empty() {
            *tracked_param_keys.entry(entry.name.clone()).or_default() += 1;
        }
    }
}

fn is_tracked_param_name(name: &str) -> bool {
    matches!(
        name,
        "Texture"
            | "TextureTiling"
            | "Material"
            | "Geometry"
            | "GeometryPieces"
            | "NormalMap"
            | "EnvironmentProbe"
            | "AudioTrigger"
            | "AudioRtpc"
            | "AudioSwitch"
            | "Sound"
    )
}

fn count_extra_nodes(nodes: &[ParticleExtraNodeSource]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_extra_nodes(&node.children))
        .sum()
}

fn known_particle_library_paths(cache_root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_particle_library_paths(cache_root, &cache_root.join("libs/particles"), &mut paths);
    paths.sort();
    paths
}

fn collect_particle_library_paths(
    cache_root: &std::path::Path,
    current: &std::path::Path,
    paths: &mut Vec<String>,
) {
    for entry in std::fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_particle_library_paths(cache_root, &path, paths);
        } else {
            let relative = path
                .strip_prefix(cache_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if particle_library_source_path(&relative).is_some() {
                paths.push(relative);
            }
        }
    }
}
