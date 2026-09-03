use super::*;
use crate::source_transform::material_override::MaterialOverrideMaxTriggerDistanceValueSource;

#[test]
fn transforms_material_override_to_authoring_source() {
    let output = XmlSourceTransform
        .transform(LegacySourceInput::new("Libs/MaterialOverrides/death_dissolve_1.xml", br#"<MaterialParamsOverride HoldLastFrame="true" MaxTriggerDistance="30" IsTransparent="false">
<Material name="Objects/Characters/example/example_mat" exclude="materials/vfx/excluded">
  <SubMaterial name="All">
    <ShaderGenerationParams>
      <Dissolve_FX enabled="true"/>
    </ShaderGenerationParams>
    <TextureMaps>
      <R:BlendlayerG:Dissolve>
        <param name="start" type="string" value="textures/vfx/solids/procedural_noise_03.dds"/>
      </R:BlendlayerG:Dissolve>
    </TextureMaps>
    <ShaderParams>
      <DissolvePercentage>
        <param name="start" type="float" value="0"/>
        <param name="end" type="float" value="1"/>
        <param name="time" type="float" value="3.0"/>
      </DissolvePercentage>
    </ShaderParams>
  </SubMaterial>
</Material>
</MaterialParamsOverride>"#))
        .unwrap();

    let artifact = output.artifact().expect("authoring artifact");
    assert_eq!(
        artifact.path,
        "materials/effects/overrides/death_dissolve_1.materialoverride.ron"
    );
    assert_eq!(artifact.schema, source_schemas::MATERIAL_OVERRIDE);

    let source: MaterialOverrideSource = ron::de::from_bytes(&artifact.bytes).unwrap();
    assert_eq!(
        source.source_path,
        "libs/materialoverrides/death_dissolve_1.xml"
    );
    assert_eq!(source.hold_last_frame, Some(true));
    assert_eq!(
        source.max_trigger_distance,
        Some(MaterialOverrideMaxTriggerDistanceSource::Distance(
            MaterialOverrideMaxTriggerDistanceValueSource { value: 30.0 }
        ))
    );
    assert_eq!(source.is_transparent, Some(false));
    assert_eq!(source.materials.len(), 1);
    assert_eq!(source.sub_materials.len(), 0);

    let material = &source.materials[0];
    assert_eq!(material.name, "Objects/Characters/example/example_mat");
    assert_eq!(material.exclude.as_deref(), Some("materials/vfx/excluded"));
    assert_eq!(material.sub_materials.len(), 1);

    let sub_material = &material.sub_materials[0];
    assert_eq!(sub_material.name, "All");
    assert_eq!(sub_material.nodes.len(), 3);
    assert_eq!(sub_material.nodes[0].name, "ShaderGenerationParams");
    assert_eq!(sub_material.nodes[0].children[0].name, "Dissolve_FX");
    assert_eq!(
        sub_material.nodes[0].children[0].attributes[0].name,
        "enabled"
    );
    assert_eq!(
        sub_material.nodes[0].children[0].attributes[0].value,
        "true"
    );

    let texture_param = &sub_material.nodes[1].children[0].params[0];
    assert_eq!(texture_param.name, "start");
    assert_eq!(texture_param.value_type, "string");
    assert_eq!(
        texture_param.value,
        "textures/vfx/solids/procedural_noise_03.dds"
    );

    let dissolve = &sub_material.nodes[2].children[0];
    assert_eq!(dissolve.name, "DissolvePercentage");
    assert_eq!(dissolve.params.len(), 3);
    assert_eq!(dissolve.params[1].name, "end");
    assert_eq!(dissolve.params[1].value_type, "float");
    assert_eq!(dissolve.params[1].value, "1");
}

#[test]
#[ignore = "requires AZOTH_RELEASE_SOURCE pointing at a local release corpus"]
fn transforms_configured_material_override_corpus() {
    let release_source =
        std::env::var("AZOTH_RELEASE_SOURCE").expect("AZOTH_RELEASE_SOURCE must be set");
    let cache_root = std::path::Path::new(&release_source);

    let mut counts = MaterialOverrideCorpusCounts::default();

    for source_path in known_material_override_paths(cache_root) {
        let bytes = std::fs::read(cache_root.join(&source_path)).unwrap();
        let output = XmlSourceTransform
            .transform(LegacySourceInput::new(&source_path, &bytes))
            .unwrap_or_else(|error| panic!("{source_path}: {error}"));
        let artifact = output.artifact().unwrap();
        assert_eq!(
            artifact.path,
            material_override_source_path(&source_path).unwrap()
        );
        assert_eq!(artifact.schema, source_schemas::MATERIAL_OVERRIDE);

        let source: MaterialOverrideSource = ron::de::from_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, source_path);

        counts.files += 1;
        if source.hold_last_frame.is_some() {
            counts.root_hold_last_frame += 1;
        }
        match &source.max_trigger_distance {
            Some(MaterialOverrideMaxTriggerDistanceSource::Distance(_)) => {
                counts.root_max_trigger_distance += 1;
            }
            Some(MaterialOverrideMaxTriggerDistanceSource::Preset(preset)) => {
                counts.root_max_trigger_distance_presets += 1;
                *counts
                    .root_max_trigger_distance_preset_names
                    .entry(preset.name.clone())
                    .or_default() += 1;
            }
            None => {}
        }
        if source.is_transparent.is_some() {
            counts.root_is_transparent += 1;
        }
        counts.comments += source.comments.len();

        counts.materials += source.materials.len();
        for material in &source.materials {
            if material.exclude.is_some() {
                counts.material_excludes += 1;
            }
            counts.comments += material.comments.len();
            count_material_override_nodes(&material.nodes, &mut counts);
            count_material_override_sub_materials(&material.sub_materials, &mut counts);
        }
        count_material_override_sub_materials(&source.sub_materials, &mut counts);
    }

    assert_eq!(counts.files, 261);
    assert_eq!(counts.materials, 89);
    assert_eq!(counts.sub_materials, 254);
    assert_eq!(counts.nodes, 1919);
    assert_eq!(counts.node_attributes, 210);
    assert_eq!(counts.params, 1871);
    assert_eq!(counts.comments, 241);
    assert_eq!(counts.root_hold_last_frame, 40);
    assert_eq!(counts.root_max_trigger_distance, 0);
    assert_eq!(counts.root_max_trigger_distance_presets, 24);
    assert_eq!(
        counts.root_max_trigger_distance_preset_names,
        std::collections::BTreeMap::from([("close".to_string(), 24)])
    );
    assert_eq!(counts.root_is_transparent, 1);
    assert_eq!(counts.material_excludes, 44);
    assert_eq!(counts.string_texture_refs, 188);
    assert_eq!(
        counts.param_types,
        std::collections::BTreeMap::from([
            ("bool".to_string(), 12),
            ("color".to_string(), 245),
            ("float".to_string(), 1388),
            ("string".to_string(), 226),
        ])
    );
    assert_eq!(
        counts.param_names,
        std::collections::BTreeMap::from([
            ("delay".to_string(), 19),
            ("easing".to_string(), 37),
            ("end".to_string(), 382),
            ("loop".to_string(), 39),
            ("start".to_string(), 1012),
            ("time".to_string(), 382),
        ])
    );
}

#[derive(Default)]
struct MaterialOverrideCorpusCounts {
    files: usize,
    materials: usize,
    sub_materials: usize,
    nodes: usize,
    node_attributes: usize,
    params: usize,
    comments: usize,
    root_hold_last_frame: usize,
    root_max_trigger_distance: usize,
    root_max_trigger_distance_presets: usize,
    root_is_transparent: usize,
    material_excludes: usize,
    string_texture_refs: usize,
    root_max_trigger_distance_preset_names: std::collections::BTreeMap<String, usize>,
    param_types: std::collections::BTreeMap<String, usize>,
    param_names: std::collections::BTreeMap<String, usize>,
}

fn count_material_override_sub_materials(
    sub_materials: &[MaterialOverrideSubMaterialSource],
    counts: &mut MaterialOverrideCorpusCounts,
) {
    counts.sub_materials += sub_materials.len();
    for sub_material in sub_materials {
        counts.comments += sub_material.comments.len();
        count_material_override_nodes(&sub_material.nodes, counts);
        count_material_override_sub_materials(&sub_material.sub_materials, counts);
    }
}

fn count_material_override_nodes(
    nodes: &[MaterialOverrideNodeSource],
    counts: &mut MaterialOverrideCorpusCounts,
) {
    for node in nodes {
        counts.nodes += 1;
        counts.node_attributes += node.attributes.len();
        counts.comments += node.comments.len();
        counts.params += node.params.len();
        for param in &node.params {
            *counts.param_names.entry(param.name.clone()).or_default() += 1;
            *counts
                .param_types
                .entry(param.value_type.clone())
                .or_default() += 1;
            let value = param.value.to_ascii_lowercase();
            if param.value_type == "string"
                && (crate::has_extension(&value, "dds") || crate::has_extension(&value, "tif"))
            {
                counts.string_texture_refs += 1;
            }
        }
        count_material_override_nodes(&node.children, counts);
    }
}

fn known_material_override_paths(cache_root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_material_override_paths(
        cache_root,
        &cache_root.join("libs/materialoverrides"),
        &mut paths,
    );
    paths.sort();
    paths
}

fn collect_material_override_paths(
    cache_root: &std::path::Path,
    current: &std::path::Path,
    paths: &mut Vec<String>,
) {
    for entry in std::fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_material_override_paths(cache_root, &path, paths);
        } else {
            let relative = path
                .strip_prefix(cache_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if material_override_source_path(&relative).is_some() {
                paths.push(relative);
            }
        }
    }
}
