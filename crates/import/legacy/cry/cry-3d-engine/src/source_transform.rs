//! Legacy `Cry3DEngine` source classification.

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceFormat, SourceSchemaType,
    normalize_source_path, source_schema_type,
};
use thiserror::Error;

use crate::{
    DatAsset, DatKind, ParseError,
    merged_mesh::{
        COMPILED_MERGED_MESHES_BASE_NAME, COMPILED_MERGED_MESHES_LIST, MergedMeshUsedMeshes,
    },
    wavefront_obj::{WavefrontObj, WavefrontObjError, is_wavefront_obj_name},
};

pub const WAVEFRONT_OBJ_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<WavefrontObjSourceFormat>();

#[derive(SourceFormat)]
#[source(schema = "azoth.compat.wavefront.ObjSource", ext = "obj")]
pub struct WavefrontObjSourceFormat;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DatSourceTransform;

impl LegacySourceTransform for DatSourceTransform {
    type Error = DatSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let path = input.source_path.to_string();
        if !is_legacy_dat_source(&path) {
            return Err(DatSourceTransformError::UnsupportedPath { path });
        }

        let asset = DatAsset::parse_path(&path, input.bytes).map_err(|source| {
            DatSourceTransformError::Parse {
                path: path.clone(),
                source,
            }
        })?;

        if is_engine_level_for_slice_editing_template(&path)
            && matches!(
                asset.kind(),
                DatKind::EditorHeightmap | DatKind::EditorVegetationMap
            )
        {
            return Ok(LegacySourceOutput::Excluded {
                reason: format!(
                    "legacy Cry3DEngine {kind:?} data {path} is the parsed engine-owned LevelForSliceEditing seed, not project terrain or vegetation authoring source",
                    kind = asset.kind()
                ),
            });
        }

        Ok(match asset.kind() {
            DatKind::EditorHeightmap => LegacySourceOutput::Unclassified {
                reason: format!(
                    "legacy Cry3DEngine editor heightmap archive {path} is known but not imported as source yet; map it into the typed Terrain source set before emitting editable source"
                ),
            },
            DatKind::EditorVegetationMap => LegacySourceOutput::Unclassified {
                reason: format!(
                    "legacy Cry3DEngine editor vegetation map archive {path} is known but not imported as source yet; map it into typed Vegetation/Terrain source before emitting editable source"
                ),
            },
            DatKind::Stars | DatKind::Terrain | DatKind::Indoor | DatKind::EngineConfig => {
                LegacySourceOutput::Excluded {
                    reason: format!(
                        "legacy Cry3DEngine {kind:?} data {path} is parsed evidence, not editable source; Azoth source must come from typed terrain, vegetation, world, sky, or TOML config inputs",
                        kind = asset.kind()
                    ),
                }
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergedMeshUsedMeshesSourceTransform;

impl LegacySourceTransform for MergedMeshUsedMeshesSourceTransform {
    type Error = MergedMeshUsedMeshesSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let path = input.source_path.to_string();
        if !is_legacy_merged_mesh_used_meshes_source(&path) {
            return Err(MergedMeshUsedMeshesSourceTransformError::UnsupportedPath { path });
        }

        MergedMeshUsedMeshes::parse(input.bytes).map_err(|source| {
            MergedMeshUsedMeshesSourceTransformError::Parse {
                path: path.clone(),
                source,
            }
        })?;

        Ok(LegacySourceOutput::Excluded {
            reason: format!(
                "legacy Cry3DEngine merged-mesh preload/reference list {path} is parsed for references; it is not editable source and native cooks rebuild merged-mesh products from mesh, terrain, and Prefab sources"
            ),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WavefrontObjSourceTransform;

impl LegacySourceTransform for WavefrontObjSourceTransform {
    type Error = WavefrontObjSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let path = input.source_path.to_string();
        if !is_legacy_wavefront_obj_source(&path) {
            return Err(WavefrontObjSourceTransformError::UnsupportedPath { path });
        }

        WavefrontObj::parse(input.bytes).map_err(|source| {
            WavefrontObjSourceTransformError::Parse {
                path: path.clone(),
                source,
            }
        })?;

        Ok(LegacySourceOutput::authoring_source(
            path,
            WAVEFRONT_OBJ_SOURCE_SCHEMA,
            input.bytes.to_vec(),
        ))
    }
}

#[must_use]
pub fn is_legacy_dat_source(source_path: &str) -> bool {
    crate::is_known_dat_name(&normalize_source_path(source_path))
}

#[must_use]
pub fn is_engine_level_for_slice_editing_template(source_path: &str) -> bool {
    normalize_source_path(source_path).starts_with("engineassets/levelforsliceediting/leveldata/")
}

#[must_use]
pub fn is_legacy_merged_mesh_used_meshes_source(source_path: &str) -> bool {
    let path = normalize_source_path(source_path);
    path.ends_with(&format!(
        "{COMPILED_MERGED_MESHES_BASE_NAME}{COMPILED_MERGED_MESHES_LIST}"
    ))
}

#[must_use]
pub fn is_legacy_wavefront_obj_source(source_path: &str) -> bool {
    let path = normalize_source_path(source_path);
    is_wavefront_obj_name(&path) && !path.ends_with(".obj.ron")
}

#[derive(Debug, Error)]
pub enum DatSourceTransformError {
    #[error("unsupported Cry3DEngine .dat path {path}")]
    UnsupportedPath { path: String },
    #[error("parse Cry3DEngine .dat {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ParseError,
    },
}

#[derive(Debug, Error)]
pub enum MergedMeshUsedMeshesSourceTransformError {
    #[error("unsupported Cry3DEngine merged-mesh used-mesh list path {path}")]
    UnsupportedPath { path: String },
    #[error("parse Cry3DEngine merged-mesh used-mesh list {path:?}")]
    Parse {
        path: String,
        #[source]
        source: ParseError,
    },
}

#[derive(Debug, Error)]
pub enum WavefrontObjSourceTransformError {
    #[error("unsupported Wavefront OBJ path {path}")]
    UnsupportedPath { path: String },
    #[error("parse Wavefront OBJ {path:?}")]
    Parse {
        path: String,
        #[source]
        source: WavefrontObjError,
    },
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    use super::*;

    #[test]
    fn routes_only_known_cry_3d_engine_sources() {
        assert!(is_legacy_dat_source("config/config.dat"));
        assert!(is_legacy_dat_source("terrain/terrain.dat"));
        assert!(is_legacy_dat_source("heightmap.dat"));
        assert!(!is_legacy_dat_source("unknown.dat"));

        assert!(is_legacy_merged_mesh_used_meshes_source(
            "terrain/merged_meshes_sectors/mmrm_used_meshes.lst"
        ));
        assert!(!is_legacy_merged_mesh_used_meshes_source("objects/foo.lst"));

        assert!(is_legacy_wavefront_obj_source("terrain/footprint.obj"));
        assert!(!is_legacy_wavefront_obj_source("terrain/footprint.obj.ron"));
    }

    #[test]
    fn excludes_generated_engine_config_dat_without_artifact() {
        let output = DatSourceTransform
            .transform(LegacySourceInput::new("Config/Config.dat", b"abc"))
            .unwrap();

        assert_eq!(output.artifact(), None);
        match output {
            LegacySourceOutput::Excluded { reason } => {
                assert!(reason.contains("config/config.dat"));
                assert!(reason.contains("not editable source"));
            }
            other => panic!("expected excluded engine config dat, got {other:?}"),
        }
    }

    #[test]
    fn blocks_editor_heightmap_dat_until_terrain_mapping_exists() {
        let err = DatSourceTransform
            .transform(LegacySourceInput::new(
                "Heightmap.dat",
                b"not a valid editor archive",
            ))
            .unwrap_err();
        assert!(err.to_string().contains("parse"));
    }

    #[test]
    fn excludes_validated_engine_slice_editing_seed_but_not_project_editor_data() {
        let bytes = editor_vegetation_map_archive();
        let output = DatSourceTransform
            .transform(LegacySourceInput::new(
                "EngineAssets/LevelForSliceEditing/LevelData/VegetationMap.dat",
                &bytes,
            ))
            .unwrap();
        match output {
            LegacySourceOutput::Excluded { reason } => {
                assert!(reason.contains("levelforsliceediting"));
                assert!(reason.contains("engine-owned"));
                assert!(reason.contains("not project terrain or vegetation"));
            }
            other => panic!("expected engine editor template exclusion, got {other:?}"),
        }

        let project = DatSourceTransform
            .transform(LegacySourceInput::new(
                "Levels/Example/LevelData/VegetationMap.dat",
                &bytes,
            ))
            .unwrap();
        assert!(matches!(project, LegacySourceOutput::Unclassified { .. }));
    }

    #[test]
    fn excludes_merged_mesh_used_meshes_list_after_validation() {
        let output = MergedMeshUsedMeshesSourceTransform
            .transform(LegacySourceInput::new(
                "terrain/merged_meshes_sectors/mmrm_used_meshes.lst",
                b"objects/tree.cgf\nobjects/rock.cgf\n",
            ))
            .unwrap();

        assert_eq!(output.artifact(), None);
        match output {
            LegacySourceOutput::Excluded { reason } => {
                assert!(reason.contains("mmrm_used_meshes.lst"));
                assert!(reason.contains("merged-mesh preload/reference list"));
            }
            other => panic!("expected excluded merged mesh list, got {other:?}"),
        }
    }

    #[test]
    fn preserves_valid_wavefront_obj_as_standard_authoring_source() {
        let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\ng footprint\ns 1\nf 1 2 3\n";
        let output = WavefrontObjSourceTransform
            .transform(LegacySourceInput::new("terrain/footprint.obj", bytes))
            .unwrap();

        let artifact = output.artifact().expect("OBJ authoring source");
        assert_eq!(artifact.path, "terrain/footprint.obj");
        assert_eq!(artifact.schema, WAVEFRONT_OBJ_SOURCE_SCHEMA);
        assert_eq!(artifact.bytes, bytes);
    }

    fn editor_vegetation_map_archive() -> Vec<u8> {
        let xml = br#"<VegetationMap Version="3"><Objects/></VegetationMap>"#;
        let mut bytes = Vec::with_capacity(1 + xml.len() + 4);
        bytes.push(xml.len().try_into().unwrap());
        bytes.extend_from_slice(xml);
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes
    }
}
