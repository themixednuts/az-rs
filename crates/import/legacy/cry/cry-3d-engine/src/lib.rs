//! `Cry3DEngine` asset parsing.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/Cry3DEngine` and related
//! `CryCommon` headers.

pub mod editor_archive;
pub mod merged_mesh;
pub mod object_tree;
pub mod source_transform;
pub mod stars;
pub mod terrain;
pub mod vis_area;
pub mod wavefront_obj;

mod read;

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use az_asset_builder::normalize_source_path;
use thiserror::Error;

pub use read::Endian;
pub use source_transform::{
    DatSourceTransform, DatSourceTransformError, MergedMeshUsedMeshesSourceTransform,
    MergedMeshUsedMeshesSourceTransformError, WavefrontObjSourceTransform,
    WavefrontObjSourceTransformError, is_legacy_dat_source,
    is_legacy_merged_mesh_used_meshes_source, is_legacy_wavefront_obj_source,
};

pub const DAT_EXTENSION: &str = "dat";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DatInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse dat asset {path:?}: {source}")]
    Parse { path: PathBuf, source: ParseError },
}

/// Path-selected `.dat` asset family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DatKind {
    Stars,
    Terrain,
    Indoor,
    EditorHeightmap,
    EditorVegetationMap,
    #[default]
    EngineConfig,
}

impl DatKind {
    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Option<Self> {
        let path = normalize_source_path(path.as_ref().to_string_lossy());
        if path.ends_with("engineassets/sky/stars.dat") {
            Some(Self::Stars)
        } else if path.ends_with("terrain/terrain.dat") {
            Some(Self::Terrain)
        } else if path.ends_with("terrain/indoor.dat") {
            Some(Self::Indoor)
        } else if path.ends_with("heightmap.dat") {
            Some(Self::EditorHeightmap)
        } else if path.ends_with("vegetationmap.dat") {
            Some(Self::EditorVegetationMap)
        } else if path.ends_with("config/config.dat") {
            Some(Self::EngineConfig)
        } else {
            None
        }
    }
}

/// Parsed `.dat` asset.
#[derive(Debug, Clone)]
pub enum DatAsset<'a> {
    Stars(stars::StarsDat<'a>),
    Terrain(terrain::CompiledTerrain<'a>),
    Indoor(vis_area::VisAreaManager<'a>),
    EditorHeightmap(editor_archive::EditorHeightmap<'a>),
    EditorVegetationMap(editor_archive::EditorVegetationMap<'a>),
    EngineConfig(EngineConfigDat<'a>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DatSummary {
    pub kind: DatKind,
    pub terrain_nodes: usize,
    pub terrain_height_nodes: usize,
    pub object_tree_nodes: usize,
    pub object_tree_objects: usize,
    pub object_block_bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DatTotals {
    pub files: usize,
    pub stars: usize,
    pub terrain: usize,
    pub indoor: usize,
    pub editor_heightmaps: usize,
    pub editor_vegetation_maps: usize,
    pub engine_configs: usize,
    pub terrain_nodes: usize,
    pub terrain_height_nodes: usize,
    pub object_tree_nodes: usize,
    pub object_tree_objects: usize,
    pub object_block_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatFileSummary {
    pub source: String,
    pub description: String,
    pub summary: DatSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DatInspection {
    pub rows: Vec<DatFileSummary>,
    pub totals: DatTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct DatInspectionReport<'a> {
    inspection: &'a DatInspection,
    limit: usize,
}

impl<'a> DatAsset<'a> {
    /// Parse a `.dat` payload using the asset path to select its family.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is not a known `.dat` family or when
    /// the selected parser rejects the payload.
    pub fn parse_path(path: impl AsRef<Path>, bytes: &'a [u8]) -> Result<Self, ParseError> {
        match DatKind::from_path(path).ok_or(ParseError::UnknownDatPath)? {
            DatKind::Stars => stars::StarsDat::parse(bytes).map(Self::Stars),
            DatKind::Terrain => terrain::CompiledTerrain::parse(bytes).map(Self::Terrain),
            DatKind::Indoor => vis_area::VisAreaManager::parse(bytes).map(Self::Indoor),
            DatKind::EditorHeightmap => {
                editor_archive::EditorHeightmap::parse(bytes).map(Self::EditorHeightmap)
            }
            DatKind::EditorVegetationMap => {
                editor_archive::EditorVegetationMap::parse(bytes).map(Self::EditorVegetationMap)
            }
            DatKind::EngineConfig => Ok(Self::EngineConfig(EngineConfigDat::parse(bytes))),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> DatKind {
        match self {
            Self::Stars(_) => DatKind::Stars,
            Self::Terrain(_) => DatKind::Terrain,
            Self::Indoor(_) => DatKind::Indoor,
            Self::EditorHeightmap(_) => DatKind::EditorHeightmap,
            Self::EditorVegetationMap(_) => DatKind::EditorVegetationMap,
            Self::EngineConfig(_) => DatKind::EngineConfig,
        }
    }

    #[must_use]
    pub fn summary(&self) -> DatSummary {
        let mut summary = DatSummary {
            kind: self.kind(),
            ..DatSummary::default()
        };
        match self {
            Self::Stars(_) | Self::EditorVegetationMap(_) | Self::EngineConfig(_) => {}
            Self::Terrain(terrain) => {
                add_terrain_summary(&mut summary, terrain);
            }
            Self::Indoor(indoor) => {
                for area in indoor
                    .areas()
                    .iter()
                    .chain(indoor.portals())
                    .chain(indoor.occlusion_areas())
                {
                    if let Some(tree) = area.object_tree() {
                        summary.object_tree_nodes += tree.node_count();
                        summary.object_tree_objects += tree.object_count();
                        summary.object_block_bytes += tree.object_bytes();
                    }
                }
            }
            Self::EditorHeightmap(heightmap) => {
                add_terrain_summary(&mut summary, heightmap.terrain());
            }
        }
        summary
    }
}

impl fmt::Display for DatAsset<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stars(stars) => {
                write!(f, "stars.dat, {} stars", stars.len())
            }
            Self::Terrain(terrain) => {
                let header = terrain.header();
                let object_nodes = terrain
                    .object_tree()
                    .map_or(0, object_tree::ObjectTree::node_count);
                write!(
                    f,
                    "terrain.dat, {}m terrain, {} nodes ({} height), {} materials, {} object-tree nodes",
                    header
                        .terrain_info
                        .terrain_size_meters()
                        .unwrap_or_default(),
                    terrain.nodes().len(),
                    terrain.height_node_count(),
                    terrain.material_names().len(),
                    object_nodes
                )
            }
            Self::Indoor(indoor) => {
                let header = indoor.header();
                write!(
                    f,
                    "indoor.dat, {} vis areas, {} portals, {} occlusion areas",
                    header.vis_area_count, header.portal_count, header.occlusion_area_count
                )
            }
            Self::EditorHeightmap(heightmap) => {
                let attrs = heightmap.attributes();
                write!(
                    f,
                    "heightmap.dat, {}x{}, {} named blocks, {} terrain nodes",
                    attrs.width,
                    attrs.height,
                    heightmap.archive().named_blocks().len(),
                    heightmap.terrain().nodes().len()
                )
            }
            Self::EditorVegetationMap(vegetation) => {
                write!(
                    f,
                    "vegetationmap.dat, version {}, {} named blocks",
                    vegetation.version(),
                    vegetation.archive().named_blocks().len()
                )
            }
            Self::EngineConfig(config) => {
                write!(f, "config.dat, {} bytes", config.bytes().len())
            }
        }
    }
}

impl DatTotals {
    pub const fn add_summary(&mut self, summary: DatSummary) {
        self.files += 1;
        match summary.kind {
            DatKind::Stars => self.stars += 1,
            DatKind::Terrain => self.terrain += 1,
            DatKind::Indoor => self.indoor += 1,
            DatKind::EditorHeightmap => self.editor_heightmaps += 1,
            DatKind::EditorVegetationMap => self.editor_vegetation_maps += 1,
            DatKind::EngineConfig => self.engine_configs += 1,
        }
        self.terrain_nodes += summary.terrain_nodes;
        self.terrain_height_nodes += summary.terrain_height_nodes;
        self.object_tree_nodes += summary.object_tree_nodes;
        self.object_tree_objects += summary.object_tree_objects;
        self.object_block_bytes += summary.object_block_bytes;
    }
}

impl DatInspection {
    pub fn add_file_summary(&mut self, row: DatFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> DatInspectionReport<'_> {
        DatInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for DatTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  stars: {}", self.stars)?;
        writeln!(f, "  terrain: {}", self.terrain)?;
        writeln!(f, "  indoor: {}", self.indoor)?;
        writeln!(f, "  editor heightmaps: {}", self.editor_heightmaps)?;
        writeln!(
            f,
            "  editor vegetation maps: {}",
            self.editor_vegetation_maps
        )?;
        writeln!(f, "  engine config blobs: {}", self.engine_configs)?;
        writeln!(f, "  terrain nodes: {}", self.terrain_nodes)?;
        writeln!(f, "  terrain height nodes: {}", self.terrain_height_nodes)?;
        writeln!(f, "  object-tree nodes: {}", self.object_tree_nodes)?;
        writeln!(f, "  object-tree objects: {}", self.object_tree_objects)?;
        write!(f, "  object block bytes: {}", self.object_block_bytes)
    }
}

impl fmt::Display for DatInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.description)?;
            }

            if self.inspection.rows.len() > self.limit {
                writeln!(
                    f,
                    "... {} more files",
                    self.inspection.rows.len() - self.limit
                )?;
            }
        }

        write!(f, "{}", self.inspection.totals)
    }
}

/// Parse `bytes` and reduce the asset to its counted [`DatSummary`].
///
/// # Errors
///
/// Returns any error [`DatAsset::parse_path`] returns.
pub fn summarize_dat_asset(path: impl AsRef<Path>, bytes: &[u8]) -> Result<DatSummary, ParseError> {
    DatAsset::parse_path(path, bytes).map(|asset| asset.summary())
}

/// Parse `bytes` into a one-row inspection record naming `path`.
///
/// # Errors
///
/// Returns any error [`DatAsset::parse_path`] returns.
pub fn inspect_dat_asset(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<DatFileSummary, ParseError> {
    let path = path.as_ref();
    let asset = DatAsset::parse_path(path, bytes)?;
    Ok(DatFileSummary {
        source: path.display().to_string(),
        description: asset.to_string(),
        summary: asset.summary(),
    })
}

/// Read `path` from disk and inspect it.
///
/// # Errors
///
/// Returns [`DatInspectionError::Read`] when `path` cannot be read, or
/// [`DatInspectionError::Parse`] when [`inspect_dat_asset`] rejects its bytes.
pub fn inspect_dat_path(path: impl AsRef<Path>) -> Result<DatFileSummary, DatInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| DatInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_dat_asset(path, &bytes).map_err(|source| DatInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Inspect every path in `paths`, accumulating per-family totals.
///
/// # Errors
///
/// Returns the first error [`inspect_dat_path`] returns; remaining paths are
/// not visited.
pub fn inspect_dat_files<I, P>(paths: I) -> Result<DatInspection, DatInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = DatInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_dat_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_dat_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(DAT_EXTENSION)
}

#[must_use]
pub fn is_known_dat_name(path: &str) -> bool {
    DatKind::from_path(path).is_some()
}

#[must_use]
pub fn is_known_dat_path(path: &Path) -> bool {
    DatKind::from_path(path).is_some()
}

fn add_terrain_summary(summary: &mut DatSummary, terrain: &terrain::CompiledTerrain<'_>) {
    summary.terrain_nodes += terrain.nodes().len();
    summary.terrain_height_nodes += terrain.height_node_count();
    if let Some(tree) = terrain.object_tree() {
        summary.object_tree_nodes += tree.node_count();
        summary.object_tree_objects += tree.object_count();
        summary.object_block_bytes += tree.object_bytes();
    }
}

/// `config/config.dat` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfigDat<'a> {
    bytes: &'a [u8],
}

impl<'a> EngineConfigDat<'a> {
    #[inline]
    #[must_use]
    pub const fn parse(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    #[inline]
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("unexpected end of file at {offset}: needed {needed} bytes, had {actual}")]
    UnexpectedEof {
        offset: usize,
        needed: usize,
        actual: usize,
    },
    #[error("integer overflow while parsing")]
    IntegerOverflow,
    #[error("unknown .dat asset path")]
    UnknownDatPath,
    #[error("invalid magic for {asset}: expected {expected:?}, found {found:?}")]
    InvalidMagic {
        asset: &'static str,
        expected: &'static [u8],
        found: Vec<u8>,
    },
    #[error("unsupported {asset} version {found}, expected {expected}")]
    UnsupportedVersion {
        asset: &'static str,
        /// Widened to `i64` so unsigned 32-bit version tags round-trip without
        /// wrapping into a negative display value.
        expected: i64,
        found: i64,
    },
    #[error("invalid count {count} for {field}")]
    InvalidCount { field: &'static str, count: i32 },
    #[error("invalid size {size} for {field}")]
    InvalidSize { field: &'static str, size: i32 },
    #[error("unsupported render node type {value} at object block offset {offset}")]
    UnsupportedRenderNodeType { offset: usize, value: i32 },
    #[error("chunk size mismatch: header says {declared}, file has {actual}")]
    ChunkSizeMismatch { declared: usize, actual: usize },
    #[error("invalid UTF-8 in {field}")]
    Utf8 {
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("invalid UTF-16 in {field}")]
    Utf16 {
        field: &'static str,
        source: std::string::FromUtf16Error,
    },
    #[error("XML parse error in {field}: {source}")]
    Xml {
        field: &'static str,
        source: quick_xml::Error,
    },
    #[error("invalid XML attribute in {field}: {name}")]
    XmlAttribute {
        field: &'static str,
        name: &'static str,
    },
    #[error("missing named data block {name}")]
    MissingNamedBlock { name: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_engine_config_dat_and_paths() {
        let asset = DatAsset::parse_path("config/config.dat", b"abc").expect("parse config dat");
        let summary = asset.summary();
        let mut totals = DatTotals::default();
        totals.add_summary(summary);

        assert_eq!(summary.kind, DatKind::EngineConfig);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.engine_configs, 1);
        assert_eq!(asset.to_string(), "config.dat, 3 bytes");
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  stars: 0\n  terrain: 0\n  indoor: 0\n  editor heightmaps: 0\n  editor vegetation maps: 0\n  engine config blobs: 1\n  terrain nodes: 0\n  terrain height nodes: 0\n  object-tree nodes: 0\n  object-tree objects: 0\n  object block bytes: 0"
        );

        let mut inspection = DatInspection::default();
        inspection.add_file_summary(
            inspect_dat_asset("config/config.dat", b"abc").expect("inspect config dat"),
        );
        assert_eq!(
            inspection.report(20).to_string(),
            "config/config.dat: config.dat, 3 bytes\n  files: 1\n  stars: 0\n  terrain: 0\n  indoor: 0\n  editor heightmaps: 0\n  editor vegetation maps: 0\n  engine config blobs: 1\n  terrain nodes: 0\n  terrain height nodes: 0\n  object-tree nodes: 0\n  object-tree objects: 0\n  object block bytes: 0"
        );

        assert!(is_dat_extension("DAT"));
        assert!(is_known_dat_name("config/config.dat"));
        assert!(is_known_dat_path(Path::new("terrain/terrain.dat")));
        assert!(!is_known_dat_name("unknown.dat"));
    }

    #[test]
    fn inspect_dat_files_aggregates_file_results() {
        let dir = std::env::temp_dir().join(format!("az-rs-cry-3d-engine-{}", std::process::id()));
        let config_dir = dir.join("config");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let path = config_dir.join("config.dat");
        std::fs::write(&path, b"abc").expect("write config dat");

        let inspection = inspect_dat_files([&path]).expect("inspect dat files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.engine_configs, 1);

        std::fs::remove_file(path).expect("remove config dat");
        std::fs::remove_dir(config_dir).expect("remove config dir");
        std::fs::remove_dir(dir).expect("remove temp dir");
    }
}
