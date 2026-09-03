//! `CryEntitySystem` class descriptor parsing.
//!
//! Follows Lumberyard's `dev/Gems/CryLegacy/Code/Source/CryEntitySystem/EntityClassRegistry.cpp`.

use std::{
    borrow::Cow,
    fmt, io,
    num::ParseIntError,
    path::{Path, PathBuf},
    str,
};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use thiserror::Error;

pub mod source_transform;

pub use source_transform::*;

const ENTITY: &[u8] = b"Entity";
pub const ENTITY_CLASS_DESCRIPTOR_EXTENSION: &str = "ent";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EntityClassInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse entity class {path:?}: {source}")]
    Parse {
        path: PathBuf,
        source: EntityClassParseError,
    },
}

/// `IEntityClass` flag bits parsed from `.ent` descriptors.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EntityClassFlags(u32);

impl EntityClassFlags {
    pub const EMPTY: Self = Self(0);
    pub const INVISIBLE: Self = Self(0x1);
    pub const BBOX_SELECTION: Self = Self(0x4);

    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Parsed `.ent` class descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityClassDescriptor<'a> {
    pub name: Cow<'a, str>,
    pub script: Option<Cow<'a, str>>,
    pub flags: EntityClassFlags,
}

impl EntityClassDescriptor<'_> {
    #[must_use]
    pub fn into_owned(self) -> EntityClassDescriptor<'static> {
        EntityClassDescriptor {
            name: Cow::Owned(self.name.into_owned()),
            script: self.script.map(|script| Cow::Owned(script.into_owned())),
            flags: self.flags,
        }
    }

    #[must_use]
    pub const fn is_invisible(&self) -> bool {
        self.flags.contains(EntityClassFlags::INVISIBLE)
    }

    #[must_use]
    pub const fn has_bbox_selection(&self) -> bool {
        self.flags.contains(EntityClassFlags::BBOX_SELECTION)
    }
}

/// Summary returned after visiting an `.ent` descriptor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EntityClassStats {
    pub descriptors: usize,
    pub scripted: usize,
    pub invisible: usize,
    pub bbox_selection: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EntityClassSummary {
    pub class_name: Option<Box<str>>,
    pub stats: EntityClassStats,
}

impl fmt::Display for EntityClassSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.class_name.as_deref().unwrap_or("<unnamed>"))
    }
}

/// Reads an `.ent` descriptor and reports its class name and element stats.
///
/// # Errors
///
/// Returns any error [`visit_entity_class_descriptor`] returns —
/// [`EntityClassParseError::Utf8`] for non-UTF-8 bytes,
/// [`EntityClassParseError::Xml`] or [`EntityClassParseError::Attribute`] for a
/// malformed document, [`EntityClassParseError::UnexpectedElement`] for an
/// element other than `Entity`, [`EntityClassParseError::MissingAttribute`] or
/// [`EntityClassParseError::EmptyName`] for a nameless class, and
/// [`EntityClassParseError::InvalidBoolean`] for a flag attribute that is not
/// an integer.
pub fn summarize_entity_class_descriptor(
    bytes: &[u8],
) -> Result<EntityClassSummary, EntityClassParseError> {
    let mut class_name = None;
    let stats = visit_entity_class_descriptor(bytes, |descriptor| {
        class_name = Some(descriptor.name.into_owned().into_boxed_str());
        Ok(())
    })?;
    Ok(EntityClassSummary { class_name, stats })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EntityClassTotals {
    pub files: usize,
    pub descriptors: usize,
    pub scripted: usize,
    pub invisible: usize,
    pub bbox_selection: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityClassFileSummary {
    pub source: String,
    pub summary: EntityClassSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EntityClassInspection {
    pub rows: Vec<EntityClassFileSummary>,
    pub totals: EntityClassTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct EntityClassInspectionReport<'a> {
    inspection: &'a EntityClassInspection,
    limit: usize,
}

impl EntityClassTotals {
    pub const fn add_stats(&mut self, stats: EntityClassStats) {
        self.files += 1;
        self.descriptors += stats.descriptors;
        self.scripted += stats.scripted;
        self.invisible += stats.invisible;
        self.bbox_selection += stats.bbox_selection;
    }

    pub const fn add_summary(&mut self, summary: &EntityClassSummary) {
        self.add_stats(summary.stats);
    }
}

impl EntityClassInspection {
    pub fn add_file_summary(&mut self, row: EntityClassFileSummary) {
        self.totals.add_summary(&row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> EntityClassInspectionReport<'_> {
        EntityClassInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for EntityClassTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  descriptors: {}", self.descriptors)?;
        writeln!(f, "  scripted: {}", self.scripted)?;
        writeln!(f, "  invisible: {}", self.invisible)?;
        writeln!(f, "  bbox selection: {}", self.bbox_selection)
    }
}

impl fmt::Display for EntityClassInspectionReport<'_> {
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

/// Summarises one `.ent` descriptor's bytes, labelling the row with `path`.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_entity_class_descriptor`] returns — the
/// [`EntityClassParseError`] variants for non-UTF-8 bytes, malformed XML, an
/// unexpected element, a missing or empty class name, or an unparseable flag
/// attribute.
pub fn inspect_entity_class_descriptor_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<EntityClassFileSummary, EntityClassParseError> {
    Ok(EntityClassFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_entity_class_descriptor(bytes)?,
    })
}

/// Reads an `.ent` descriptor from disk and summarises it.
///
/// # Errors
///
/// Returns [`EntityClassInspectionError::Read`] if `path` cannot be read
/// (missing file, permissions), or [`EntityClassInspectionError::Parse`]
/// wrapping the [`EntityClassParseError`] from a malformed descriptor. Both
/// variants carry the offending path.
pub fn inspect_entity_class_descriptor_path(
    path: impl AsRef<Path>,
) -> Result<EntityClassFileSummary, EntityClassInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| EntityClassInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_entity_class_descriptor_file(path, &bytes).map_err(|source| {
        EntityClassInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Reads and summarises every `.ent` descriptor in `paths`, accumulating
/// totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_entity_class_descriptor_path`] returns for the
/// first path that fails — [`EntityClassInspectionError::Read`] for an
/// unreadable file, or [`EntityClassInspectionError::Parse`] for a malformed
/// descriptor.
pub fn inspect_entity_class_descriptor_files<I, P>(
    paths: I,
) -> Result<EntityClassInspection, EntityClassInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = EntityClassInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_entity_class_descriptor_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_entity_class_descriptor_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(ENTITY_CLASS_DESCRIPTOR_EXTENSION)
}

#[must_use]
pub fn is_entity_class_descriptor_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_entity_class_descriptor_extension(extension))
}

#[must_use]
pub fn is_entity_class_descriptor_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_entity_class_descriptor_extension)
}

/// Parse an `.ent` descriptor into structured data.
///
/// # Errors
///
/// Returns [`EntityClassParseError::MissingRoot`] if the document contains no
/// `Entity` element at all, plus any error
/// [`visit_entity_class_descriptor`] returns —
/// [`EntityClassParseError::Utf8`] for non-UTF-8 bytes,
/// [`EntityClassParseError::Xml`] or [`EntityClassParseError::Attribute`] for a
/// malformed document, [`EntityClassParseError::UnexpectedElement`] for an
/// element other than `Entity`, [`EntityClassParseError::MissingAttribute`] or
/// [`EntityClassParseError::EmptyName`] for a nameless class, and
/// [`EntityClassParseError::InvalidBoolean`] for a flag attribute that is not
/// an integer.
pub fn parse_entity_class_descriptor(
    bytes: &[u8],
) -> Result<EntityClassDescriptor<'static>, EntityClassParseError> {
    let mut descriptor = None;
    visit_entity_class_descriptor(bytes, |item| {
        descriptor = Some(item.into_owned());
        Ok(())
    })?;
    descriptor.ok_or(EntityClassParseError::MissingRoot)
}

/// Parse an `.ent` descriptor with a streaming visitor.
///
/// # Errors
///
/// Returns [`EntityClassParseError::Utf8`] if `bytes` is not valid UTF-8,
/// [`EntityClassParseError::Xml`] for a document the reader rejects,
/// [`EntityClassParseError::Attribute`] for an unparseable attribute,
/// [`EntityClassParseError::UnexpectedElement`] for any element other than
/// `Entity`, [`EntityClassParseError::MissingAttribute`] when the `Name`
/// attribute is absent, [`EntityClassParseError::EmptyName`] when it is
/// present but blank, and [`EntityClassParseError::InvalidBoolean`] when a
/// flag attribute is not an integer. Any error `visitor` itself returns is
/// propagated unchanged.
pub fn visit_entity_class_descriptor<F>(
    bytes: &[u8],
    mut visitor: F,
) -> Result<EntityClassStats, EntityClassParseError>
where
    F: FnMut(EntityClassDescriptor<'_>) -> Result<(), EntityClassParseError>,
{
    let xml = str::from_utf8(bytes)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stats = EntityClassStats::default();

    loop {
        match reader.read_event()? {
            Event::Start(event) | Event::Empty(event) => {
                let descriptor = parse_entity_start(&reader, &event)?;
                stats.descriptors += 1;
                if descriptor.script.is_some() {
                    stats.scripted += 1;
                }
                if descriptor.is_invisible() {
                    stats.invisible += 1;
                }
                if descriptor.has_bbox_selection() {
                    stats.bbox_selection += 1;
                }
                visitor(descriptor)?;
            }
            Event::Eof => break,
            Event::End(_)
            | Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::Comment(_)
            | Event::GeneralRef(_) => {}
        }
    }

    if stats.descriptors == 0 {
        return Err(EntityClassParseError::MissingRoot);
    }

    Ok(stats)
}

fn parse_entity_start<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
) -> Result<EntityClassDescriptor<'a>, EntityClassParseError> {
    let name = event.local_name();
    if name.as_ref() != ENTITY {
        return Err(EntityClassParseError::UnexpectedElement {
            element: String::from_utf8_lossy(name.as_ref()).into_owned(),
        });
    }

    let name = required_attr(reader, event, b"Name", "Entity.Name")?;
    if name.is_empty() {
        return Err(EntityClassParseError::EmptyName);
    }

    let script = attr_value(reader, event, b"Script")?.filter(|script| !script.is_empty());
    let mut flags = EntityClassFlags::EMPTY;
    if attr_bool(reader, event, b"Invisible", "Entity.Invisible")?.unwrap_or(false) {
        flags.insert(EntityClassFlags::INVISIBLE);
    }
    if attr_bool(reader, event, b"BBoxSelection", "Entity.BBoxSelection")?.unwrap_or(false) {
        flags.insert(EntityClassFlags::BBOX_SELECTION);
    }

    Ok(EntityClassDescriptor {
        name,
        script,
        flags,
    })
}

fn required_attr<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    key: &[u8],
    name: &'static str,
) -> Result<Cow<'a, str>, EntityClassParseError> {
    attr_value(reader, event, key)?.ok_or(EntityClassParseError::MissingAttribute(name))
}

fn attr_value<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    key: &[u8],
) -> Result<Option<Cow<'a, str>>, EntityClassParseError> {
    for attribute in event.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.decoded_and_normalized_value(
                quick_xml::XmlVersion::default(),
                reader.decoder(),
            )?));
        }
    }
    Ok(None)
}

fn attr_bool(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Option<bool>, EntityClassParseError> {
    let Some(value) = attr_value(reader, event, key)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(Some(false));
    }
    if value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes") {
        return Ok(Some(true));
    }
    if value == "0" || value.eq_ignore_ascii_case("false") || value.eq_ignore_ascii_case("no") {
        return Ok(Some(false));
    }
    value
        .parse::<u32>()
        .map(|number| Some(number != 0))
        .map_err(|source| EntityClassParseError::InvalidBoolean {
            name,
            value: value.to_string(),
            source,
        })
}

/// Errors returned while parsing an `.ent` descriptor.
#[derive(Debug, Error)]
pub enum EntityClassParseError {
    #[error("expected Entity root")]
    MissingRoot,
    #[error("unexpected element `{element}`")]
    UnexpectedElement { element: String },
    #[error("missing required attribute `{0}`")]
    MissingAttribute(&'static str),
    #[error("entity class name is empty")]
    EmptyName,
    #[error("invalid boolean `{value}` in `{name}`")]
    InvalidBoolean {
        name: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("xml parse error")]
    Xml(#[from] quick_xml::Error),
    #[error("xml attribute error")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("asset is not utf-8")]
    Utf8(#[from] str::Utf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scripted_invisible_entity_class() {
        let xml = br#"<Entity Name="NavigationSeedPoint" Script="Scripts/Entities/AI/NavigationSeedPoint.lua" Invisible="1"/>"#;

        let descriptor = parse_entity_class_descriptor(xml).unwrap();

        assert_eq!(descriptor.name, "NavigationSeedPoint");
        assert_eq!(
            descriptor.script.as_deref(),
            Some("Scripts/Entities/AI/NavigationSeedPoint.lua")
        );
        assert!(descriptor.is_invisible());
        assert!(!descriptor.has_bbox_selection());

        let summary = summarize_entity_class_descriptor(xml).unwrap();
        assert_eq!(summary.class_name.as_deref(), Some("NavigationSeedPoint"));
        assert_eq!(summary.stats.descriptors, 1);
        assert_eq!(summary.stats.scripted, 1);
        assert_eq!(summary.stats.invisible, 1);

        let mut totals = EntityClassTotals::default();
        totals.add_summary(&summary);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.scripted, 1);
        assert_eq!(summary.to_string(), "NavigationSeedPoint");
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  descriptors: 1\n  scripted: 1\n  invisible: 1\n  bbox selection: 0\n"
        );

        let row =
            inspect_entity_class_descriptor_file("entities/navigationseedpoint.ent", xml).unwrap();
        let mut inspection = EntityClassInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "entities/navigationseedpoint.ent: NavigationSeedPoint\n  files: 1\n  descriptors: 1\n  scripted: 1\n  invisible: 1\n  bbox selection: 0\n"
        );

        let path = std::env::temp_dir().join(format!(
            "az-rs-cry-entity-system-{}-navigationseedpoint.ent",
            std::process::id()
        ));
        std::fs::write(&path, xml).expect("write entity class");
        let inspection =
            inspect_entity_class_descriptor_files([&path]).expect("inspect entity classes");
        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.scripted, 1);
        assert_eq!(inspection.totals.invisible, 1);
        std::fs::remove_file(path).expect("remove entity class");

        assert!(is_entity_class_descriptor_name("NavigationSeedPoint.ENT"));
    }

    #[test]
    fn parses_bbox_selection_flag() {
        let xml = br#"<Entity Name="BoxSelectable" BBoxSelection="true"/>"#;

        let descriptor = parse_entity_class_descriptor(xml).unwrap();

        assert_eq!(
            descriptor.flags,
            EntityClassFlags::from_bits(EntityClassFlags::BBOX_SELECTION.bits())
        );
    }
}
