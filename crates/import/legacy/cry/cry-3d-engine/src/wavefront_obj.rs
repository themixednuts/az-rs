use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use bevy::math::{Vec2, Vec3};
use smallvec::SmallVec;
use thiserror::Error;

pub const WAVEFRONT_OBJ_EXTENSION: &str = "obj";

/// Wavefront OBJ terrain footprint mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct WavefrontObj<'a> {
    positions: Vec<Position>,
    texcoords: Vec<TexCoord>,
    normals: Vec<Vec3>,
    faces: Vec<Face>,
    groups: Vec<NamedRange<'a>>,
    material_libraries: Vec<&'a str>,
    material_uses: Vec<NamedRange<'a>>,
    smoothing: Vec<SmoothingRange<'a>>,
    comments: usize,
}

impl<'a> WavefrontObj<'a> {
    /// Parse a Wavefront OBJ text payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is not UTF-8, uses an unsupported
    /// directive, has malformed numeric data, or references an out-of-range
    /// face index.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, WavefrontObjError> {
        let text = std::str::from_utf8(bytes)?;
        Self::parse_str(text)
    }

    /// Parse a Wavefront OBJ string.
    ///
    /// # Errors
    ///
    /// Returns an error when the file uses an unsupported directive, has
    /// malformed numeric data, or references an out-of-range face index.
    pub fn parse_str(text: &'a str) -> Result<Self, WavefrontObjError> {
        let mut obj = Self {
            positions: Vec::new(),
            texcoords: Vec::new(),
            normals: Vec::new(),
            faces: Vec::new(),
            groups: Vec::new(),
            material_libraries: Vec::new(),
            material_uses: Vec::new(),
            smoothing: Vec::new(),
            comments: 0,
        };

        for (line_index, raw_line) in text.lines().enumerate() {
            let line_number = line_index + 1;
            if raw_line.trim_start().starts_with('#') {
                obj.comments += 1;
            }

            let Some((directive, rest)) = directive_and_rest(raw_line) else {
                continue;
            };
            match directive {
                "v" => obj.positions.push(parse_position(line_number, rest)?),
                "vt" => obj.texcoords.push(parse_texcoord(line_number, rest)?),
                "vn" => obj.normals.push(parse_normal(line_number, rest)?),
                "f" => {
                    let face = parse_face(
                        line_number,
                        rest,
                        obj.positions.len(),
                        obj.texcoords.len(),
                        obj.normals.len(),
                    )?;
                    obj.faces.push(face);
                }
                "g" => obj.groups.push(NamedRange::new(rest, obj.faces.len())),
                "mtllib" => {
                    for library in rest.split_whitespace() {
                        obj.material_libraries.push(library);
                    }
                }
                "usemtl" => obj
                    .material_uses
                    .push(NamedRange::new(rest, obj.faces.len())),
                "s" => obj.smoothing.push(SmoothingRange::new(
                    SmoothingGroup::parse(rest),
                    obj.faces.len(),
                )),
                other => {
                    return Err(WavefrontObjError::UnsupportedDirective {
                        line: line_number,
                        directive: other.into(),
                    });
                }
            }
        }

        Ok(obj)
    }

    #[must_use]
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    #[must_use]
    pub fn texcoords(&self) -> &[TexCoord] {
        &self.texcoords
    }

    #[must_use]
    pub fn normals(&self) -> &[Vec3] {
        &self.normals
    }

    #[must_use]
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    #[must_use]
    pub fn groups(&self) -> &[NamedRange<'a>] {
        &self.groups
    }

    #[must_use]
    pub fn material_libraries(&self) -> &[&'a str] {
        &self.material_libraries
    }

    #[must_use]
    pub fn material_uses(&self) -> &[NamedRange<'a>] {
        &self.material_uses
    }

    #[must_use]
    pub fn smoothing(&self) -> &[SmoothingRange<'a>] {
        &self.smoothing
    }

    #[must_use]
    pub const fn comments(&self) -> usize {
        self.comments
    }

    #[must_use]
    pub fn summary(&self) -> WavefrontObjSummary {
        WavefrontObjSummary::from_obj(self)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WavefrontObjSummary {
    pub positions: usize,
    pub texcoords: usize,
    pub normals: usize,
    pub faces: usize,
    pub max_face_vertices: usize,
    pub groups: usize,
    pub material_libraries: usize,
    pub material_uses: usize,
    pub smoothing_ranges: usize,
    pub comments: usize,
}

impl WavefrontObjSummary {
    #[must_use]
    pub fn from_obj(obj: &WavefrontObj<'_>) -> Self {
        Self {
            positions: obj.positions().len(),
            texcoords: obj.texcoords().len(),
            normals: obj.normals().len(),
            faces: obj.faces().len(),
            max_face_vertices: obj
                .faces()
                .iter()
                .map(|face| face.vertices().len())
                .max()
                .unwrap_or(0),
            groups: obj.groups().len(),
            material_libraries: obj.material_libraries().len(),
            material_uses: obj.material_uses().len(),
            smoothing_ranges: obj.smoothing().len(),
            comments: obj.comments(),
        }
    }
}

impl fmt::Display for WavefrontObjSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} positions, {} texcoords, {} normals, {} faces, {} groups, {} material uses",
            self.positions,
            self.texcoords,
            self.normals,
            self.faces,
            self.groups,
            self.material_uses
        )
    }
}

/// Parse `bytes` and reduce the mesh to its element counts.
///
/// # Errors
///
/// Returns any error [`WavefrontObj::parse`] returns.
pub fn summarize_wavefront_obj(bytes: &[u8]) -> Result<WavefrontObjSummary, WavefrontObjError> {
    WavefrontObj::parse(bytes).map(|obj| obj.summary())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WavefrontObjTotals {
    pub files: usize,
    pub positions: usize,
    pub texcoords: usize,
    pub normals: usize,
    pub faces: usize,
    pub max_face_vertices: usize,
    pub groups: usize,
    pub material_libraries: usize,
    pub material_uses: usize,
    pub smoothing_ranges: usize,
    pub comments: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavefrontObjFileSummary {
    pub source: String,
    pub summary: WavefrontObjSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WavefrontObjInspection {
    pub rows: Vec<WavefrontObjFileSummary>,
    pub totals: WavefrontObjTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct WavefrontObjInspectionReport<'a> {
    inspection: &'a WavefrontObjInspection,
    limit: usize,
}

impl WavefrontObjTotals {
    pub fn add_summary(&mut self, summary: WavefrontObjSummary) {
        self.files += 1;
        self.positions += summary.positions;
        self.texcoords += summary.texcoords;
        self.normals += summary.normals;
        self.faces += summary.faces;
        self.max_face_vertices = self.max_face_vertices.max(summary.max_face_vertices);
        self.groups += summary.groups;
        self.material_libraries += summary.material_libraries;
        self.material_uses += summary.material_uses;
        self.smoothing_ranges += summary.smoothing_ranges;
        self.comments += summary.comments;
    }
}

impl WavefrontObjInspection {
    pub fn add_file_summary(&mut self, row: WavefrontObjFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> WavefrontObjInspectionReport<'_> {
        WavefrontObjInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for WavefrontObjTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  positions: {}", self.positions)?;
        writeln!(f, "  texcoords: {}", self.texcoords)?;
        writeln!(f, "  normals: {}", self.normals)?;
        writeln!(f, "  faces: {}", self.faces)?;
        writeln!(f, "  max face vertices: {}", self.max_face_vertices)?;
        writeln!(f, "  groups: {}", self.groups)?;
        writeln!(f, "  material libraries: {}", self.material_libraries)?;
        writeln!(f, "  material uses: {}", self.material_uses)?;
        writeln!(f, "  smoothing ranges: {}", self.smoothing_ranges)
    }
}

impl fmt::Display for WavefrontObjInspectionReport<'_> {
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

/// Summarize `bytes` into a one-row inspection record naming `path`.
///
/// # Errors
///
/// Returns any error [`summarize_wavefront_obj`] returns.
pub fn inspect_wavefront_obj_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<WavefrontObjFileSummary, WavefrontObjError> {
    Ok(WavefrontObjFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_wavefront_obj(bytes)?,
    })
}

#[derive(Debug, Error)]
pub enum WavefrontObjInspectionError {
    #[error("read Wavefront OBJ {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse Wavefront OBJ {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: WavefrontObjError,
    },
}

/// Read `path` from disk and inspect it as a Wavefront OBJ mesh.
///
/// # Errors
///
/// Returns [`WavefrontObjInspectionError::Read`] when `path` cannot be read,
/// or [`WavefrontObjInspectionError::Parse`] when its text is not a valid OBJ.
pub fn inspect_wavefront_obj_path(
    path: impl AsRef<Path>,
) -> Result<WavefrontObjFileSummary, WavefrontObjInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| WavefrontObjInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_wavefront_obj_file(path, &bytes).map_err(|source| WavefrontObjInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Inspect every path in `paths`, accumulating per-element totals.
///
/// # Errors
///
/// Returns the first error [`inspect_wavefront_obj_path`] returns; remaining
/// paths are not visited.
pub fn inspect_wavefront_obj_files(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<WavefrontObjInspection, WavefrontObjInspectionError> {
    let mut inspection = WavefrontObjInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_wavefront_obj_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_wavefront_obj_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(WAVEFRONT_OBJ_EXTENSION)
}

#[must_use]
pub fn is_wavefront_obj_name(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_wavefront_obj_extension)
}

#[must_use]
pub fn is_wavefront_obj_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_wavefront_obj_extension)
}

/// OBJ position vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub xyz: Vec3,
    pub w: Option<f32>,
}

impl Position {
    #[must_use]
    pub const fn new(xyz: Vec3, w: Option<f32>) -> Self {
        Self { xyz, w }
    }
}

/// OBJ texture coordinate vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexCoord {
    pub uv: Vec2,
    pub w: Option<f32>,
}

impl TexCoord {
    #[must_use]
    pub const fn new(uv: Vec2, w: Option<f32>) -> Self {
        Self { uv, w }
    }
}

/// One OBJ polygon face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    vertices: SmallVec<[FaceVertex; 4]>,
}

impl Face {
    #[must_use]
    pub fn vertices(&self) -> &[FaceVertex] {
        &self.vertices
    }
}

/// Zero-based index triplet for one corner of a face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceVertex {
    pub position: u32,
    pub texcoord: Option<u32>,
    pub normal: Option<u32>,
}

impl FaceVertex {
    #[must_use]
    pub const fn new(position: u32, texcoord: Option<u32>, normal: Option<u32>) -> Self {
        Self {
            position,
            texcoord,
            normal,
        }
    }
}

/// A named OBJ range starting at `face_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NamedRange<'a> {
    pub name: &'a str,
    pub face_start: usize,
}

impl<'a> NamedRange<'a> {
    #[must_use]
    pub const fn new(name: &'a str, face_start: usize) -> Self {
        Self { name, face_start }
    }
}

/// OBJ smoothing-group state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmoothingGroup<'a> {
    Off,
    Name(&'a str),
}

impl<'a> SmoothingGroup<'a> {
    #[must_use]
    pub fn parse(value: &'a str) -> Self {
        if value.eq_ignore_ascii_case("off") || value == "0" {
            Self::Off
        } else {
            Self::Name(value)
        }
    }
}

/// A smoothing-group range starting at `face_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SmoothingRange<'a> {
    pub group: SmoothingGroup<'a>,
    pub face_start: usize,
}

impl<'a> SmoothingRange<'a> {
    #[must_use]
    pub const fn new(group: SmoothingGroup<'a>, face_start: usize) -> Self {
        Self { group, face_start }
    }
}

#[derive(Debug, Error)]
pub enum WavefrontObjError {
    #[error("OBJ text is not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("unsupported OBJ directive `{directive}` at line {line}")]
    UnsupportedDirective { line: usize, directive: Box<str> },

    #[error("missing {field} at line {line}")]
    MissingField { line: usize, field: &'static str },

    #[error("invalid {field} `{value}` at line {line}")]
    InvalidNumber {
        line: usize,
        field: &'static str,
        value: Box<str>,
    },

    #[error("extra data after {directive} at line {line}: `{value}`")]
    ExtraData {
        line: usize,
        directive: &'static str,
        value: Box<str>,
    },

    #[error("face at line {line} has {count} vertices, expected at least 3")]
    FaceTooSmall { line: usize, count: usize },

    #[error("empty face index at line {line}")]
    EmptyFaceIndex { line: usize },

    #[error("face index {value} for {component} at line {line} is zero")]
    ZeroIndex {
        line: usize,
        component: &'static str,
        value: i32,
    },

    #[error(
        "face index {value} for {component} at line {line} is out of range for {available} entries"
    )]
    IndexOutOfRange {
        line: usize,
        component: &'static str,
        value: i32,
        available: usize,
    },

    #[error("face vertex `{value}` at line {line} has too many index components")]
    TooManyFaceComponents { line: usize, value: Box<str> },
}

fn directive_and_rest(line: &str) -> Option<(&str, &str)> {
    let line = line
        .split_once('#')
        .map_or(line, |(before, _)| before)
        .trim();
    if line.is_empty() {
        return None;
    }

    let first_space = line
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index));
    Some(first_space.map_or((line, ""), |index| (&line[..index], line[index..].trim())))
}

fn parse_position(line: usize, rest: &str) -> Result<Position, WavefrontObjError> {
    let mut tokens = rest.split_whitespace();
    let x = parse_required_f32(line, "x", &mut tokens)?;
    let y = parse_required_f32(line, "y", &mut tokens)?;
    let z = parse_required_f32(line, "z", &mut tokens)?;
    let w = parse_optional_f32(line, "w", &mut tokens)?;
    reject_extra(line, "v", &mut tokens)?;
    Ok(Position::new(Vec3::new(x, y, z), w))
}

fn parse_texcoord(line: usize, rest: &str) -> Result<TexCoord, WavefrontObjError> {
    let mut tokens = rest.split_whitespace();
    let u = parse_required_f32(line, "u", &mut tokens)?;
    let v = parse_required_f32(line, "v", &mut tokens)?;
    let w = parse_optional_f32(line, "w", &mut tokens)?;
    reject_extra(line, "vt", &mut tokens)?;
    Ok(TexCoord::new(Vec2::new(u, v), w))
}

fn parse_normal(line: usize, rest: &str) -> Result<Vec3, WavefrontObjError> {
    let mut tokens = rest.split_whitespace();
    let x = parse_required_f32(line, "x", &mut tokens)?;
    let y = parse_required_f32(line, "y", &mut tokens)?;
    let z = parse_required_f32(line, "z", &mut tokens)?;
    reject_extra(line, "vn", &mut tokens)?;
    Ok(Vec3::new(x, y, z))
}

fn parse_required_f32<'a>(
    line: usize,
    field: &'static str,
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<f32, WavefrontObjError> {
    let value = tokens
        .next()
        .ok_or(WavefrontObjError::MissingField { line, field })?;
    parse_f32(line, field, value)
}

fn parse_optional_f32<'a>(
    line: usize,
    field: &'static str,
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Option<f32>, WavefrontObjError> {
    tokens
        .next()
        .map(|value| parse_f32(line, field, value))
        .transpose()
}

fn parse_f32(line: usize, field: &'static str, value: &str) -> Result<f32, WavefrontObjError> {
    value.parse().map_err(|_| WavefrontObjError::InvalidNumber {
        line,
        field,
        value: value.into(),
    })
}

fn reject_extra<'a>(
    line: usize,
    directive: &'static str,
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<(), WavefrontObjError> {
    if let Some(value) = tokens.next() {
        return Err(WavefrontObjError::ExtraData {
            line,
            directive,
            value: value.into(),
        });
    }
    Ok(())
}

fn parse_face(
    line: usize,
    rest: &str,
    positions: usize,
    texcoords: usize,
    normals: usize,
) -> Result<Face, WavefrontObjError> {
    let mut vertices = SmallVec::new();
    for token in rest.split_whitespace() {
        vertices.push(parse_face_vertex(
            line, token, positions, texcoords, normals,
        )?);
    }
    if vertices.len() < 3 {
        return Err(WavefrontObjError::FaceTooSmall {
            line,
            count: vertices.len(),
        });
    }
    Ok(Face { vertices })
}

fn parse_face_vertex(
    line: usize,
    token: &str,
    positions: usize,
    texcoords: usize,
    normals: usize,
) -> Result<FaceVertex, WavefrontObjError> {
    let mut parts = token.split('/');
    let position = parts.next().unwrap_or_default();
    let texcoord = parts.next();
    let normal = parts.next();
    if parts.next().is_some() {
        return Err(WavefrontObjError::TooManyFaceComponents {
            line,
            value: token.into(),
        });
    }

    Ok(FaceVertex::new(
        parse_index(line, "position", position, positions)?,
        parse_optional_index(line, "texcoord", texcoord, texcoords)?,
        parse_optional_index(line, "normal", normal, normals)?,
    ))
}

fn parse_optional_index(
    line: usize,
    component: &'static str,
    value: Option<&str>,
    available: usize,
) -> Result<Option<u32>, WavefrontObjError> {
    match value {
        Some(value) if !value.is_empty() => {
            parse_index(line, component, value, available).map(Some)
        }
        _ => Ok(None),
    }
}

fn parse_index(
    line: usize,
    component: &'static str,
    value: &str,
    available: usize,
) -> Result<u32, WavefrontObjError> {
    if value.is_empty() {
        return Err(WavefrontObjError::EmptyFaceIndex { line });
    }

    let raw = value
        .parse::<i32>()
        .map_err(|_| WavefrontObjError::InvalidNumber {
            line,
            field: component,
            value: value.into(),
        })?;
    if raw == 0 {
        return Err(WavefrontObjError::ZeroIndex {
            line,
            component,
            value: raw,
        });
    }

    let out_of_range = || WavefrontObjError::IndexOutOfRange {
        line,
        component,
        value: raw,
        available,
    };

    // A positive index is 1-based from the start of the element list; a
    // negative index counts back from the most recently parsed element.
    let resolved: usize = if raw > 0 {
        let forward = (raw.unsigned_abs() - 1) as usize;
        if forward >= available {
            return Err(out_of_range());
        }
        forward
    } else {
        available
            .checked_sub(raw.unsigned_abs() as usize)
            .ok_or_else(out_of_range)?
    };

    u32::try_from(resolved).map_err(|_| out_of_range())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wavefront_obj_mesh() {
        let obj = WavefrontObj::parse_str(
            r"# units centimeters
mtllib terrain.mtl
g terrain
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vt 0.0 0.0
vt 1.0 0.0
vt 0.0 1.0
vn 0.0 0.0 1.0
usemtl grey
s 1
f 1/1/1 2/2/1 3/3/1
",
        )
        .unwrap();

        assert_eq!(obj.comments(), 1);
        assert_eq!(obj.positions().len(), 3);
        assert_eq!(obj.texcoords().len(), 3);
        assert_eq!(obj.normals(), &[Vec3::Z]);
        assert_eq!(obj.material_libraries(), &["terrain.mtl"]);
        assert_eq!(obj.groups(), &[NamedRange::new("terrain", 0)]);
        assert_eq!(obj.material_uses(), &[NamedRange::new("grey", 0)]);
        assert_eq!(
            obj.smoothing(),
            &[SmoothingRange::new(SmoothingGroup::Name("1"), 0)]
        );
        assert_eq!(
            obj.faces()[0].vertices(),
            &[
                FaceVertex::new(0, Some(0), Some(0)),
                FaceVertex::new(1, Some(1), Some(0)),
                FaceVertex::new(2, Some(2), Some(0)),
            ]
        );
        assert_eq!(
            obj.summary(),
            WavefrontObjSummary {
                positions: 3,
                texcoords: 3,
                normals: 1,
                faces: 1,
                max_face_vertices: 3,
                groups: 1,
                material_libraries: 1,
                material_uses: 1,
                smoothing_ranges: 1,
                comments: 1,
            }
        );
    }

    #[test]
    fn resolves_negative_face_indices() {
        let obj = WavefrontObj::parse_str(
            r"v 0 0 0
v 1 0 0
v 0 1 0
f -3 -2 -1
",
        )
        .unwrap();

        assert_eq!(
            obj.faces()[0].vertices(),
            &[
                FaceVertex::new(0, None, None),
                FaceVertex::new(1, None, None),
                FaceVertex::new(2, None, None),
            ]
        );
    }

    #[test]
    fn tracks_wavefront_obj_totals_and_paths() {
        let mut totals = WavefrontObjTotals::default();
        totals.add_summary(WavefrontObjSummary {
            positions: 1,
            faces: 1,
            max_face_vertices: 3,
            ..WavefrontObjSummary::default()
        });
        totals.add_summary(WavefrontObjSummary {
            positions: 2,
            faces: 1,
            max_face_vertices: 4,
            ..WavefrontObjSummary::default()
        });

        assert_eq!(totals.files, 2);
        assert_eq!(totals.positions, 3);
        assert_eq!(totals.faces, 2);
        assert_eq!(totals.max_face_vertices, 4);
        assert_eq!(
            WavefrontObjSummary {
                positions: 1,
                faces: 1,
                max_face_vertices: 3,
                ..WavefrontObjSummary::default()
            }
            .to_string(),
            "1 positions, 0 texcoords, 0 normals, 1 faces, 0 groups, 0 material uses"
        );
        assert_eq!(
            totals.to_string(),
            "  files: 2\n  positions: 3\n  texcoords: 0\n  normals: 0\n  faces: 2\n  max face vertices: 4\n  groups: 0\n  material libraries: 0\n  material uses: 0\n  smoothing ranges: 0\n"
        );

        let row = inspect_wavefront_obj_file(
            "terrain/footprint.obj",
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        )
        .unwrap();
        let mut inspection = WavefrontObjInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "terrain/footprint.obj: 3 positions, 0 texcoords, 0 normals, 1 faces, 0 groups, 0 material uses\n  files: 1\n  positions: 3\n  texcoords: 0\n  normals: 0\n  faces: 1\n  max face vertices: 3\n  groups: 0\n  material libraries: 0\n  material uses: 0\n  smoothing ranges: 0\n"
        );
        assert!(is_wavefront_obj_name("terrain/foo.obj"));
        assert!(is_wavefront_obj_name("terrain/foo.OBJ"));
        assert!(!is_wavefront_obj_name("terrain/foo.cgf"));
    }
}
