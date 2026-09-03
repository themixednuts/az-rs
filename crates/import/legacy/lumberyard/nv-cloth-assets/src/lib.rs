//! `NvCloth` asset parsers.

pub mod source_transform;

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use az_core::{AssetPathBuf, AssetPathError};
use az_nv_cloth::{
    ClothFabric, ClothFabricSource, ClothMaterial as ClothMaterialAsset, ClothMaterialBinding,
    ClothMesh, ClothPaintMaps, ClothRenderMapping, ClothSimulationVertex, ClothSkinMapEntry,
    ClothSkinMapRange, FabricCookedData as OwnedFabricCookedData, FabricPhaseConfigs,
    FabricPhaseType, MotionConstraintConfig, PhaseConfig, SelfCollisionConfig,
    TetherConstraintConfig,
};
use bevy::math::{Quat, Vec3};
use thiserror::Error;

/// Size of a `.clothmaterial` payload in bytes.
pub const CLOTH_MATERIAL_SIZE: usize = 208;
pub const CLOTH_ASSET_HEADER_SIZE: usize = 16;
pub const CLOTH_ASSET_VERSION: u32 = 1;
pub const CLOTH_ASSET_EXTENSION: &str = "cloth";
pub const CLOTH_MATERIAL_EXTENSION: &str = "clothmaterial";

pub use source_transform::{
    ClothFabricSourceTransform, ClothFabricSourceTransformError, ClothMaterialSourceTransform,
    ClothMaterialSourceTransformError, cloth_fabric_source_path, cloth_material_source_path,
    is_legacy_cloth_fabric_source, is_legacy_cloth_material_source,
};

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ClothInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse cloth asset {path:?}: {source}")]
    Parse {
        path: PathBuf,
        source: ClothParseError,
    },
}

const FABRIC_LAYOUT_OFFSET: usize = CLOTH_ASSET_HEADER_SIZE + CLOTH_MATERIAL_SIZE;
const FABRIC_LAYOUT_SIZE: usize = 272;
const FABRIC_INTERNAL_ARRAYS: usize = 10;
const FABRIC_INTERNAL_OFFSETS: usize = 11;
const FABRIC_GEOMETRY_OFFSETS: usize = 11;

/// Parse a `.clothmaterial` payload.
///
/// # Errors
///
/// Returns an error when the payload is not the fixed material block size or
/// contains non-zero vector padding.
pub fn parse_cloth_material(bytes: &[u8]) -> Result<ClothMaterialAsset, ClothMaterialParseError> {
    if bytes.len() != CLOTH_MATERIAL_SIZE {
        return Err(ClothMaterialParseError::InvalidSize {
            actual: bytes.len(),
            expected: CLOTH_MATERIAL_SIZE,
        });
    }

    let mut reader = FloatReader::new(bytes);
    let phase_configs = FabricPhaseConfigs::new(
        reader.read_phase_config(),
        reader.read_phase_config(),
        reader.read_phase_config(),
        reader.read_phase_config(),
    );
    let stiffness_frequency = reader.read_f32();
    let motion_constraints = MotionConstraintConfig::new(
        reader.read_f32(),
        reader.read_f32(),
        reader.read_f32(),
        reader.read_f32(),
    );
    let self_collision = SelfCollisionConfig::new(reader.read_f32(), reader.read_f32());
    let tether_constraints = TetherConstraintConfig::new(reader.read_f32(), reader.read_f32());
    let solver_frequency = reader.read_f32();
    let acceleration_filter_width = reader.read_f32();
    let continuous_collision = reader.read_bool_u32("continuous collision")?;
    let damping = reader.read_padded_vec3("damping")?;
    let linear_drag = reader.read_padded_vec3("linear_drag")?;
    let angular_drag = reader.read_padded_vec3("angular_drag")?;
    let linear_inertia = reader.read_padded_vec3("linear_inertia")?;
    let angular_inertia = reader.read_padded_vec3("angular_inertia")?;
    let centrifugal_inertia = reader.read_padded_vec3("centrifugal_inertia")?;

    Ok(ClothMaterialAsset::new(
        phase_configs,
        stiffness_frequency,
        motion_constraints,
        self_collision,
        tether_constraints,
        solver_frequency,
        acceleration_filter_width,
        continuous_collision,
        damping,
        linear_drag,
        angular_drag,
        linear_inertia,
        angular_inertia,
        centrifugal_inertia,
    ))
}

/// Write a `.clothmaterial` payload.
///
/// # Errors
///
/// Returns an error when the writer fails.
pub fn write_cloth_material(
    material: &ClothMaterialAsset,
    mut writer: impl io::Write,
) -> io::Result<()> {
    let mut bytes = Vec::with_capacity(CLOTH_MATERIAL_SIZE);
    write_phase_config(&mut bytes, material.phase_configs.horizontal);
    write_phase_config(&mut bytes, material.phase_configs.vertical);
    write_phase_config(&mut bytes, material.phase_configs.bending);
    write_phase_config(&mut bytes, material.phase_configs.shearing);
    write_f32(&mut bytes, material.stiffness_frequency);
    write_f32(&mut bytes, material.motion_constraints.max_distance);
    write_f32(&mut bytes, material.motion_constraints.scale);
    write_f32(&mut bytes, material.motion_constraints.bias);
    write_f32(&mut bytes, material.motion_constraints.stiffness);
    write_f32(&mut bytes, material.self_collision.distance);
    write_f32(&mut bytes, material.self_collision.stiffness);
    write_f32(&mut bytes, material.tether_constraints.stiffness);
    write_f32(&mut bytes, material.tether_constraints.scale);
    write_f32(&mut bytes, material.solver_frequency);
    write_f32(&mut bytes, material.acceleration_filter_width);
    bytes.extend(u32::from(material.continuous_collision).to_le_bytes());
    write_padded_vec3(&mut bytes, material.damping);
    write_padded_vec3(&mut bytes, material.linear_drag);
    write_padded_vec3(&mut bytes, material.angular_drag);
    write_padded_vec3(&mut bytes, material.linear_inertia);
    write_padded_vec3(&mut bytes, material.angular_inertia);
    write_padded_vec3(&mut bytes, material.centrifugal_inertia);

    debug_assert_eq!(bytes.len(), CLOTH_MATERIAL_SIZE);
    writer.write_all(&bytes)
}

/// Summarizes a cloth payload, choosing the reader from `path`'s extension.
///
/// # Errors
///
/// Returns [`ClothParseError::UnsupportedExtension`] when `path` is neither
/// `.cloth` nor `.clothmaterial`, [`ClothParseError::Asset`] for any error
/// [`parse_cloth_asset`] returns, and [`ClothParseError::Material`] for any
/// error [`parse_cloth_material`] returns.
pub fn summarize_cloth_path(path: &Path, bytes: &[u8]) -> Result<ClothSummary, ClothParseError> {
    if is_cloth_asset_path(path) {
        parse_cloth_asset(bytes)
            .map(|asset| ClothSummary::from_asset(&asset))
            .map_err(ClothParseError::Asset)
    } else if is_cloth_material_path(path) {
        parse_cloth_material(bytes)
            .map(|material| ClothSummary::from_material(&material))
            .map_err(ClothParseError::Material)
    } else {
        Err(ClothParseError::UnsupportedExtension)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothSummary {
    pub kind: ClothSummaryKind,
    pub stiffness_frequency: f32,
    pub solver_frequency: f32,
    pub damping: Vec3,
    pub linear_inertia: Vec3,
    pub continuous_collision: bool,
    pub motion_max_distance: f32,
}

impl ClothSummary {
    #[must_use]
    pub const fn from_asset(asset: &ClothAsset<'_>) -> Self {
        let material = &asset.material;
        Self {
            kind: ClothSummaryKind::Asset {
                particles: asset.internal.num_particles,
                triangle_indices: asset.internal.triangles.len() / 4,
            },
            stiffness_frequency: material.stiffness_frequency,
            solver_frequency: material.solver_frequency,
            damping: material.damping,
            linear_inertia: material.linear_inertia,
            continuous_collision: material.continuous_collision,
            motion_max_distance: material.motion_constraints.max_distance,
        }
    }

    #[must_use]
    pub const fn from_material(material: &ClothMaterialAsset) -> Self {
        Self {
            kind: ClothSummaryKind::Material,
            stiffness_frequency: material.stiffness_frequency,
            solver_frequency: material.solver_frequency,
            damping: material.damping,
            linear_inertia: material.linear_inertia,
            continuous_collision: material.continuous_collision,
            motion_max_distance: material.motion_constraints.max_distance,
        }
    }
}

impl fmt::Display for ClothSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ClothSummaryKind::Asset {
                particles,
                triangle_indices,
            } => write!(
                f,
                "ClothAsset, {particles} particles, {triangle_indices} triangle indices, stiffness={} solver={}",
                self.stiffness_frequency, self.solver_frequency
            ),
            ClothSummaryKind::Material => write!(
                f,
                "ClothMaterialAsset, stiffness={} solver={} damping={} linear_inertia={}",
                self.stiffness_frequency,
                self.solver_frequency,
                format_vec3(self.damping),
                format_vec3(self.linear_inertia)
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClothSummaryKind {
    Asset {
        particles: u32,
        triangle_indices: usize,
    },
    Material,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ClothMaterialTotals {
    pub files: usize,
    pub stiffness_frequency: FloatStats,
    pub solver_frequency: FloatStats,
    pub continuous_collision_files: usize,
    pub motion_max_distance: Option<FloatRange>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClothFileSummary {
    pub source: String,
    pub summary: ClothSummary,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ClothInspection {
    pub rows: Vec<ClothFileSummary>,
    pub totals: ClothMaterialTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct ClothInspectionReport<'a> {
    inspection: &'a ClothInspection,
    limit: usize,
}

impl ClothMaterialTotals {
    pub fn add_summary(&mut self, summary: ClothSummary) {
        self.files += 1;
        self.stiffness_frequency.add(summary.stiffness_frequency);
        self.solver_frequency.add(summary.solver_frequency);
        self.continuous_collision_files += usize::from(summary.continuous_collision);
        self.motion_max_distance = Some(FloatRange::with_value(
            self.motion_max_distance,
            summary.motion_max_distance,
        ));
    }
}

impl ClothInspection {
    pub fn add_file_summary(&mut self, row: ClothFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> ClothInspectionReport<'_> {
        ClothInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for ClothMaterialTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  stiffness frequency: {}", self.stiffness_frequency)?;
        writeln!(f, "  solver frequency: {}", self.solver_frequency)?;
        writeln!(
            f,
            "  continuous collision: {} files",
            self.continuous_collision_files
        )?;
        writeln!(
            f,
            "  max motion distance range: {}",
            self.motion_max_distance
                .map_or_else(|| "n/a".to_string(), |range| range.to_string())
        )
    }
}

impl fmt::Display for ClothInspectionReport<'_> {
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

/// Summarizes one cloth payload and labels it with its display path.
///
/// # Errors
///
/// Returns any error [`summarize_cloth_path`] returns.
pub fn inspect_cloth_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<ClothFileSummary, ClothParseError> {
    let path = path.as_ref();
    Ok(ClothFileSummary {
        source: path.display().to_string(),
        summary: summarize_cloth_path(path, bytes)?,
    })
}

/// Reads and summarizes one cloth asset from disk.
///
/// # Errors
///
/// Returns [`ClothInspectionError::Read`] when `path` cannot be read, and
/// [`ClothInspectionError::Parse`] when its contents do not parse as the cloth
/// asset or cloth material the extension names.
pub fn inspect_cloth_path(
    path: impl AsRef<Path>,
) -> Result<ClothFileSummary, ClothInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| ClothInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_cloth_file(path, &bytes).map_err(|source| ClothInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and aggregates every cloth asset in `paths`.
///
/// # Errors
///
/// Returns the first error [`inspect_cloth_path`] returns; the walk stops at
/// that path and the partial inspection is discarded.
pub fn inspect_cloth_files<I, P>(paths: I) -> Result<ClothInspection, ClothInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = ClothInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_cloth_path(path)?);
    }
    Ok(inspection)
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FloatStats {
    count: usize,
    sum: f32,
    range: Option<FloatRange>,
}

impl FloatStats {
    pub fn add(&mut self, value: f32) {
        self.count += 1;
        self.sum += value;
        self.range = Some(FloatRange::with_value(self.range, value));
    }
}

impl std::fmt::Display for FloatStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.range {
            Some(range) => {
                // An inspection run never reaches `u32::MAX` samples, and
                // `f64::from` is exact for every count below it, so the mean
                // needs no lossy `as` cast.
                let count = f64::from(u32::try_from(self.count).unwrap_or(u32::MAX));
                let average = f64::from(self.sum) / count;
                write!(f, "min={} max={} avg={average}", range.min, range.max)
            }
            None => f.write_str("n/a"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatRange {
    min: f32,
    max: f32,
}

impl FloatRange {
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self {
            min: value,
            max: value,
        }
    }

    #[must_use]
    pub const fn with_value(range: Option<Self>, value: f32) -> Self {
        match range {
            Some(range) => Self {
                min: range.min.min(value),
                max: range.max.max(value),
            },
            None => Self::new(value),
        }
    }
}

impl std::fmt::Display for FloatRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "min={} max={}", self.min, self.max)
    }
}

fn format_vec3(value: Vec3) -> String {
    format!("({}, {}, {})", value.x, value.y, value.z)
}

#[derive(Debug, Error, PartialEq)]
pub enum ClothParseError {
    #[error(transparent)]
    Material(#[from] ClothMaterialParseError),
    #[error(transparent)]
    Asset(#[from] ClothAssetParseError),
    #[error("unsupported cloth extension")]
    UnsupportedExtension,
}

#[must_use]
pub const fn is_cloth_asset_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(CLOTH_ASSET_EXTENSION)
}

#[must_use]
pub fn is_cloth_asset_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_cloth_asset_extension(extension))
}

#[must_use]
pub fn is_cloth_asset_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_cloth_asset_extension)
}

#[must_use]
pub const fn is_cloth_material_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(CLOTH_MATERIAL_EXTENSION)
}

#[must_use]
pub fn is_cloth_material_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_cloth_material_extension(extension))
}

#[must_use]
pub fn is_cloth_material_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_cloth_material_extension)
}

#[must_use]
pub fn is_cloth_path(path: &Path) -> bool {
    is_cloth_asset_path(path) || is_cloth_material_path(path)
}

/// Parse a `.cloth` asset payload.
///
/// # Errors
///
/// Returns [`ClothAssetParseError::UnexpectedEof`] when the 16-byte header,
/// the fabric layout block, or a scalar read runs past the end of `bytes`,
/// [`ClothAssetParseError::UnsupportedVersion`] for a header version other
/// than [`CLOTH_ASSET_VERSION`], [`ClothAssetParseError::OutOfBounds`] when a
/// cooked-array or geometry offset points outside the file, and
/// [`ClothAssetParseError::Material`] when the embedded material block is
/// malformed.
pub fn parse_cloth_asset(bytes: &[u8]) -> Result<ClothAsset<'_>, ClothAssetParseError> {
    let header = ClothAssetHeader::parse(bytes)?;
    let material = parse_cloth_material(slice(
        bytes,
        CLOTH_ASSET_HEADER_SIZE,
        CLOTH_MATERIAL_SIZE,
        "cloth material",
    )?)?;
    let layout = FabricCookedDataLayout::parse(slice(
        bytes,
        FABRIC_LAYOUT_OFFSET,
        FABRIC_LAYOUT_SIZE,
        "fabric layout",
    )?)?;
    let internal = layout.internal_data(bytes)?;
    let geometry = layout.geometry();
    geometry.validate(bytes.len())?;

    Ok(ClothAsset {
        header,
        material,
        internal,
        geometry,
    })
}

/// Converts a native v1 `.cloth` payload into the engine cloth authoring model.
///
/// # Errors
///
/// Returns an error when a native array, packed vertex, render mapping, asset
/// reference, or cloth invariant is invalid.
pub fn parse_cloth_fabric_source(
    bytes: &[u8],
) -> Result<ClothFabricSource, ClothFabricImportError> {
    let asset = parse_cloth_asset(bytes)?;
    let geometry = asset.geometry;
    let particle_count = usize::try_from(geometry.counts[0])
        .map_err(|_| ClothFabricImportError::CountOverflow("simulation vertices"))?;
    if particle_count != asset.internal.num_particles as usize {
        return Err(ClothFabricImportError::ParticleCountMismatch {
            fabric: asset.internal.num_particles,
            mesh: geometry.counts[0],
        });
    }

    let vertices = parse_simulation_vertices(block(
        bytes,
        geometry.offsets[0],
        particle_count,
        64,
        "simulation vertices",
    )?)?;
    let indices = decode_u32s(block(
        bytes,
        geometry.offsets[1],
        geometry.counts[1] as usize,
        4,
        "triangle indices",
    )?)?;
    let render_mapping = parse_render_mapping(bytes, geometry, particle_count)?;

    if geometry.flags[1] != 4 {
        return Err(ClothFabricImportError::UnsupportedInfluenceCount {
            count: geometry.flags[1],
        });
    }
    let paint = parse_paint_maps(bytes, geometry, particle_count)?;

    let render_model = canonical_skinned_mesh_path(read_c_string(
        bytes,
        geometry.offsets[6],
        "render skin path",
    )?)?;
    let material = canonical_cloth_material_path(read_c_string(
        bytes,
        geometry.offsets[7],
        "cloth material path",
    )?)?;
    let cooked = parse_cooked_data(&asset.internal)?;
    let fabric = ClothFabric {
        render_model,
        material: material.map_or(
            ClothMaterialBinding::Embedded(asset.material),
            ClothMaterialBinding::Asset,
        ),
        mesh: ClothMesh {
            vertices,
            indices,
            render_mapping,
            paint,
        },
        cooked,
    };
    fabric.validate()?;
    Ok(ClothFabricSource::new(fabric))
}

/// Reads the per-particle paint maps; the backstop maps are only present when
/// the geometry flags say so.
fn parse_paint_maps(
    bytes: &[u8],
    geometry: ClothGeometryLayout,
    particle_count: usize,
) -> Result<ClothPaintMaps, ClothFabricImportError> {
    let flags = geometry.flags[0];
    let motion_constraint_max_distances = decode_f32s(block(
        bytes,
        geometry.offsets[8],
        particle_count,
        4,
        "motion-constraint paint",
    )?)?;
    let backstop_offsets = (flags & CLOTH_HAS_BACKSTOP_OFFSETS != 0)
        .then(|| {
            block(
                bytes,
                geometry.offsets[9],
                particle_count,
                4,
                "backstop-offset paint",
            )
            .and_then(decode_f32s)
        })
        .transpose()?;
    let backstop_radii = (flags & CLOTH_HAS_BACKSTOP_RADII != 0)
        .then(|| {
            block(
                bytes,
                geometry.offsets[10],
                particle_count,
                4,
                "backstop-radius paint",
            )
            .and_then(decode_f32s)
        })
        .transpose()?;

    Ok(ClothPaintMaps {
        motion_constraint_max_distances,
        backstop_offsets,
        backstop_radii,
    })
}

/// Decodes the borrowed cooked-solver arrays into their owned engine form.
fn parse_cooked_data(
    internal: &FabricCookedData<'_>,
) -> Result<OwnedFabricCookedData, ClothFabricImportError> {
    Ok(OwnedFabricCookedData {
        phase_indices: decode_u32s(internal.phase_indices)?,
        phase_types: decode_i32s(internal.phase_types)?
            .into_iter()
            .map(FabricPhaseType::try_from)
            .collect::<Result<_, _>>()?,
        sets: decode_u32s(internal.sets)?,
        rest_values: decode_f32s(internal.rest_values)?,
        stiffness_values: decode_f32s(internal.stiffness_values)?,
        constraint_indices: decode_u32s(internal.indices)?,
        anchors: decode_u32s(internal.anchors)?,
        tether_lengths: decode_f32s(internal.tether_lengths)?,
        triangles: decode_u32s(internal.triangles)?,
    })
}

/// Borrowed `.cloth` asset.
#[derive(Debug)]
pub struct ClothAsset<'a> {
    pub header: ClothAssetHeader,
    pub material: ClothMaterialAsset,
    pub internal: FabricCookedData<'a>,
    pub geometry: ClothGeometryLayout,
}

/// `.cloth` asset header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClothAssetHeader {
    pub version: u32,
    pub flags: [u32; 3],
}

impl ClothAssetHeader {
    fn parse(bytes: &[u8]) -> Result<Self, ClothAssetParseError> {
        let version = read_u32(bytes, 0, "cloth version")?;
        if version != CLOTH_ASSET_VERSION {
            return Err(ClothAssetParseError::UnsupportedVersion { version });
        }
        Ok(Self {
            version,
            flags: [
                read_u32(bytes, 4, "cloth header flags[0]")?,
                read_u32(bytes, 8, "cloth header flags[1]")?,
                read_u32(bytes, 12, "cloth header flags[2]")?,
            ],
        })
    }
}

/// Borrowed cooked fabric arrays.
#[derive(Debug, Clone, Copy)]
pub struct FabricCookedData<'a> {
    pub num_particles: u32,
    pub phase_indices: &'a [u8],
    pub phase_types: &'a [u8],
    pub sets: &'a [u8],
    pub rest_values: &'a [u8],
    pub stiffness_values: &'a [u8],
    pub indices: &'a [u8],
    pub anchors: &'a [u8],
    pub tether_lengths: &'a [u8],
    pub triangles: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FabricCookedDataLayout {
    num_particles: u32,
    counts: [u32; FABRIC_INTERNAL_ARRAYS],
    offsets: [u64; FABRIC_INTERNAL_OFFSETS],
    geometry_counts: [u32; 4],
    geometry_offsets: [u64; FABRIC_GEOMETRY_OFFSETS],
    geometry_flags: [u32; 2],
}

impl FabricCookedDataLayout {
    fn parse(bytes: &[u8]) -> Result<Self, ClothAssetParseError> {
        let num_particles = read_u32(bytes, 0, "fabric particle count")?;
        let mut counts = [0; FABRIC_INTERNAL_ARRAYS];
        counts[0] = num_particles;
        for (index, count) in counts.iter_mut().enumerate().skip(1) {
            *count = read_u32(bytes, index * 4, "fabric internal count")?;
        }

        let mut offsets = [0; FABRIC_INTERNAL_OFFSETS];
        for (index, offset) in offsets.iter_mut().enumerate() {
            *offset = read_u64(bytes, 40 + index * 8, "fabric internal offset")?;
        }

        let geometry_base = 40 + FABRIC_INTERNAL_OFFSETS * 8;
        let geometry_counts = [
            read_u32(bytes, geometry_base, "cloth geometry count[0]")?,
            read_u32(bytes, geometry_base + 4, "cloth geometry count[1]")?,
            read_u32(bytes, geometry_base + 8, "cloth geometry count[2]")?,
            read_u32(bytes, geometry_base + 12, "cloth geometry count[3]")?,
        ];

        let mut geometry_offsets = [0; FABRIC_GEOMETRY_OFFSETS];
        let geometry_offsets_base = geometry_base + 16;
        for (index, offset) in geometry_offsets.iter_mut().enumerate() {
            *offset = read_u64(
                bytes,
                geometry_offsets_base + index * 8,
                "cloth geometry offset",
            )?;
        }

        let geometry_flags_base = geometry_offsets_base + FABRIC_GEOMETRY_OFFSETS * 8;
        let geometry_flags = [
            read_u32(bytes, geometry_flags_base, "cloth geometry flags[0]")?,
            read_u32(bytes, geometry_flags_base + 4, "cloth geometry flags[1]")?,
        ];

        Ok(Self {
            num_particles,
            counts,
            offsets,
            geometry_counts,
            geometry_offsets,
            geometry_flags,
        })
    }

    fn internal_data<'a>(
        &self,
        bytes: &'a [u8],
    ) -> Result<FabricCookedData<'a>, ClothAssetParseError> {
        Ok(FabricCookedData {
            num_particles: self.num_particles,
            phase_indices: self.range(bytes, 1, 4, "phase indices")?,
            phase_types: self.range(bytes, 2, 4, "phase types")?,
            sets: self.range(bytes, 3, 4, "sets")?,
            rest_values: self.range(bytes, 4, 4, "rest values")?,
            stiffness_values: self.range(bytes, 5, 4, "stiffness values")?,
            indices: self.range(bytes, 6, 4, "indices")?,
            anchors: self.range(bytes, 7, 4, "anchors")?,
            tether_lengths: self.range(bytes, 8, 4, "tether lengths")?,
            triangles: self.range(bytes, 9, 4, "triangles")?,
        })
    }

    fn range<'a>(
        &self,
        bytes: &'a [u8],
        index: usize,
        stride: usize,
        field: &'static str,
    ) -> Result<&'a [u8], ClothAssetParseError> {
        let count = self.counts[index] as usize;
        let offset = usize::try_from(self.offsets[index]).map_err(|_| {
            ClothAssetParseError::OutOfBounds {
                field,
                offset: self.offsets[index],
                len: bytes.len(),
            }
        })?;
        let len = count
            .checked_mul(stride)
            .ok_or(ClothAssetParseError::OutOfBounds {
                field,
                offset: self.offsets[index],
                len: bytes.len(),
            })?;
        slice(bytes, offset, len, field)
    }

    const fn geometry(&self) -> ClothGeometryLayout {
        ClothGeometryLayout {
            counts: self.geometry_counts,
            offsets: self.geometry_offsets,
            flags: self.geometry_flags,
        }
    }
}

/// Byte-offset layout for cloth geometry payload blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClothGeometryLayout {
    pub counts: [u32; 4],
    pub offsets: [u64; FABRIC_GEOMETRY_OFFSETS],
    pub flags: [u32; 2],
}

impl ClothGeometryLayout {
    fn validate(&self, file_len: usize) -> Result<(), ClothAssetParseError> {
        for &offset in &self.offsets {
            if offset == 0 {
                continue;
            }
            let offset_usize =
                usize::try_from(offset).map_err(|_| ClothAssetParseError::OutOfBounds {
                    field: "geometry",
                    offset,
                    len: file_len,
                })?;
            if offset_usize > file_len {
                return Err(ClothAssetParseError::OutOfBounds {
                    field: "geometry",
                    offset,
                    len: file_len,
                });
            }
        }
        Ok(())
    }
}

const CLOTH_BARYCENTRIC_MAPPING: u32 = 0x0100_0000;
const CLOTH_HAS_BACKSTOP_OFFSETS: u32 = 0x0000_0100;
const CLOTH_HAS_BACKSTOP_RADII: u32 = 0x0001_0000;

fn parse_simulation_vertices(
    bytes: &[u8],
) -> Result<Vec<ClothSimulationVertex>, ClothFabricImportError> {
    bytes
        .chunks_exact(64)
        .enumerate()
        .map(|(index, record)| {
            ensure_zero_padding(record, 12..16, "simulation vertex position", index)?;
            ensure_zero_padding(record, 40..48, "simulation vertex joints", index)?;
            ensure_zero_padding(record, 52..64, "simulation vertex weights", index)?;
            Ok(ClothSimulationVertex {
                position: Vec3::new(
                    read_f32_at(record, 0)?,
                    read_f32_at(record, 4)?,
                    read_f32_at(record, 8)?,
                ),
                tangent_frame: Quat::from_xyzw(
                    read_f32_at(record, 16)?,
                    read_f32_at(record, 20)?,
                    read_f32_at(record, 24)?,
                    read_f32_at(record, 28)?,
                ),
                joint_indices: [
                    read_u16_at(record, 32)?,
                    read_u16_at(record, 34)?,
                    read_u16_at(record, 36)?,
                    read_u16_at(record, 38)?,
                ],
                joint_weights: record[48..52].try_into().map_err(|_| {
                    ClothFabricImportError::InvalidRecord {
                        field: "simulation vertex weights",
                        index,
                    }
                })?,
            })
        })
        .collect()
}

fn parse_render_mapping(
    bytes: &[u8],
    geometry: ClothGeometryLayout,
    particle_count: usize,
) -> Result<ClothRenderMapping, ClothFabricImportError> {
    let flags = geometry.flags[0];
    if flags & CLOTH_BARYCENTRIC_MAPPING == 0 {
        let count = geometry.counts[2] as usize;
        return Ok(ClothRenderMapping::Direct {
            particle_indices: decode_u32s(block(
                bytes,
                geometry.offsets[2],
                count,
                4,
                "direct render mapping",
            )?)?,
        });
    }

    let range_count = geometry.counts[3] as usize;
    let range_bytes = block(
        bytes,
        geometry.offsets[3],
        range_count,
        16,
        "barycentric map ranges",
    )?;
    let mut ranges = Vec::with_capacity(range_count);
    for (index, record) in range_bytes.chunks_exact(16).enumerate() {
        ensure_zero_padding(record, 8..16, "barycentric map range", index)?;
        ranges.push(ClothSkinMapRange {
            first_vertex: read_u32_at(record, 0)?,
            vertex_count: read_u32_at(record, 4)?,
        });
    }
    let entry_count = ranges.last().map_or(particle_count, |range| {
        range.first_vertex as usize + range.vertex_count as usize
    });
    let entry_bytes = block(
        bytes,
        geometry.offsets[2],
        entry_count,
        32,
        "barycentric render mapping",
    )?;
    let mut entries = Vec::with_capacity(entry_count);
    for (index, record) in entry_bytes.chunks_exact(32).enumerate() {
        ensure_zero_padding(record, 20..32, "barycentric map entry", index)?;
        entries.push(ClothSkinMapEntry {
            barycentric: Vec3::new(
                read_f32_at(record, 0)?,
                read_f32_at(record, 4)?,
                read_f32_at(record, 8)?,
            ),
            height: read_f32_at(record, 12)?,
            triangle: read_u32_at(record, 16)?,
        });
    }
    Ok(ClothRenderMapping::Barycentric { entries, ranges })
}

fn canonical_skinned_mesh_path(value: &str) -> Result<AssetPathBuf, ClothFabricImportError> {
    let normalized = az_asset_builder::normalize_source_path(value);
    let Some(stem) = normalized.strip_suffix(".skin") else {
        return Err(ClothFabricImportError::UnsupportedReference {
            kind: "render skin",
            path: normalized,
        });
    };
    Ok(AssetPathBuf::new(format!("{stem}.skinnedmesh.glb"))?)
}

fn canonical_cloth_material_path(
    value: &str,
) -> Result<Option<AssetPathBuf>, ClothFabricImportError> {
    let normalized = az_asset_builder::normalize_source_path(value);
    let is_material_reference = std::path::Path::new(&normalized)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mtl"));
    if normalized.is_empty() || is_material_reference {
        return Ok(None);
    }
    let Some(stem) = normalized.strip_suffix(".clothmaterial") else {
        return Err(ClothFabricImportError::UnsupportedReference {
            kind: "cloth material",
            path: normalized,
        });
    };
    Ok(Some(AssetPathBuf::new(format!(
        "{stem}.clothmaterial.ron"
    ))?))
}

fn read_c_string<'a>(
    bytes: &'a [u8],
    offset: u64,
    field: &'static str,
) -> Result<&'a str, ClothFabricImportError> {
    let offset = usize::try_from(offset)
        .map_err(|_| ClothFabricImportError::InvalidStringOffset { field, offset })?;
    let tail = bytes
        .get(offset..)
        .ok_or(ClothFabricImportError::InvalidStringOffset {
            field,
            offset: offset as u64,
        })?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(ClothFabricImportError::UnterminatedString { field })?;
    std::str::from_utf8(&tail[..length])
        .map_err(|source| ClothFabricImportError::Utf8 { field, source })
}

fn block<'a>(
    bytes: &'a [u8],
    offset: u64,
    count: usize,
    stride: usize,
    field: &'static str,
) -> Result<&'a [u8], ClothFabricImportError> {
    let offset = usize::try_from(offset)
        .map_err(|_| ClothFabricImportError::BlockOutOfBounds { field, offset })?;
    let length = count
        .checked_mul(stride)
        .ok_or(ClothFabricImportError::CountOverflow(field))?;
    slice(bytes, offset, length, field).map_err(ClothFabricImportError::Asset)
}

fn decode_u32s(bytes: &[u8]) -> Result<Vec<u32>, ClothFabricImportError> {
    bytes
        .chunks_exact(4)
        .map(|value| {
            value
                .try_into()
                .map(u32::from_le_bytes)
                .map_err(|_| ClothFabricImportError::InvalidScalarArray)
        })
        .collect()
}

fn decode_i32s(bytes: &[u8]) -> Result<Vec<i32>, ClothFabricImportError> {
    bytes
        .chunks_exact(4)
        .map(|value| {
            value
                .try_into()
                .map(i32::from_le_bytes)
                .map_err(|_| ClothFabricImportError::InvalidScalarArray)
        })
        .collect()
}

fn decode_f32s(bytes: &[u8]) -> Result<Vec<f32>, ClothFabricImportError> {
    bytes
        .chunks_exact(4)
        .map(|value| {
            value
                .try_into()
                .map(f32::from_le_bytes)
                .map_err(|_| ClothFabricImportError::InvalidScalarArray)
        })
        .collect()
}

fn read_u16_at(bytes: &[u8], offset: usize) -> Result<u16, ClothFabricImportError> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or(ClothFabricImportError::InvalidScalarArray)
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, ClothFabricImportError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(ClothFabricImportError::InvalidScalarArray)
}

fn read_f32_at(bytes: &[u8], offset: usize) -> Result<f32, ClothFabricImportError> {
    read_u32_at(bytes, offset).map(f32::from_bits)
}

fn ensure_zero_padding(
    bytes: &[u8],
    range: std::ops::Range<usize>,
    field: &'static str,
    index: usize,
) -> Result<(), ClothFabricImportError> {
    if bytes
        .get(range)
        .is_some_and(|padding| padding.iter().all(|byte| *byte == 0))
    {
        Ok(())
    } else {
        Err(ClothFabricImportError::NonZeroPadding { field, index })
    }
}

#[derive(Debug, Error)]
pub enum ClothFabricImportError {
    #[error(transparent)]
    Asset(#[from] ClothAssetParseError),
    #[error(transparent)]
    Validation(#[from] az_nv_cloth::ClothValidationError),
    #[error(transparent)]
    AssetPath(#[from] AssetPathError),
    #[error("cloth {0} count cannot be represented")]
    CountOverflow(&'static str),
    #[error("cooked fabric has {fabric} particles but the mesh has {mesh}")]
    ParticleCountMismatch { fabric: u32, mesh: u32 },
    #[error("cloth {field} block at {offset:#x} is outside the file")]
    BlockOutOfBounds { field: &'static str, offset: u64 },
    #[error("cloth {field} record {index} is malformed")]
    InvalidRecord { field: &'static str, index: usize },
    #[error("cloth {field} record {index} contains non-zero padding")]
    NonZeroPadding { field: &'static str, index: usize },
    #[error("cloth scalar array is malformed")]
    InvalidScalarArray,
    #[error("cloth {field} string offset {offset:#x} is outside the file")]
    InvalidStringOffset { field: &'static str, offset: u64 },
    #[error("cloth {field} string is not terminated")]
    UnterminatedString { field: &'static str },
    #[error("cloth {field} string is not UTF-8: {source}")]
    Utf8 {
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("unsupported {kind} reference {path}")]
    UnsupportedReference { kind: &'static str, path: String },
    #[error(
        "cloth asset uses {count} skin influences; only the native four-influence layout is supported"
    )]
    UnsupportedInfluenceCount { count: u32 },
}

#[derive(Debug, Error, PartialEq)]
pub enum ClothMaterialParseError {
    #[error("invalid cloth material size: expected {expected} bytes, got {actual}")]
    InvalidSize { actual: usize, expected: usize },
    #[error("{field} vector padding is {value}, expected 0")]
    NonZeroVectorPadding { field: &'static str, value: f32 },
    #[error("{field} boolean is {value}, expected 0 or 1")]
    InvalidBoolean { field: &'static str, value: u32 },
}

#[derive(Debug, Error, PartialEq)]
pub enum ClothAssetParseError {
    #[error("unexpected end of file while reading {context}")]
    UnexpectedEof { context: &'static str },
    #[error("unsupported cloth asset version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("{field} points outside the cloth asset: offset {offset:#x}, file length {len:#x}")]
    OutOfBounds {
        field: &'static str,
        offset: u64,
        len: usize,
    },
    #[error(transparent)]
    Material(#[from] ClothMaterialParseError),
}

fn read_u32(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<u32, ClothAssetParseError> {
    Ok(u32::from_le_bytes(read_array(bytes, offset, context)?))
}

fn read_u64(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<u64, ClothAssetParseError> {
    Ok(u64::from_le_bytes(read_array(bytes, offset, context)?))
}

fn read_array<const N: usize>(
    bytes: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<[u8; N], ClothAssetParseError> {
    bytes
        .get(offset..offset + N)
        .ok_or(ClothAssetParseError::UnexpectedEof { context })?
        .try_into()
        .map_err(|_| ClothAssetParseError::UnexpectedEof { context })
}

fn slice<'a>(
    bytes: &'a [u8],
    offset: usize,
    len: usize,
    field: &'static str,
) -> Result<&'a [u8], ClothAssetParseError> {
    let end = offset
        .checked_add(len)
        .ok_or(ClothAssetParseError::OutOfBounds {
            field,
            offset: offset as u64,
            len: bytes.len(),
        })?;
    bytes
        .get(offset..end)
        .ok_or(ClothAssetParseError::OutOfBounds {
            field,
            offset: offset as u64,
            len: bytes.len(),
        })
}

struct FloatReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> FloatReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_phase_config(&mut self) -> PhaseConfig {
        PhaseConfig::new(
            self.read_f32(),
            self.read_f32(),
            self.read_f32(),
            self.read_f32(),
        )
    }

    fn read_padded_vec3(&mut self, field: &'static str) -> Result<Vec3, ClothMaterialParseError> {
        let value = Vec3::new(self.read_f32(), self.read_f32(), self.read_f32());
        let padding = self.read_f32();
        if padding != 0.0 {
            return Err(ClothMaterialParseError::NonZeroVectorPadding {
                field,
                value: padding,
            });
        }
        Ok(value)
    }

    fn read_f32(&mut self) -> f32 {
        let value = f32::from_le_bytes(
            self.bytes[self.offset..self.offset + 4]
                .try_into()
                .expect("cloth material parser bounds were validated up front"),
        );
        self.offset += 4;
        value
    }

    fn read_bool_u32(&mut self, field: &'static str) -> Result<bool, ClothMaterialParseError> {
        let value = u32::from_le_bytes(
            self.bytes[self.offset..self.offset + 4]
                .try_into()
                .expect("material size was validated"),
        );
        self.offset += 4;
        match value {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ClothMaterialParseError::InvalidBoolean { field, value }),
        }
    }
}

fn write_phase_config(bytes: &mut Vec<u8>, config: PhaseConfig) {
    write_f32(bytes, config.stiffness);
    write_f32(bytes, config.stiffness_multiplier);
    write_f32(bytes, config.compression_limit);
    write_f32(bytes, config.stretch_limit);
}

fn write_padded_vec3(bytes: &mut Vec<u8>, value: Vec3) {
    write_f32(bytes, value.x);
    write_f32(bytes, value.y);
    write_f32(bytes, value.z);
    write_f32(bytes, 0.0);
}

fn write_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend(value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cloth_material_block() {
        let material = parse_cloth_material(&continuous_collision_material_bytes()).unwrap();

        assert_exact(material.phase_configs.horizontal.stiffness, 1.0);
        assert_exact(material.phase_configs.horizontal.stiffness_multiplier, 2.0);
        assert_exact(material.stiffness_frequency, 50.0);
        assert_exact(material.motion_constraints.max_distance, 0.5);
        assert_exact(material.self_collision.distance, 0.0);
        assert_exact(material.tether_constraints.stiffness, 1.0);
        assert_exact(material.solver_frequency, 120.0);
        assert_exact(material.acceleration_filter_width, 30.0);
        assert!(material.continuous_collision);
        assert_eq!(material.damping, Vec3::splat(0.1));
        assert_eq!(material.linear_inertia, Vec3::ONE);
    }

    #[test]
    fn summarizes_and_inspects_cloth_material_block() {
        let bytes = continuous_collision_material_bytes();
        let material = parse_cloth_material(&bytes).unwrap();

        let summary = ClothSummary::from_material(&material);
        assert_eq!(summary.kind, ClothSummaryKind::Material);
        assert_exact(summary.stiffness_frequency, 50.0);
        assert_exact(summary.motion_max_distance, 0.5);
        assert_eq!(
            summary.to_string(),
            "ClothMaterialAsset, stiffness=50 solver=120 damping=(0.1, 0.1, 0.1) linear_inertia=(1, 1, 1)"
        );

        let mut totals = ClothMaterialTotals::default();
        totals.add_summary(summary);
        assert_eq!(totals.files, 1);
        assert_eq!(
            totals.to_string(),
            "  files: 1
  stiffness frequency: min=50 max=50 avg=50
  solver frequency: min=120 max=120 avg=120
  continuous collision: 1 files
  max motion distance range: min=0.5 max=0.5
"
        );

        let row = inspect_cloth_file("cloth/default.clothmaterial", &bytes).unwrap();
        let mut inspection = ClothInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "cloth/default.clothmaterial: ClothMaterialAsset, stiffness=50 solver=120 damping=(0.1, 0.1, 0.1) linear_inertia=(1, 1, 1)
  files: 1
  stiffness frequency: min=50 max=50 avg=50
  solver frequency: min=120 max=120 avg=120
  continuous collision: 1 files
  max motion distance range: min=0.5 max=0.5
"
        );
        assert!(is_cloth_material_name("default.CLOTHMATERIAL"));

        let path = std::env::temp_dir().join(format!(
            "az-rs-nv-cloth-{}-default.clothmaterial",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).expect("write cloth material");
        let inspection = inspect_cloth_files([&path]).expect("inspect cloth files");
        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.stiffness_frequency.count, 1);
        std::fs::remove_file(path).expect("remove cloth material");
    }

    /// Compares a round-tripped `f32` bit-exactly.
    ///
    /// The fixture writes the same little-endian pattern the parser reads
    /// back, so any difference is a decode bug rather than accumulated error;
    /// an epsilon window would hide exactly the bugs this asserts against.
    #[track_caller]
    fn assert_exact(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    /// The shared fixture with `continuous_collision` set, written as the
    /// native `u32` boolean `1` in the float slot the parser reads.
    fn continuous_collision_material_bytes() -> Vec<u8> {
        let mut bytes = cloth_material_bytes();
        bytes[27 * 4..28 * 4].copy_from_slice(&1_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn rejects_wrong_size() {
        assert_eq!(
            parse_cloth_material(&[]).unwrap_err(),
            ClothMaterialParseError::InvalidSize {
                actual: 0,
                expected: CLOTH_MATERIAL_SIZE,
            }
        );
    }

    #[test]
    fn writes_cloth_material_block_roundtrip() {
        let bytes = cloth_material_bytes();
        let material = parse_cloth_material(&bytes).unwrap();

        let mut written = Vec::new();
        write_cloth_material(&material, &mut written).unwrap();

        assert_eq!(written, bytes);
        assert_eq!(parse_cloth_material(&written).unwrap(), material);
    }

    #[test]
    fn parses_cloth_asset_layout() {
        let mut bytes = Vec::new();
        bytes.extend(CLOTH_ASSET_VERSION.to_le_bytes());
        bytes.extend([0; 12]);
        bytes.extend(cloth_material_bytes());

        let layout_offset = bytes.len();
        bytes.resize(layout_offset + FABRIC_LAYOUT_SIZE, 0);
        let phase_indices_offset = bytes.len() as u64;
        bytes.extend([1_u32.to_le_bytes(), 2_u32.to_le_bytes()].concat());
        let phase_types_offset = bytes.len() as u64;
        bytes.extend([3_i32.to_le_bytes(), 4_i32.to_le_bytes()].concat());
        let sets_offset = bytes.len() as u64;
        bytes.extend([5_u32.to_le_bytes(), 6_u32.to_le_bytes()].concat());
        let rest_values_offset = bytes.len() as u64;
        bytes.extend([0.25_f32.to_le_bytes(), 0.5_f32.to_le_bytes()].concat());
        let indices_offset = bytes.len() as u64;
        bytes.extend(
            [
                0_u32.to_le_bytes(),
                1_u32.to_le_bytes(),
                2_u32.to_le_bytes(),
            ]
            .concat(),
        );
        let anchors_offset = bytes.len() as u64;
        bytes.extend([7_u32.to_le_bytes()].concat());
        let tether_lengths_offset = bytes.len() as u64;
        bytes.extend([1.0_f32.to_le_bytes()].concat());
        let triangles_offset = bytes.len() as u64;
        bytes.extend(
            [
                0_u32.to_le_bytes(),
                1_u32.to_le_bytes(),
                2_u32.to_le_bytes(),
            ]
            .concat(),
        );

        let layout = &mut bytes[layout_offset..layout_offset + FABRIC_LAYOUT_SIZE];
        for (index, count) in [3_u32, 2, 2, 2, 2, 0, 3, 1, 1, 3].into_iter().enumerate() {
            layout[index * 4..index * 4 + 4].copy_from_slice(&count.to_le_bytes());
        }
        for (index, offset) in [
            0,
            phase_indices_offset,
            phase_types_offset,
            sets_offset,
            rest_values_offset,
            rest_values_offset,
            indices_offset,
            anchors_offset,
            tether_lengths_offset,
            triangles_offset,
            0,
        ]
        .into_iter()
        .enumerate()
        {
            layout[40 + index * 8..40 + index * 8 + 8].copy_from_slice(&offset.to_le_bytes());
        }

        let asset = parse_cloth_asset(&bytes).unwrap();

        assert_eq!(asset.header.version, CLOTH_ASSET_VERSION);
        assert_eq!(asset.internal.num_particles, 3);
        assert_eq!(asset.internal.phase_indices.len(), 8);
        assert_eq!(asset.internal.stiffness_values.len(), 0);
        assert_eq!(asset.internal.triangles.len(), 12);

        let summary = ClothSummary::from_asset(&asset);
        assert_eq!(
            summary.kind,
            ClothSummaryKind::Asset {
                particles: 3,
                triangle_indices: 3
            }
        );
        assert_eq!(
            summary.to_string(),
            "ClothAsset, 3 particles, 3 triangle indices, stiffness=50 solver=120"
        );
        assert!(is_cloth_asset_name("cape.CLOTH"));
    }

    fn cloth_material_bytes() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CLOTH_MATERIAL_SIZE);
        for value in [
            1.0_f32, 2.0, 0.5, 1.0, 1.0, 2.0, 0.5, 1.0, 1.0, 2.0, 0.5, 1.0, 1.0, 2.0, 0.5, 1.0,
            50.0, 0.5, 0.5, 0.5, 1.0, 0.0, 0.0, 1.0, 1.0, 120.0, 30.0, 0.0, 0.1, 0.1, 0.1, 0.0,
            0.1, 0.1, 0.1, 0.0, 0.1, 0.1, 0.1, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0,
            1.0, 1.0, 0.0,
        ] {
            bytes.extend(value.to_le_bytes());
        }
        bytes
    }
}
