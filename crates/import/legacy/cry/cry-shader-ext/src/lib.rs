//! Cry/Lumberyard shader generation extension parsing.
//!
//! Follows Lumberyard's `dev/Code/CryEngine/RenderDll/Common/Shaders/ShaderParse.cpp`.

use std::{
    fmt, io,
    num::ParseIntError,
    path::{Path, PathBuf},
    str,
};

use az_core::crc::crc32;
use thiserror::Error;

pub mod source_transform;
pub use source_transform::*;

const TOP_LEVEL_COMMANDS: &[CommandDesc] = &[
    CommandDesc::new(Command::Property, "Property"),
    CommandDesc::new(Command::Version, "Version"),
    CommandDesc::new(Command::UsesCommonGlobalFlags, "UsesCommonGlobalFlags"),
];

const PROPERTY_COMMANDS: &[CommandDesc] = &[
    CommandDesc::new(Command::Name, "Name"),
    CommandDesc::new(Command::Property, "Property"),
    CommandDesc::new(Command::Description, "Description"),
    CommandDesc::new(Command::Mask, "Mask"),
    CommandDesc::new(Command::Hidden, "Hidden"),
    CommandDesc::new(Command::Precache, "Precache"),
    CommandDesc::new(Command::Runtime, "Runtime"),
    CommandDesc::new(Command::AutoPrecache, "AutoPrecache"),
    CommandDesc::new(Command::LowSpecAutoPrecache, "LowSpecAutoPrecache"),
    CommandDesc::new(Command::DependencySet, "DependencySet"),
    CommandDesc::new(Command::DependencyReset, "DependencyReset"),
    CommandDesc::new(Command::DependFlagSet, "DependFlagSet"),
    CommandDesc::new(Command::DependFlagReset, "DependFlagReset"),
];

/// File extension used by Cry shader generation metadata assets.
pub const SHADER_EXT_EXTENSION: &str = "ext";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ShaderExtInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse shader extension {path:?}: {source}")]
    Parse { path: PathBuf, source: ParseError },
}

/// Parsed `.ext` shader generation metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderExtension<'a> {
    items: Vec<ShaderItem<'a>>,
}

impl<'a> ShaderExtension<'a> {
    /// Parse a `Shaders/*.ext` payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is not UTF-8 or the script does not
    /// match the shader generation grammar.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let source = str::from_utf8(bytes).map_err(ParseError::Utf8)?;
        Self::parse_str(source)
    }

    /// Parse a `Shaders/*.ext` script.
    ///
    /// # Errors
    ///
    /// Returns an error when the script does not match the shader generation
    /// grammar.
    pub fn parse_str(source: &'a str) -> Result<Self, ParseError> {
        let mut cursor = Cursor::new(source);
        let mut items = Vec::new();

        while let Some(object) = cursor.next_object(TOP_LEVEL_COMMANDS)? {
            match object {
                RawObject::Directive(directive) => {
                    items.push(ShaderItem::Directive(directive));
                }
                RawObject::Command { command, data } => match command {
                    Command::Property => {
                        let data = data.ok_or(ParseError::MissingPropertyBlock)?;
                        items.push(ShaderItem::Property(parse_property(data)?));
                    }
                    Command::Version => {
                        if let Some(version) = data {
                            items.push(ShaderItem::Version(version));
                        }
                    }
                    Command::UsesCommonGlobalFlags => {
                        items.push(ShaderItem::UsesCommonGlobalFlags);
                    }
                    _ => return Err(ParseError::UnexpectedCommand(command.name())),
                },
            }
        }

        Ok(Self { items })
    }

    #[inline]
    #[must_use]
    pub fn items(&self) -> &[ShaderItem<'a>] {
        &self.items
    }

    #[inline]
    pub fn properties(&self) -> impl Iterator<Item = &ShaderProperty<'a>> {
        self.items.iter().filter_map(ShaderItem::property)
    }

    #[inline]
    #[must_use]
    pub fn property_count(&self) -> usize {
        self.properties().count()
    }

    #[must_use]
    pub fn version(&self) -> Option<&'a str> {
        self.items.iter().find_map(ShaderItem::version)
    }
}

/// Deterministic summary for one `.ext` shader generation asset.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShaderExtSummary<'a> {
    pub version: Option<&'a str>,
    pub properties: usize,
    pub precache_refs: usize,
    pub dependency_set_refs: usize,
    pub dependency_reset_refs: usize,
    pub hidden_properties: usize,
    pub runtime_properties: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedShaderExtSummary {
    pub version: Option<String>,
    pub properties: usize,
    pub precache_refs: usize,
    pub dependency_set_refs: usize,
    pub dependency_reset_refs: usize,
    pub hidden_properties: usize,
    pub runtime_properties: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderExtFileSummary {
    pub source: String,
    pub summary: OwnedShaderExtSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShaderExtInspection {
    pub rows: Vec<ShaderExtFileSummary>,
    pub totals: ShaderExtTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct ShaderExtInspectionReport<'a> {
    inspection: &'a ShaderExtInspection,
    limit: usize,
}

impl<'a> ShaderExtSummary<'a> {
    #[must_use]
    pub fn from_extension(extension: &ShaderExtension<'a>) -> Self {
        let mut summary = Self {
            version: extension.version(),
            ..Self::default()
        };
        for property in extension.properties() {
            summary.properties += 1;
            summary.precache_refs += property.precache_names.len();
            summary.dependency_set_refs += property.dependency_set.bits().count_ones() as usize;
            summary.dependency_reset_refs += property.dependency_reset.bits().count_ones() as usize;
            if property.flags.contains(ShaderPropertyFlags::HIDDEN) {
                summary.hidden_properties += 1;
            }
            if property.flags.contains(ShaderPropertyFlags::RUNTIME) {
                summary.runtime_properties += 1;
            }
        }
        summary
    }
}

impl OwnedShaderExtSummary {
    #[must_use]
    pub fn as_borrowed(&self) -> ShaderExtSummary<'_> {
        ShaderExtSummary {
            version: self.version.as_deref(),
            properties: self.properties,
            precache_refs: self.precache_refs,
            dependency_set_refs: self.dependency_set_refs,
            dependency_reset_refs: self.dependency_reset_refs,
            hidden_properties: self.hidden_properties,
            runtime_properties: self.runtime_properties,
        }
    }
}

impl From<ShaderExtSummary<'_>> for OwnedShaderExtSummary {
    fn from(summary: ShaderExtSummary<'_>) -> Self {
        Self {
            version: summary.version.map(str::to_owned),
            properties: summary.properties,
            precache_refs: summary.precache_refs,
            dependency_set_refs: summary.dependency_set_refs,
            dependency_reset_refs: summary.dependency_reset_refs,
            hidden_properties: summary.hidden_properties,
            runtime_properties: summary.runtime_properties,
        }
    }
}

impl fmt::Display for ShaderExtSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "version {}, {} properties, {} precache refs",
            self.version.unwrap_or("(none)"),
            self.properties,
            self.precache_refs
        )
    }
}

impl fmt::Display for OwnedShaderExtSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_borrowed().fmt(f)
    }
}

/// Summarizes one `.ext` shader generation script.
///
/// # Errors
///
/// Returns any error [`ShaderExtension::parse`] returns — [`ParseError::Utf8`]
/// for a non-UTF-8 payload, and the grammar variants for a script that does
/// not match the shader generation grammar.
pub fn summarize_shader_extension(bytes: &[u8]) -> Result<ShaderExtSummary<'_>, ParseError> {
    let extension = ShaderExtension::parse(bytes)?;
    Ok(ShaderExtSummary::from_extension(&extension))
}

/// Summarizes one `.ext` script and labels it with its display path.
///
/// # Errors
///
/// Returns any error [`summarize_shader_extension`] returns.
pub fn inspect_shader_extension_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<ShaderExtFileSummary, ParseError> {
    Ok(ShaderExtFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_shader_extension(bytes)?.into(),
    })
}

/// Reads and summarizes one `.ext` script from disk.
///
/// # Errors
///
/// Returns [`ShaderExtInspectionError::Read`] when `path` cannot be read, and
/// [`ShaderExtInspectionError::Parse`] when its contents are not a valid
/// shader generation script.
pub fn inspect_shader_extension_path(
    path: impl AsRef<Path>,
) -> Result<ShaderExtFileSummary, ShaderExtInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| ShaderExtInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_shader_extension_file(path, &bytes).map_err(|source| ShaderExtInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads and aggregates every `.ext` script in `paths`.
///
/// # Errors
///
/// Returns the first error [`inspect_shader_extension_path`] returns; the walk
/// stops at that path and the partial inspection is discarded.
pub fn inspect_shader_extension_files<I, P>(
    paths: I,
) -> Result<ShaderExtInspection, ShaderExtInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = ShaderExtInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_shader_extension_path(path)?);
    }
    Ok(inspection)
}

/// Aggregate summary across many `.ext` shader generation assets.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShaderExtTotals {
    pub files: usize,
    pub properties: usize,
    pub precache_refs: usize,
    pub dependency_set_refs: usize,
    pub dependency_reset_refs: usize,
    pub hidden_properties: usize,
    pub runtime_properties: usize,
}

impl ShaderExtTotals {
    pub const fn add_summary(&mut self, summary: ShaderExtSummary<'_>) {
        self.files += 1;
        self.properties += summary.properties;
        self.precache_refs += summary.precache_refs;
        self.dependency_set_refs += summary.dependency_set_refs;
        self.dependency_reset_refs += summary.dependency_reset_refs;
        self.hidden_properties += summary.hidden_properties;
        self.runtime_properties += summary.runtime_properties;
    }

    pub fn add_extension(&mut self, extension: &ShaderExtension<'_>) {
        self.add_summary(ShaderExtSummary::from_extension(extension));
    }
}

impl ShaderExtInspection {
    pub fn add_file_summary(&mut self, row: ShaderExtFileSummary) {
        self.totals.add_summary(row.summary.as_borrowed());
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> ShaderExtInspectionReport<'_> {
        ShaderExtInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for ShaderExtTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  properties: {}", self.properties)?;
        writeln!(f, "  precache refs: {}", self.precache_refs)?;
        writeln!(f, "  dependency set refs: {}", self.dependency_set_refs)?;
        writeln!(f, "  dependency reset refs: {}", self.dependency_reset_refs)?;
        writeln!(f, "  hidden properties: {}", self.hidden_properties)?;
        writeln!(f, "  runtime properties: {}", self.runtime_properties)
    }
}

impl fmt::Display for ShaderExtInspectionReport<'_> {
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

#[must_use]
pub const fn is_shader_ext_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(SHADER_EXT_EXTENSION)
}

#[must_use]
pub fn is_shader_ext_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_shader_ext_extension(extension))
}

#[must_use]
pub fn is_shader_ext_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_shader_ext_extension)
}

/// One top-level item in a `.ext` shader generation script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderItem<'a> {
    Version(&'a str),
    UsesCommonGlobalFlags,
    Directive(PreprocessorDirective<'a>),
    Property(ShaderProperty<'a>),
}

impl<'a> ShaderItem<'a> {
    #[inline]
    #[must_use]
    pub const fn version(&self) -> Option<&'a str> {
        match self {
            Self::Version(version) => Some(version),
            Self::UsesCommonGlobalFlags | Self::Directive(_) | Self::Property(_) => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn directive(&self) -> Option<PreprocessorDirective<'a>> {
        match self {
            Self::Directive(directive) => Some(*directive),
            Self::Version(_) | Self::UsesCommonGlobalFlags | Self::Property(_) => None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn property(&self) -> Option<&ShaderProperty<'a>> {
        match self {
            Self::Property(property) => Some(property),
            Self::Version(_) | Self::UsesCommonGlobalFlags | Self::Directive(_) => None,
        }
    }
}

/// A shader-generation property from `SShaderGenBit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderProperty<'a> {
    pub name: &'a str,
    pub name_crc32: u32,
    pub display_name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub mask: u64,
    pub flags: ShaderPropertyFlags,
    pub precache_names: Vec<&'a str>,
    pub depend_flag_set: Vec<&'a str>,
    pub depend_flag_reset: Vec<&'a str>,
    pub dependency_set: ShaderDependencyFlags,
    pub dependency_reset: ShaderDependencyFlags,
}

impl ShaderProperty<'_> {
    #[inline]
    pub fn precache_crc32(&self) -> impl Iterator<Item = u32> + '_ {
        self.precache_names
            .iter()
            .copied()
            .map(|name| crc32(name.as_bytes()))
    }
}

/// Property flags from `SHGF_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct ShaderPropertyFlags(u32);

impl ShaderPropertyFlags {
    pub const HIDDEN: Self = Self(0x01);
    pub const PRECACHE: Self = Self(0x02);
    pub const AUTO_PRECACHE: Self = Self(0x04);
    pub const LOW_SPEC_AUTO_PRECACHE: Self = Self(0x08);
    pub const RUNTIME: Self = Self(0x10);

    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    #[inline]
    pub const fn insert(&mut self, flag: Self) {
        self.0 |= flag.0;
    }
}

/// Shader dependency flags from `SHGD_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct ShaderDependencyFlags(u32);

impl ShaderDependencyFlags {
    pub const LM_DIFFUSE: Self = Self(0x0000_0001);
    pub const TEX_DETAIL: Self = Self(0x0000_0002);
    pub const TEX_NORMALS: Self = Self(0x0000_0004);
    pub const TEX_ENVCM: Self = Self(0x0000_0008);
    pub const TEX_SPECULAR: Self = Self(0x0000_0010);
    pub const TEX_SECOND_SMOOTHNESS: Self = Self(0x0000_0020);
    pub const TEX_HEIGHT: Self = Self(0x0000_0040);
    pub const TEX_SUBSURFACE: Self = Self(0x0000_0080);
    pub const HW_BILINEAR_FP16: Self = Self(0x0000_0100);
    pub const HW_SEPARATE_FP16: Self = Self(0x0000_0200);
    pub const HW_DURANGO: Self = Self(0x0000_0400);
    pub const HW_ORBIS: Self = Self(0x0000_0800);
    pub const TEX_CUSTOM: Self = Self(0x0000_1000);
    pub const TEX_CUSTOM_SECONDARY: Self = Self(0x0000_2000);
    pub const TEX_DECAL: Self = Self(0x0000_4000);
    pub const TEX_OCC: Self = Self(0x0000_8000);
    pub const TEX_SPECULAR_2: Self = Self(0x0001_0000);
    pub const HW_GLES3: Self = Self(0x0002_0000);
    pub const USER_ENABLED: Self = Self(0x0004_0000);
    pub const HW_SAA: Self = Self(0x0008_0000);
    pub const TEX_EMITTANCE: Self = Self(0x0010_0000);
    pub const HW_DX12: Self = Self(0x0020_0000);
    pub const HW_DX10: Self = Self::HW_DX12;
    pub const HW_DX11: Self = Self(0x0040_0000);
    pub const HW_GL4: Self = Self(0x0080_0000);
    pub const HW_WATER_TESSELLATION: Self = Self(0x0100_0000);
    pub const HW_SILHOUETTE_POM: Self = Self(0x0200_0000);
    pub const HW_PROSPERO: Self = Self(0x0400_0000);
    pub const HW_METAL: Self = Self::HW_PROSPERO;
    pub const HW_SCARLETT: Self = Self(0x0800_0000);
    pub const TEX_MASK: Self = Self(
        Self::TEX_DETAIL.0
            | Self::TEX_NORMALS.0
            | Self::TEX_ENVCM.0
            | Self::TEX_SPECULAR.0
            | Self::TEX_SECOND_SMOOTHNESS.0
            | Self::TEX_HEIGHT.0
            | Self::TEX_SUBSURFACE.0
            | Self::TEX_CUSTOM.0
            | Self::TEX_CUSTOM_SECONDARY.0
            | Self::TEX_DECAL.0
            | Self::TEX_OCC.0
            | Self::TEX_SPECULAR_2.0
            | Self::TEX_EMITTANCE.0,
    );

    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    #[inline]
    pub const fn insert(&mut self, flag: Self) {
        self.0 |= flag.0;
    }
}

/// A preprocessor directive preserved from the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreprocessorDirective<'a> {
    pub kind: PreprocessorDirectiveKind,
    pub condition: Option<&'a str>,
}

/// Supported preprocessor directive categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreprocessorDirectiveKind {
    If,
    Ifdef,
    Ifndef,
    Elif,
    Else,
    Endif,
    Other,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("shader extension is not valid UTF-8: {0}")]
    Utf8(str::Utf8Error),
    #[error("unterminated block comment")]
    UnterminatedComment,
    #[error("unterminated {delimiter} block")]
    UnterminatedDelimited { delimiter: char },
    #[error("unexpected token at line {line}, column {column}")]
    UnknownToken { line: usize, column: usize },
    #[error("unexpected command {0}")]
    UnexpectedCommand(&'static str),
    #[error("Property command is missing a block")]
    MissingPropertyBlock,
    #[error("Property block is missing Name")]
    MissingPropertyName,
    #[error("invalid shader property mask {value:?}: {source}")]
    InvalidMask {
        value: String,
        source: ParseIntError,
    },
    #[error("unknown {field} dependency flag {value:?}")]
    UnknownDependency { field: &'static str, value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Name,
    Property,
    Description,
    Mask,
    Hidden,
    Runtime,
    Precache,
    DependencySet,
    DependencyReset,
    DependFlagSet,
    DependFlagReset,
    AutoPrecache,
    LowSpecAutoPrecache,
    Version,
    UsesCommonGlobalFlags,
}

impl Command {
    const fn name(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Property => "Property",
            Self::Description => "Description",
            Self::Mask => "Mask",
            Self::Hidden => "Hidden",
            Self::Runtime => "Runtime",
            Self::Precache => "Precache",
            Self::DependencySet => "DependencySet",
            Self::DependencyReset => "DependencyReset",
            Self::DependFlagSet => "DependFlagSet",
            Self::DependFlagReset => "DependFlagReset",
            Self::AutoPrecache => "AutoPrecache",
            Self::LowSpecAutoPrecache => "LowSpecAutoPrecache",
            Self::Version => "Version",
            Self::UsesCommonGlobalFlags => "UsesCommonGlobalFlags",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CommandDesc {
    command: Command,
    token: &'static str,
}

impl CommandDesc {
    const fn new(command: Command, token: &'static str) -> Self {
        Self { command, token }
    }
}

#[derive(Debug, Clone, Copy)]
enum RawObject<'a> {
    Command {
        command: Command,
        data: Option<&'a str>,
    },
    Directive(PreprocessorDirective<'a>),
}

fn parse_property(source: &str) -> Result<ShaderProperty<'_>, ParseError> {
    let mut cursor = Cursor::new(source);
    let mut property = ShaderProperty {
        name: "",
        name_crc32: 0,
        display_name: None,
        description: None,
        mask: 0,
        flags: ShaderPropertyFlags::empty(),
        precache_names: Vec::new(),
        depend_flag_set: Vec::new(),
        depend_flag_reset: Vec::new(),
        dependency_set: ShaderDependencyFlags::empty(),
        dependency_reset: ShaderDependencyFlags::empty(),
    };

    while let Some(object) = cursor.next_object(PROPERTY_COMMANDS)? {
        let RawObject::Command { command, data } = object else {
            continue;
        };
        let data = data.unwrap_or("");

        match command {
            Command::Name => {
                property.name = data;
                property.name_crc32 = crc32(data.as_bytes());
            }
            Command::Property => property.display_name = Some(data),
            Command::Description => property.description = Some(data),
            Command::Mask => property.mask = parse_mask(data)?,
            Command::Hidden => property.flags.insert(ShaderPropertyFlags::HIDDEN),
            Command::Runtime => property.flags.insert(ShaderPropertyFlags::RUNTIME),
            Command::AutoPrecache => property.flags.insert(ShaderPropertyFlags::AUTO_PRECACHE),
            Command::LowSpecAutoPrecache => property
                .flags
                .insert(ShaderPropertyFlags::LOW_SPEC_AUTO_PRECACHE),
            Command::Precache => {
                property.precache_names.push(data);
                property.flags.insert(ShaderPropertyFlags::PRECACHE);
            }
            Command::DependencySet => {
                let flag =
                    dependency_set_flag(data).ok_or_else(|| ParseError::UnknownDependency {
                        field: "set",
                        value: data.to_string(),
                    })?;
                property.dependency_set.insert(flag);
            }
            Command::DependencyReset => {
                apply_dependency_reset(data, &mut property)?;
            }
            Command::DependFlagSet => property.depend_flag_set.push(data),
            Command::DependFlagReset => property.depend_flag_reset.push(data),
            Command::Version | Command::UsesCommonGlobalFlags => {
                return Err(ParseError::UnexpectedCommand(command.name()));
            }
        }
    }

    if property.name.is_empty() {
        return Err(ParseError::MissingPropertyName);
    }

    Ok(property)
}

fn parse_mask(value: &str) -> Result<u64, ParseError> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(|| value.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|source| ParseError::InvalidMask {
            value: value.to_string(),
            source,
        })
}

const fn dependency_set_flag(name: &str) -> Option<ShaderDependencyFlags> {
    if name.eq_ignore_ascii_case("$LM_Diffuse") {
        Some(ShaderDependencyFlags::LM_DIFFUSE)
    } else if name.eq_ignore_ascii_case("$TEX_Detail") {
        Some(ShaderDependencyFlags::TEX_DETAIL)
    } else if name.eq_ignore_ascii_case("$TEX_Normals") || name.eq_ignore_ascii_case("$TEX_Bump") {
        Some(ShaderDependencyFlags::TEX_NORMALS)
    } else if name.eq_ignore_ascii_case("$TEX_Height")
        || name.eq_ignore_ascii_case("$TEX_BumpHeight")
    {
        Some(ShaderDependencyFlags::TEX_HEIGHT)
    } else if name.eq_ignore_ascii_case("$TEX_SecondSmoothness")
        || name.eq_ignore_ascii_case("$TEX_Translucency")
        || name.eq_ignore_ascii_case("$TEX_BumpDif")
    {
        Some(ShaderDependencyFlags::TEX_SECOND_SMOOTHNESS)
    } else if name.eq_ignore_ascii_case("$TEX_Specular") || name.eq_ignore_ascii_case("$TEX_Gloss")
    {
        Some(ShaderDependencyFlags::TEX_SPECULAR)
    } else if name.eq_ignore_ascii_case("$TEX_EnvCM") {
        Some(ShaderDependencyFlags::TEX_ENVCM)
    } else if name.eq_ignore_ascii_case("$TEX_Subsurface") {
        Some(ShaderDependencyFlags::TEX_SUBSURFACE)
    } else if name.eq_ignore_ascii_case("$HW_BilinearFP16") {
        Some(ShaderDependencyFlags::HW_BILINEAR_FP16)
    } else if name.eq_ignore_ascii_case("$HW_SeparateFP16") {
        Some(ShaderDependencyFlags::HW_SEPARATE_FP16)
    } else if name.eq_ignore_ascii_case("$TEX_Custom") {
        Some(ShaderDependencyFlags::TEX_CUSTOM)
    } else if name.eq_ignore_ascii_case("$TEX_CustomSecondary") {
        Some(ShaderDependencyFlags::TEX_CUSTOM_SECONDARY)
    } else if name.eq_ignore_ascii_case("$TEX_Decal") {
        Some(ShaderDependencyFlags::TEX_DECAL)
    } else if name.eq_ignore_ascii_case("$TEX_Occ") {
        Some(ShaderDependencyFlags::TEX_OCC)
    } else if name.eq_ignore_ascii_case("$HW_WaterTessellation") {
        Some(ShaderDependencyFlags::HW_WATER_TESSELLATION)
    } else if name.eq_ignore_ascii_case("$HW_SilhouettePom") {
        Some(ShaderDependencyFlags::HW_SILHOUETTE_POM)
    } else if name.eq_ignore_ascii_case("$HW_SpecularAntialiasing") {
        Some(ShaderDependencyFlags::HW_SAA)
    } else if name.eq_ignore_ascii_case("$UserEnabled") {
        Some(ShaderDependencyFlags::USER_ENABLED)
    } else if name.eq_ignore_ascii_case("$HW_DURANGO") {
        Some(ShaderDependencyFlags::HW_DURANGO)
    } else if name.eq_ignore_ascii_case("$HW_ORBIS") {
        Some(ShaderDependencyFlags::HW_ORBIS)
    } else if name.eq_ignore_ascii_case("$HW_DX11") {
        Some(ShaderDependencyFlags::HW_DX11)
    } else if name.eq_ignore_ascii_case("$HW_DX12") || name.eq_ignore_ascii_case("$HW_DX10") {
        Some(ShaderDependencyFlags::HW_DX12)
    } else if name.eq_ignore_ascii_case("$HW_GL4") {
        Some(ShaderDependencyFlags::HW_GL4)
    } else if name.eq_ignore_ascii_case("$HW_GLES3") {
        Some(ShaderDependencyFlags::HW_GLES3)
    } else if name.eq_ignore_ascii_case("$TEX_Emittance") {
        Some(ShaderDependencyFlags::TEX_EMITTANCE)
    } else if name.eq_ignore_ascii_case("$HW_PROSPERO") || name.eq_ignore_ascii_case("$HW_METAL") {
        Some(ShaderDependencyFlags::HW_PROSPERO)
    } else if name.eq_ignore_ascii_case("$HW_SCARLETT") {
        Some(ShaderDependencyFlags::HW_SCARLETT)
    } else {
        None
    }
}

fn apply_dependency_reset(name: &str, property: &mut ShaderProperty<'_>) -> Result<(), ParseError> {
    if name.eq_ignore_ascii_case("$TEX_Bump")
        || name.eq_ignore_ascii_case("$TEX_BumpHeight")
        || name.eq_ignore_ascii_case("$TEX_BumpDif")
        || name.eq_ignore_ascii_case("$TEX_Gloss")
    {
        if let Some(flag) = dependency_set_flag(name) {
            property.dependency_set.insert(flag);
        }
    } else if let Some(flag) = dependency_set_flag(name) {
        property.dependency_reset.insert(flag);
    } else {
        return Err(ParseError::UnknownDependency {
            field: "reset",
            value: name.to_string(),
        });
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct Cursor<'a> {
    source: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a str) -> Self {
        let pos = source
            .as_bytes()
            .strip_prefix(b"\xEF\xBB\xBF")
            .map_or(0, |_| 3);
        Self { source, pos }
    }

    fn next_object(
        &mut self,
        commands: &[CommandDesc],
    ) -> Result<Option<RawObject<'a>>, ParseError> {
        self.skip_space_and_comments()?;
        if self.is_eof() {
            return Ok(None);
        }

        if self.peek_byte() == Some(b'#') {
            return Ok(Some(RawObject::Directive(self.read_directive())));
        }

        for desc in commands {
            if self.starts_with_ignore_ascii_case(desc.token) {
                self.pos += desc.token.len();
                self.skip_space();
                let name = self.read_delimited('\'', '\'')?;
                self.skip_space();
                let data = if self.consume_byte(b'=') {
                    Some(self.read_assignment())
                } else if let Some(data) = self.read_delimited('(', ')')? {
                    Some(data)
                } else {
                    self.read_delimited('{', '}')?
                };
                return Ok(Some(RawObject::Command {
                    command: desc.command,
                    data: name.or(data),
                }));
            }
        }

        let (line, column) = self.line_column();
        Err(ParseError::UnknownToken { line, column })
    }

    const fn is_eof(self) -> bool {
        self.pos >= self.source.len()
    }

    fn peek_byte(self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn consume_byte(&mut self, byte: u8) -> bool {
        if self.peek_byte() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn starts_with_ignore_ascii_case(self, prefix: &str) -> bool {
        self.source.as_bytes()[self.pos..]
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(prefix.as_bytes()))
    }

    fn skip_space(&mut self) {
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_whitespace() || byte == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_space_and_comments(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_space();
            if self.source.as_bytes()[self.pos..].starts_with(b"//") {
                self.skip_line_comment();
            } else if self.source.as_bytes()[self.pos..].starts_with(b"/*") {
                self.skip_block_comment()?;
            } else {
                return Ok(());
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(byte) = self.peek_byte() {
            self.pos += 1;
            if byte == b'\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), ParseError> {
        let mut depth = 0usize;
        while !self.is_eof() {
            let remaining = &self.source.as_bytes()[self.pos..];
            if remaining.starts_with(b"/*") {
                depth += 1;
                self.pos += 2;
            } else if remaining.starts_with(b"*/") {
                depth -= 1;
                self.pos += 2;
                if depth == 0 {
                    return Ok(());
                }
            } else {
                self.pos += 1;
            }
        }

        Err(ParseError::UnterminatedComment)
    }

    fn read_assignment(&mut self) -> &'a str {
        self.skip_space();
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if byte <= 0x20 || byte == b';' {
                break;
            }
            self.pos += 1;
        }
        let end = self.pos;
        if self.peek_byte().is_some() {
            self.pos += 1;
        }
        &self.source[start..end]
    }

    fn read_delimited(&mut self, open: char, close: char) -> Result<Option<&'a str>, ParseError> {
        if self.peek_byte() != Some(open as u8) {
            return Ok(None);
        }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1usize;
        let open_byte = open as u8;
        let close_byte = close as u8;

        while let Some(byte) = self.peek_byte() {
            if open != close && byte == open_byte {
                depth += 1;
            } else if byte == close_byte {
                depth -= 1;
                if depth == 0 {
                    let end = self.pos;
                    self.pos += 1;
                    return Ok(Some(&self.source[start..end]));
                }
            }
            self.pos += 1;
        }

        Err(ParseError::UnterminatedDelimited { delimiter: close })
    }

    fn read_directive(&mut self) -> PreprocessorDirective<'a> {
        self.pos += 1;
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if byte == b'\r' || byte == b'\n' {
                break;
            }
            self.pos += 1;
        }
        let end = self.pos;
        self.skip_line_comment();

        let line = self.source[start..end].trim();
        let mut parts = line.splitn(2, char::is_whitespace);
        let directive = parts.next().unwrap_or("");
        let condition = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let kind = if directive.eq_ignore_ascii_case("if") {
            PreprocessorDirectiveKind::If
        } else if directive.eq_ignore_ascii_case("ifdef") {
            PreprocessorDirectiveKind::Ifdef
        } else if directive.eq_ignore_ascii_case("ifndef") {
            PreprocessorDirectiveKind::Ifndef
        } else if directive.eq_ignore_ascii_case("elif") {
            PreprocessorDirectiveKind::Elif
        } else if directive.eq_ignore_ascii_case("else") {
            PreprocessorDirectiveKind::Else
        } else if directive.eq_ignore_ascii_case("endif") {
            PreprocessorDirectiveKind::Endif
        } else {
            PreprocessorDirectiveKind::Other
        };

        PreprocessorDirective { kind, condition }
    }

    fn line_column(self) -> (usize, usize) {
        let mut line = 1usize;
        let mut column = 1usize;
        for byte in self.source.as_bytes()[..self.pos].iter().copied() {
            if byte == b'\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        (line, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shader_generation_properties() {
        let script = br"
            Version (1.00)
            Property
            {
                Name = %NORMAL_MAP
                Mask = 0x1 // inline comment
                Property (Normal map)
                Description (Use normal-map texture)
                DependencySet = $TEX_Normals
                DependencyReset = $TEX_Normals
                Hidden
            }
        ";

        let ext = ShaderExtension::parse(script).unwrap();
        let property = ext.properties().next().unwrap();

        assert_eq!(ext.version(), Some("1.00"));
        assert_eq!(property.name, "%NORMAL_MAP");
        assert_eq!(property.name_crc32, crc32(b"%NORMAL_MAP"));
        assert_eq!(property.mask, 1);
        assert_eq!(property.display_name, Some("Normal map"));
        assert_eq!(property.description, Some("Use normal-map texture"));
        assert!(property.flags.contains(ShaderPropertyFlags::HIDDEN));
        assert!(
            property
                .dependency_set
                .contains(ShaderDependencyFlags::TEX_NORMALS)
        );
        assert!(
            property
                .dependency_reset
                .contains(ShaderDependencyFlags::TEX_NORMALS)
        );

        let summary = summarize_shader_extension(script).unwrap();
        assert_eq!(summary.version, Some("1.00"));
        assert_eq!(summary.properties, 1);
        assert_eq!(summary.dependency_set_refs, 1);
        assert_eq!(summary.dependency_reset_refs, 1);
        assert_eq!(summary.hidden_properties, 1);

        let mut totals = ShaderExtTotals::default();
        totals.add_summary(summary);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.properties, 1);
        assert_eq!(
            summary.to_string(),
            "version 1.00, 1 properties, 0 precache refs"
        );
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  properties: 1\n  precache refs: 0\n  dependency set refs: 1\n  dependency reset refs: 1\n  hidden properties: 1\n  runtime properties: 0\n"
        );

        let mut inspection = ShaderExtInspection::default();
        inspection.add_file_summary(
            inspect_shader_extension_file("shaders/terrain.ext", script)
                .expect("inspect shader ext"),
        );
        assert_eq!(
            inspection.report(20).to_string(),
            "shaders/terrain.ext: version 1.00, 1 properties, 0 precache refs\n  files: 1\n  properties: 1\n  precache refs: 0\n  dependency set refs: 1\n  dependency reset refs: 1\n  hidden properties: 1\n  runtime properties: 0\n"
        );
    }

    #[test]
    fn inspect_shader_extension_files_aggregates_file_results() {
        let path = std::env::temp_dir().join(format!(
            "az-rs-cry-shader-ext-{}-terrain.ext",
            std::process::id()
        ));
        std::fs::write(&path, b"Version (1.00)\nProperty { Name = %A Mask = 1 }")
            .expect("write shader ext");

        let inspection =
            inspect_shader_extension_files([&path]).expect("inspect shader extension files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.properties, 1);

        std::fs::remove_file(path).expect("remove shader ext");
    }

    #[test]
    fn preserves_preprocessor_directives() {
        let ext = ShaderExtension::parse_str(
            r"
            #ifdef FEATURE_MESH_TESSELLATION
            Property { Name = %DISPLACEMENT_MAPPING Mask = 0x10000000 }
            #endif
            ",
        )
        .unwrap();

        assert_eq!(
            ext.items()[0].directive().unwrap(),
            PreprocessorDirective {
                kind: PreprocessorDirectiveKind::Ifdef,
                condition: Some("FEATURE_MESH_TESSELLATION"),
            }
        );
        assert_eq!(ext.property_count(), 1);
    }

    #[test]
    fn maps_dx12_dependency_name() {
        let ext = ShaderExtension::parse_str(
            "Property { Name = %DX12 Mask = 1 DependencySet = $HW_DX12 }",
        )
        .unwrap();
        let property = ext.properties().next().unwrap();
        assert!(
            property
                .dependency_set
                .contains(ShaderDependencyFlags::HW_DX12)
        );
    }

    #[test]
    fn rejects_unknown_dependency_names() {
        let error = ShaderExtension::parse_str(
            "Property { Name = %BAD Mask = 1 DependencyReset = $UNKNOWN }",
        )
        .unwrap_err();

        assert!(matches!(error, ParseError::UnknownDependency { .. }));
    }

    #[test]
    fn recognizes_shader_ext_paths() {
        assert!(is_shader_ext_name("illum.EXT"));
        assert!(is_shader_ext_path(Path::new("Shaders/illum.ext")));
        assert!(!is_shader_ext_name("illum.cfx"));
    }

    #[test]
    fn shader_preset_transform_emits_authoring_source() {
        use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

        use crate::source_transform::{
            SHADER_PRESET_SOURCE_SCHEMA, ShaderPresetItemSource, ShaderPresetSource,
            ShaderPresetSourceTransform, ShaderPropertyFlagSource, ShaderPropertySource,
            is_legacy_shader_preset_source, shader_preset_source_path,
        };

        assert!(is_legacy_shader_preset_source("Shaders/Illum.ext"));
        assert!(!is_legacy_shader_preset_source("Shaders/Cache/Illum.ext"));
        assert_eq!(
            shader_preset_source_path("Shaders/Illum.ext"),
            "shaders/illum.shaderpreset.ron"
        );

        let output = ShaderPresetSourceTransform
            .transform(LegacySourceInput::new(
                "Shaders/Illum.ext",
                br"
                    Version (1.00)
                    UsesCommonGlobalFlags
                    #ifdef FEATURE_MESH_TESSELLATION
                    Property
                    {
                        Name = %NORMAL_MAP
                        Mask = 0x1
                        Property (Normal map)
                        Description (Use normal-map texture)
                        Precache = %DETAIL_MAP
                        AutoPrecache
                        Runtime
                        DependencySet = $TEX_Normals
                        DependencyReset = $HW_DX12
                        DependFlagSet = %USE_FOAM
                        DependFlagReset = %NO_DECALS
                    }
                    #endif
                ",
            ))
            .unwrap();

        let LegacySourceOutput::AuthoringSource(artifact) = output else {
            panic!("shader .ext should become authoring source");
        };
        assert_eq!(artifact.path, "shaders/illum.shaderpreset.ron");
        assert_eq!(artifact.schema, SHADER_PRESET_SOURCE_SCHEMA);

        let source = ShaderPresetSource::from_ron_bytes(&artifact.bytes).unwrap();
        assert_eq!(source.source_path, "shaders/illum.ext");
        assert_eq!(
            source.items[0],
            ShaderPresetItemSource::Version("1.00".to_string())
        );
        assert_eq!(
            source.items[1],
            ShaderPresetItemSource::UsesCommonGlobalFlags
        );
        assert!(matches!(
            source.items[2],
            ShaderPresetItemSource::Directive(_)
        ));

        let ShaderPresetItemSource::Property(ShaderPropertySource {
            name,
            display_name,
            description,
            mask,
            flags,
            precache_names,
            depend_flag_set,
            depend_flag_reset,
            dependency_set,
            dependency_reset,
        }) = &source.items[3]
        else {
            panic!("expected property item");
        };
        assert_eq!(name, "%NORMAL_MAP");
        assert_eq!(display_name.as_deref(), Some("Normal map"));
        assert_eq!(description.as_deref(), Some("Use normal-map texture"));
        assert_eq!(*mask, 1);
        assert!(flags.contains(&ShaderPropertyFlagSource::AutoPrecache));
        assert!(flags.contains(&ShaderPropertyFlagSource::Runtime));
        assert_eq!(precache_names, &vec!["%DETAIL_MAP".to_string()]);
        assert_eq!(depend_flag_set, &vec!["%USE_FOAM".to_string()]);
        assert_eq!(depend_flag_reset, &vec!["%NO_DECALS".to_string()]);
        assert_eq!(dependency_set, &vec!["tex_normals".to_string()]);
        assert_eq!(dependency_reset, &vec!["hw_dx12".to_string()]);
    }
}
