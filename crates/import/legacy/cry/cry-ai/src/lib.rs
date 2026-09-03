//! `CryAI` navigation asset parsing.
//!
//! Follows Lumberyard's `dev/Gems/CryLegacy/Code/Source/CryAISystem`.

use std::{
    fmt, io,
    path::{Path, PathBuf},
    str,
};

use glam::Vec3;
use thiserror::Error;

pub mod source_transform;

pub use source_transform::*;

pub const AREA_FILE_VERSION_READ: u32 = 19;
pub const AREA_FILE_VERSION_WRITE: u32 = 24;
pub const COVER_FILE_VERSION_READ: u32 = 2;
pub const MNM_NAVIGATION_FILE_VERSION: u16 = 7;
pub const GRAPH_FILE_VERSION: u32 = 54;
pub const ROAD_NAVIGATION_FILE_VERSION: u32 = 2;
pub const BAI_EXTENSION: &str = "bai";

const NODE_DESCRIPTOR_SIZE: usize = 60;
const LINK_DESCRIPTOR_SIZE: usize = 44;

/// Binary AI asset family selected by `.bai` filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaiKind {
    Areas,
    Cover,
    MnmNavigation,
    Graph,
    RoadNavigation,
}

impl BaiKind {
    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Option<Self> {
        let name = path.as_ref().file_name()?.to_str()?;
        Self::from_file_name(name)
    }

    #[must_use]
    pub fn from_file_name(name: &str) -> Option<Self> {
        let name = name
            .strip_suffix(".bai")
            .or_else(|| name.strip_suffix(".BAI"))
            .unwrap_or(name);

        if name.starts_with("areas") {
            Some(Self::Areas)
        } else if name.starts_with("cover") {
            Some(Self::Cover)
        } else if name.starts_with("mnmnav") {
            Some(Self::MnmNavigation)
        } else if name.starts_with("net") {
            Some(Self::Graph)
        } else if name.starts_with("roadnav") {
            Some(Self::RoadNavigation)
        } else {
            None
        }
    }
}

/// Parsed `.bai` asset.
#[derive(Debug, Clone, Copy)]
pub enum BaiAsset<'a> {
    Areas(AreasBai),
    Cover(CoverBai),
    MnmNavigation(MnmNavigationBai<'a>),
    Graph(GraphBai<'a>),
    RoadNavigation(RoadNavigationBai),
}

impl BaiAsset<'_> {
    #[must_use]
    pub const fn kind(&self) -> BaiKind {
        match self {
            Self::Areas(_) => BaiKind::Areas,
            Self::Cover(_) => BaiKind::Cover,
            Self::MnmNavigation(_) => BaiKind::MnmNavigation,
            Self::Graph(_) => BaiKind::Graph,
            Self::RoadNavigation(_) => BaiKind::RoadNavigation,
        }
    }

    #[must_use]
    pub const fn summary(self) -> BaiSummary {
        BaiSummary::from_asset(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaiSummary {
    Areas {
        version: u32,
        designer_paths: usize,
        generic_shapes: usize,
    },
    Cover {
        version: u32,
        surfaces: usize,
    },
    MnmNavigation {
        version: u16,
        configuration_version: u32,
        areas: usize,
        agents: usize,
    },
    Graph {
        version: u32,
        nodes: usize,
        links: usize,
    },
    RoadNavigation {
        version: u32,
        roads: usize,
    },
}

impl BaiSummary {
    #[must_use]
    pub const fn from_asset(asset: BaiAsset<'_>) -> Self {
        match asset {
            BaiAsset::Areas(asset) => Self::Areas {
                version: asset.version,
                designer_paths: asset.designer_paths as usize,
                generic_shapes: asset.generic_shapes as usize,
            },
            BaiAsset::Cover(asset) => Self::Cover {
                version: asset.version,
                surfaces: asset.surfaces as usize,
            },
            BaiAsset::MnmNavigation(asset) => Self::MnmNavigation {
                version: asset.version,
                configuration_version: asset.configuration_version,
                areas: asset.areas as usize,
                agents: asset.agents.len() as usize,
            },
            BaiAsset::Graph(asset) => Self::Graph {
                version: asset.version,
                nodes: asset.nodes.len() as usize,
                links: asset.links.len() as usize,
            },
            BaiAsset::RoadNavigation(asset) => Self::RoadNavigation {
                version: asset.version,
                roads: asset.roads as usize,
            },
        }
    }

    #[must_use]
    pub fn label(self) -> String {
        self.to_string()
    }
}

impl fmt::Display for BaiSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Areas {
                version,
                designer_paths,
                generic_shapes,
            } => write!(
                f,
                "areas v{version}: {designer_paths} designer paths, {generic_shapes} generic shapes",
            ),
            Self::Cover { version, surfaces } => {
                write!(f, "cover v{version}: {surfaces} surfaces")
            }
            Self::MnmNavigation {
                version,
                configuration_version,
                areas,
                agents,
            } => write!(
                f,
                "mnm v{version} config {configuration_version}: {areas} areas, {agents} agents",
            ),
            Self::Graph {
                version,
                nodes,
                links,
            } => write!(f, "graph v{version}: {nodes} nodes, {links} links"),
            Self::RoadNavigation { version, roads } => {
                write!(f, "roadnav v{version}: {roads} roads")
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BaiTotals {
    pub files: usize,
    pub areas: usize,
    pub cover: usize,
    pub mnm_navigation: usize,
    pub graph: usize,
    pub road_navigation: usize,
    pub designer_paths: usize,
    pub generic_shapes: usize,
    pub cover_surfaces: usize,
    pub mnm_areas: usize,
    pub mnm_agents: usize,
    pub graph_nodes: usize,
    pub graph_links: usize,
    pub roads: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaiFileSummary {
    pub source: String,
    pub summary: BaiSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BaiInspection {
    pub rows: Vec<BaiFileSummary>,
    pub totals: BaiTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct BaiInspectionReport<'a> {
    inspection: &'a BaiInspection,
    limit: usize,
}

impl BaiTotals {
    pub const fn add_summary(&mut self, summary: BaiSummary) {
        self.files += 1;
        match summary {
            BaiSummary::Areas {
                designer_paths,
                generic_shapes,
                ..
            } => {
                self.areas += 1;
                self.designer_paths += designer_paths;
                self.generic_shapes += generic_shapes;
            }
            BaiSummary::Cover { surfaces, .. } => {
                self.cover += 1;
                self.cover_surfaces += surfaces;
            }
            BaiSummary::MnmNavigation { areas, agents, .. } => {
                self.mnm_navigation += 1;
                self.mnm_areas += areas;
                self.mnm_agents += agents;
            }
            BaiSummary::Graph { nodes, links, .. } => {
                self.graph += 1;
                self.graph_nodes += nodes;
                self.graph_links += links;
            }
            BaiSummary::RoadNavigation { roads, .. } => {
                self.road_navigation += 1;
                self.roads += roads;
            }
        }
    }
}

impl BaiInspection {
    pub fn add_file_summary(&mut self, row: BaiFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> BaiInspectionReport<'_> {
        BaiInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for BaiTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  areas: {}", self.areas)?;
        writeln!(f, "  cover: {}", self.cover)?;
        writeln!(f, "  mnm navigation: {}", self.mnm_navigation)?;
        writeln!(f, "  graph: {}", self.graph)?;
        writeln!(f, "  road navigation: {}", self.road_navigation)?;
        writeln!(f, "  designer paths: {}", self.designer_paths)?;
        writeln!(f, "  generic shapes: {}", self.generic_shapes)?;
        writeln!(f, "  cover surfaces: {}", self.cover_surfaces)?;
        writeln!(f, "  mnm areas: {}", self.mnm_areas)?;
        writeln!(f, "  mnm agents: {}", self.mnm_agents)?;
        writeln!(f, "  graph nodes: {}", self.graph_nodes)?;
        writeln!(f, "  graph links: {}", self.graph_links)?;
        writeln!(f, "  roads: {}", self.roads)
    }
}

impl fmt::Display for BaiInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.summary)?;
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

/// Summarises one `.bai` navigation cache's section counts.
///
/// # Errors
///
/// Returns any error [`parse_bai`] returns —
/// [`BaiParseError::UnexpectedEof`], [`BaiParseError::UnsupportedVersion`],
/// [`BaiParseError::InvalidSectionSize`],
/// [`BaiParseError::UnsupportedNonEmpty`], [`BaiParseError::TrailingBytes`] or
/// [`BaiParseError::Utf8`].
pub fn summarize_bai(bytes: &[u8], kind: BaiKind) -> Result<BaiSummary, BaiParseError> {
    parse_bai(bytes, kind).map(BaiAsset::summary)
}

#[derive(Debug, Error)]
pub enum BaiInspectionError {
    #[error("unknown BAI family for {path}")]
    UnknownPath { path: String },
    #[error("read CryAI BAI {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse CryAI BAI {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: BaiParseError,
    },
}

/// Summarises one `.bai` file's bytes, selecting the family from `path`.
///
/// `path` selects the BAI family and becomes the display label; it is not read
/// from disk.
///
/// # Errors
///
/// Returns [`BaiInspectionError::UnknownPath`] if `path`'s file name matches
/// no known BAI family, or [`BaiInspectionError::Parse`] wrapping the
/// [`BaiParseError`] from a malformed cache.
pub fn inspect_bai_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<BaiFileSummary, BaiInspectionError> {
    let path = path.as_ref();
    let kind = BaiKind::from_path(path).ok_or_else(|| BaiInspectionError::UnknownPath {
        path: path.display().to_string(),
    })?;
    Ok(BaiFileSummary {
        source: path.display().to_string(),
        summary: summarize_bai(bytes, kind).map_err(|source| BaiInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        })?,
    })
}

/// Reads a `.bai` navigation cache from disk and summarises it.
///
/// # Errors
///
/// Returns [`BaiInspectionError::Read`] if `path` cannot be read (missing
/// file, permissions), plus any error [`inspect_bai_file`] returns —
/// [`BaiInspectionError::UnknownPath`] for an unrecognised family, or
/// [`BaiInspectionError::Parse`] for a malformed cache.
pub fn inspect_bai_path(path: impl AsRef<Path>) -> Result<BaiFileSummary, BaiInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| BaiInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_bai_file(path, &bytes)
}

/// Reads and summarises every `.bai` cache in `paths`, accumulating totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_bai_path`] returns for the first path that
/// fails — [`BaiInspectionError::Read`] for an unreadable file,
/// [`BaiInspectionError::UnknownPath`] for an unrecognised family, or
/// [`BaiInspectionError::Parse`] for a malformed cache.
pub fn inspect_bai_files(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<BaiInspection, BaiInspectionError> {
    let mut inspection = BaiInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_bai_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_bai_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(BAI_EXTENSION)
}

#[must_use]
pub fn is_bai_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_bai_extension(extension))
}

#[must_use]
pub fn is_bai_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_bai_extension)
}

/// `areas*.bai` designer path and shape counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AreasBai {
    pub version: u32,
    pub designer_paths: u32,
    pub generic_shapes: u32,
}

/// `cover*.bai` cover surface count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverBai {
    pub version: u32,
    pub surfaces: u32,
}

/// `mnmnav*.bai` navigation mesh header and agent table.
#[derive(Debug, Clone, Copy)]
pub struct MnmNavigationBai<'a> {
    pub version: u16,
    pub configuration_version: u32,
    pub areas: u32,
    pub agents: MnmAgentRecords<'a>,
}

/// Borrowed `mnmnav*.bai` agent record table.
#[derive(Debug, Clone, Copy)]
pub struct MnmAgentRecords<'a> {
    bytes: &'a [u8],
    count: u32,
}

impl<'a> MnmAgentRecords<'a> {
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.count
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub const fn iter(&self) -> MnmAgentIter<'a> {
        MnmAgentIter {
            cursor: Cursor::new(self.bytes),
            remaining: self.count,
        }
    }
}

impl<'a> IntoIterator for MnmAgentRecords<'a> {
    type Item = Result<MnmAgentRef<'a>, BaiParseError>;
    type IntoIter = MnmAgentIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &MnmAgentRecords<'a> {
    type Item = Result<MnmAgentRef<'a>, BaiParseError>;
    type IntoIter = MnmAgentIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over borrowed `mnmnav*.bai` agent records.
// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct MnmAgentIter<'a> {
    cursor: Cursor<'a>,
    remaining: u32,
}

impl<'a> Iterator for MnmAgentIter<'a> {
    type Item = Result<MnmAgentRef<'a>, BaiParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(read_mnm_agent(&mut self.cursor))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.remaining as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for MnmAgentIter<'_> {}

/// Borrowed `mnmnav*.bai` agent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MnmAgentRef<'a> {
    pub name: &'a str,
    pub total_memory: u32,
    pub meshes: u32,
}

/// `net*.bai` old graph navigation data.
#[derive(Debug, Clone, Copy)]
pub struct GraphBai<'a> {
    pub version: u32,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
    pub nodes: NodeDescriptors<'a>,
    pub links: LinkDescriptors<'a>,
}

/// Borrowed graph node descriptor table.
#[derive(Debug, Clone, Copy)]
pub struct NodeDescriptors<'a> {
    bytes: &'a [u8],
    count: u32,
}

impl<'a> NodeDescriptors<'a> {
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.count
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub fn get(&self, index: u32) -> Option<NodeDescriptor> {
        let offset = (index as usize).checked_mul(NODE_DESCRIPTOR_SIZE)?;
        let bytes = self.bytes.get(offset..offset + NODE_DESCRIPTOR_SIZE)?;
        Some(read_node_descriptor(bytes))
    }

    #[must_use]
    pub const fn iter(&self) -> NodeDescriptorIter<'a> {
        NodeDescriptorIter {
            bytes: self.bytes,
            index: 0,
            count: self.count,
        }
    }
}

impl<'a> IntoIterator for NodeDescriptors<'a> {
    type Item = NodeDescriptor;
    type IntoIter = NodeDescriptorIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &NodeDescriptors<'a> {
    type Item = NodeDescriptor;
    type IntoIter = NodeDescriptorIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over graph node descriptors.
// Not `Copy`: a copyable iterator silently restarts when passed by value.
#[derive(Debug, Clone)]
pub struct NodeDescriptorIter<'a> {
    bytes: &'a [u8],
    index: u32,
    count: u32,
}

impl Iterator for NodeDescriptorIter<'_> {
    type Item = NodeDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.count {
            return None;
        }
        let offset = self.index as usize * NODE_DESCRIPTOR_SIZE;
        self.index += 1;
        Some(read_node_descriptor(
            &self.bytes[offset..offset + NODE_DESCRIPTOR_SIZE],
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.count - self.index) as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for NodeDescriptorIter<'_> {}

/// `CryAI` `NodeDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeDescriptor {
    pub id: u32,
    pub dir: Vec3,
    pub up: Vec3,
    pub pos: Vec3,
    pub index: i32,
    pub obstacle: [i32; 3],
    pub nav_type: u16,
    pub flags: NodeDescriptorFlags,
}

/// Bit-packed `CryAI` waypoint flags from `NodeDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeDescriptorFlags {
    pub waypoint_type: u8,
    pub forbidden: bool,
    pub forbidden_designer: bool,
    pub removable: bool,
}

/// Borrowed graph link descriptor table.
#[derive(Debug, Clone, Copy)]
pub struct LinkDescriptors<'a> {
    bytes: &'a [u8],
    count: u32,
}

impl<'a> LinkDescriptors<'a> {
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.count
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub fn get(&self, index: u32) -> Option<LinkDescriptor> {
        let offset = (index as usize).checked_mul(LINK_DESCRIPTOR_SIZE)?;
        let bytes = self.bytes.get(offset..offset + LINK_DESCRIPTOR_SIZE)?;
        Some(read_link_descriptor(bytes))
    }
}

/// `CryAI` `LinkDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkDescriptor {
    pub source_node: u32,
    pub target_node: u32,
    pub edge_center: Vec3,
    pub max_pass_radius: f32,
    pub exposure: f32,
    pub length: f32,
    pub max_water_depth: f32,
    pub min_water_depth: f32,
    pub start_index: u8,
    pub end_index: u8,
    pub flags: LinkDescriptorFlags,
}

/// Bit-packed `CryAI` link flags from `LinkDescriptor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkDescriptorFlags {
    pub pure_triangular_link: bool,
    pub simple_passability_check: bool,
}

/// `roadnav*.bai` road navigation count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoadNavigationBai {
    pub version: u32,
    pub roads: u32,
}

/// Parse a `.bai` navigation cache of the given family.
///
/// # Errors
///
/// Returns [`BaiParseError::UnexpectedEof`] when the buffer ends mid-field
/// (the `field` payload names it), [`BaiParseError::UnsupportedVersion`] for a
/// version this reader does not accept for `kind`,
/// [`BaiParseError::InvalidSectionSize`] when a declared section size is not a
/// whole number of records, [`BaiParseError::UnsupportedNonEmpty`] when a
/// section this reader only handles empty carries entries,
/// [`BaiParseError::TrailingBytes`] when bytes remain after the last section,
/// and [`BaiParseError::Utf8`] when an embedded name is not valid UTF-8.
pub fn parse_bai(bytes: &[u8], kind: BaiKind) -> Result<BaiAsset<'_>, BaiParseError> {
    match kind {
        BaiKind::Areas => parse_areas(bytes).map(BaiAsset::Areas),
        BaiKind::Cover => parse_cover(bytes).map(BaiAsset::Cover),
        BaiKind::MnmNavigation => parse_mnm_navigation(bytes).map(BaiAsset::MnmNavigation),
        BaiKind::Graph => parse_graph(bytes).map(BaiAsset::Graph),
        BaiKind::RoadNavigation => parse_road_navigation(bytes).map(BaiAsset::RoadNavigation),
    }
}

fn parse_areas(bytes: &[u8]) -> Result<AreasBai, BaiParseError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u32("version")?;
    if version < AREA_FILE_VERSION_READ {
        return Err(BaiParseError::UnsupportedVersion {
            kind: BaiKind::Areas,
            version,
        });
    }
    let designer_paths = cursor.u32("designer_paths")?;
    let generic_shapes = cursor.u32("generic_shapes")?;
    ensure_end(cursor)?;
    Ok(AreasBai {
        version,
        designer_paths,
        generic_shapes,
    })
}

fn parse_cover(bytes: &[u8]) -> Result<CoverBai, BaiParseError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u32("version")?;
    if version < COVER_FILE_VERSION_READ {
        return Err(BaiParseError::UnsupportedVersion {
            kind: BaiKind::Cover,
            version,
        });
    }
    let surfaces = cursor.u32("surfaces")?;
    ensure_end(cursor)?;
    Ok(CoverBai { version, surfaces })
}

fn parse_mnm_navigation(bytes: &[u8]) -> Result<MnmNavigationBai<'_>, BaiParseError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u16("version")?;
    if version != MNM_NAVIGATION_FILE_VERSION {
        return Err(BaiParseError::UnsupportedVersion {
            kind: BaiKind::MnmNavigation,
            version: u32::from(version),
        });
    }
    let configuration_version = cursor.u32("configuration_version")?;
    let areas = cursor.u32("areas")?;
    if areas != 0 {
        return Err(BaiParseError::UnsupportedNonEmpty {
            kind: BaiKind::MnmNavigation,
            section: "areas",
            count: areas,
        });
    }
    let agent_count = cursor.u32("agents")?;
    let records = cursor.remaining();
    let agents = MnmAgentRecords {
        bytes: records,
        count: agent_count,
    };

    let mut validation = agents.iter();
    for agent in validation.by_ref() {
        agent?;
    }
    if validation.cursor.remaining_len() != 0 {
        return Err(BaiParseError::TrailingBytes(
            validation.cursor.remaining_len(),
        ));
    }

    Ok(MnmNavigationBai {
        version,
        configuration_version,
        areas,
        agents,
    })
}

fn parse_graph(bytes: &[u8]) -> Result<GraphBai<'_>, BaiParseError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u32("version")?;
    if version != GRAPH_FILE_VERSION {
        return Err(BaiParseError::UnsupportedVersion {
            kind: BaiKind::Graph,
            version,
        });
    }
    let bounds_min = cursor.vec3("bounds_min")?;
    let bounds_max = cursor.vec3("bounds_max")?;
    let node_count = cursor.u32("nodes")?;
    let node_bytes_len = checked_table_len(node_count, NODE_DESCRIPTOR_SIZE, "nodes")?;
    let node_bytes = cursor.take(node_bytes_len, "nodes")?;
    let nodes = NodeDescriptors {
        bytes: node_bytes,
        count: node_count,
    };
    let link_count = cursor.u32("links")?;
    let link_bytes_len = checked_table_len(link_count, LINK_DESCRIPTOR_SIZE, "links")?;
    let link_bytes = cursor.take(link_bytes_len, "links")?;
    let links = LinkDescriptors {
        bytes: link_bytes,
        count: link_count,
    };
    ensure_end(cursor)?;
    Ok(GraphBai {
        version,
        bounds_min,
        bounds_max,
        nodes,
        links,
    })
}

fn parse_road_navigation(bytes: &[u8]) -> Result<RoadNavigationBai, BaiParseError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u32("version")?;
    if version != ROAD_NAVIGATION_FILE_VERSION {
        return Err(BaiParseError::UnsupportedVersion {
            kind: BaiKind::RoadNavigation,
            version,
        });
    }
    let roads = cursor.u32("roads")?;
    ensure_end(cursor)?;
    Ok(RoadNavigationBai { version, roads })
}

fn read_mnm_agent<'a>(cursor: &mut Cursor<'a>) -> Result<MnmAgentRef<'a>, BaiParseError> {
    let name_len = cursor.u32("agent.name_len")? as usize;
    let name = cursor.take(name_len, "agent.name")?;
    let name = str::from_utf8(name)?;
    let total_memory = cursor.u32("agent.total_memory")?;
    if total_memory < 4 {
        return Err(BaiParseError::InvalidSectionSize {
            section: "agent.total_memory",
            size: total_memory as usize,
        });
    }
    let payload = cursor.take(total_memory as usize, "agent.payload")?;
    let meshes = read_u32(payload, 0);
    if meshes != 0 {
        return Err(BaiParseError::UnsupportedNonEmpty {
            kind: BaiKind::MnmNavigation,
            section: "agent.meshes",
            count: meshes,
        });
    }
    Ok(MnmAgentRef {
        name,
        total_memory,
        meshes,
    })
}

fn read_node_descriptor(bytes: &[u8]) -> NodeDescriptor {
    let flags = bytes[58];
    NodeDescriptor {
        id: read_u32(bytes, 0),
        dir: read_vec3(bytes, 4),
        up: read_vec3(bytes, 16),
        pos: read_vec3(bytes, 28),
        index: read_i32(bytes, 40),
        obstacle: [
            read_i32(bytes, 44),
            read_i32(bytes, 48),
            read_i32(bytes, 52),
        ],
        nav_type: read_u16(bytes, 56),
        flags: NodeDescriptorFlags {
            waypoint_type: flags & 0x0f,
            forbidden: flags & 0x10 != 0,
            forbidden_designer: flags & 0x20 != 0,
            removable: flags & 0x40 != 0,
        },
    }
}

fn read_link_descriptor(bytes: &[u8]) -> LinkDescriptor {
    let flags = bytes[42];
    LinkDescriptor {
        source_node: read_u32(bytes, 0),
        target_node: read_u32(bytes, 4),
        edge_center: read_vec3(bytes, 8),
        max_pass_radius: read_f32(bytes, 20),
        exposure: read_f32(bytes, 24),
        length: read_f32(bytes, 28),
        max_water_depth: read_f32(bytes, 32),
        min_water_depth: read_f32(bytes, 36),
        start_index: bytes[40],
        end_index: bytes[41],
        flags: LinkDescriptorFlags {
            pure_triangular_link: flags & 0x01 != 0,
            simple_passability_check: flags & 0x02 != 0,
        },
    }
}

fn checked_table_len(
    count: u32,
    stride: usize,
    section: &'static str,
) -> Result<usize, BaiParseError> {
    (count as usize)
        .checked_mul(stride)
        .ok_or(BaiParseError::InvalidSectionSize {
            section,
            size: usize::MAX,
        })
}

const fn ensure_end(cursor: Cursor<'_>) -> Result<(), BaiParseError> {
    let remaining = cursor.remaining_len();
    if remaining == 0 {
        Ok(())
    } else {
        Err(BaiParseError::TrailingBytes(remaining))
    }
}

#[derive(Debug, Clone, Copy)]
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    #[must_use]
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    #[must_use]
    const fn remaining_len(&self) -> usize {
        self.bytes.len() - self.offset
    }

    #[must_use]
    const fn remaining(&self) -> &'a [u8] {
        let (_, remaining) = self.bytes.split_at(self.offset);
        remaining
    }

    fn take(&mut self, len: usize, field: &'static str) -> Result<&'a [u8], BaiParseError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(BaiParseError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(BaiParseError::UnexpectedEof { field })?;
        self.offset = end;
        Ok(bytes)
    }

    fn u16(&mut self, field: &'static str) -> Result<u16, BaiParseError> {
        let bytes = self.take(2, field)?;
        Ok(read_u16(bytes, 0))
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, BaiParseError> {
        let bytes = self.take(4, field)?;
        Ok(read_u32(bytes, 0))
    }

    fn vec3(&mut self, field: &'static str) -> Result<Vec3, BaiParseError> {
        let bytes = self.take(12, field)?;
        Ok(read_vec3(bytes, 0))
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_vec3(bytes: &[u8], offset: usize) -> Vec3 {
    Vec3::new(
        read_f32(bytes, offset),
        read_f32(bytes, offset + 4),
        read_f32(bytes, offset + 8),
    )
}

#[derive(Debug, Error)]
pub enum BaiParseError {
    #[error("unexpected end of BAI file while reading `{field}`")]
    UnexpectedEof { field: &'static str },
    #[error("unsupported {kind:?} BAI version `{version}`")]
    UnsupportedVersion { kind: BaiKind, version: u32 },
    #[error("non-empty {kind:?} BAI section `{section}` has {count} entries")]
    UnsupportedNonEmpty {
        kind: BaiKind,
        section: &'static str,
        count: u32,
    },
    #[error("invalid BAI section `{section}` size `{size}`")]
    InvalidSectionSize { section: &'static str, size: usize },
    #[error("BAI file has {0} trailing bytes")]
    TrailingBytes(usize),
    #[error("BAI string is not utf-8")]
    Utf8(#[from] str::Utf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_area_file() {
        let bytes = [
            24, 0, 0, 0, //
            0, 0, 0, 0, //
            0, 0, 0, 0,
        ];
        let parsed = parse_bai(&bytes, BaiKind::Areas).unwrap();
        let BaiAsset::Areas(asset) = parsed else {
            panic!("expected areas");
        };

        assert_eq!(asset.version, AREA_FILE_VERSION_WRITE);
        assert_eq!(asset.designer_paths, 0);
        assert_eq!(asset.generic_shapes, 0);

        let summary = parsed.summary();
        assert_eq!(
            summary,
            BaiSummary::Areas {
                version: AREA_FILE_VERSION_WRITE,
                designer_paths: 0,
                generic_shapes: 0,
            }
        );
        let mut totals = BaiTotals::default();
        totals.add_summary(summary);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.areas, 1);
        assert_eq!(
            summary.to_string(),
            "areas v24: 0 designer paths, 0 generic shapes"
        );
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  areas: 1\n  cover: 0\n  mnm navigation: 0\n  graph: 0\n  road navigation: 0\n  designer paths: 0\n  generic shapes: 0\n  cover surfaces: 0\n  mnm areas: 0\n  mnm agents: 0\n  graph nodes: 0\n  graph links: 0\n  roads: 0\n"
        );

        let row = inspect_bai_file("ai/areas.bai", &bytes).unwrap();
        let mut inspection = BaiInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "ai/areas.bai: areas v24: 0 designer paths, 0 generic shapes\n  files: 1\n  areas: 1\n  cover: 0\n  mnm navigation: 0\n  graph: 0\n  road navigation: 0\n  designer paths: 0\n  generic shapes: 0\n  cover surfaces: 0\n  mnm areas: 0\n  mnm agents: 0\n  graph nodes: 0\n  graph links: 0\n  roads: 0\n"
        );
    }

    #[test]
    fn parses_mnm_agents() {
        let mut bytes = Vec::new();
        bytes.extend(MNM_NAVIGATION_FILE_VERSION.to_le_bytes());
        bytes.extend(6u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(b"Human");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());

        let BaiAsset::MnmNavigation(asset) = parse_bai(&bytes, BaiKind::MnmNavigation).unwrap()
        else {
            panic!("expected mnm navigation");
        };
        let agents = asset.agents.iter().collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(asset.configuration_version, 6);
        assert_eq!(
            agents,
            [MnmAgentRef {
                name: "Human",
                total_memory: 4,
                meshes: 0,
            }]
        );
    }

    #[test]
    fn recognizes_bai_paths() {
        assert!(is_bai_name("areas.BAI"));
        assert!(is_bai_path(Path::new("ai/areas.bai")));
        assert!(!is_bai_name("areas.bin"));
    }

    #[test]
    fn parses_empty_graph_file() {
        let mut bytes = Vec::new();
        bytes.extend(GRAPH_FILE_VERSION.to_le_bytes());
        bytes.extend([0u8; 24]);
        bytes.extend(1u32.to_le_bytes());
        let mut node = [0u8; NODE_DESCRIPTOR_SIZE];
        node[0..4].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend(node);
        bytes.extend(0u32.to_le_bytes());

        let BaiAsset::Graph(asset) = parse_bai(&bytes, BaiKind::Graph).unwrap() else {
            panic!("expected graph");
        };
        let node = asset.nodes.get(0).unwrap();

        assert_eq!(asset.version, GRAPH_FILE_VERSION);
        assert_eq!(asset.nodes.len(), 1);
        assert_eq!(asset.links.len(), 0);
        assert_eq!(node.id, 1);
    }
}
