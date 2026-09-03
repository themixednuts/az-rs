//! `LyShine` asset parsing.
//!
//! Follows O3DE's `Gems/LyShine/Code/Source/Sprite.cpp`.

mod font;

use std::{
    borrow::Cow,
    fmt, io,
    num::{ParseFloatError, ParseIntError},
    path::{Path, PathBuf},
    str,
};

use glam::Vec2;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use thiserror::Error;

pub use font::*;

const SPRITE: &[u8] = b"Sprite";
const SPRITE_SHEET: &[u8] = b"SpriteSheet";
const CELL: &[u8] = b"Cell";

/// File extension used by `LyShine` sprite sidecars.
pub const SPRITE_EXTENSION: &str = "sprite";

/// Latest `LyShine` sprite sidecar version supported by this reader.
pub const SPRITE_FILE_VERSION: u32 = 2;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SpriteInspectionError {
    #[error("read {path:?}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("parse LyShine sprite {path:?}: {source}")]
    Parse {
        path: PathBuf,
        source: SpriteParseError,
    },
}

/// Summary returned after visiting a `LyShine` `.sprite` asset.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SpriteStats {
    pub sprites: usize,
    pub bordered_sprites: usize,
    pub sprite_sheets: usize,
    pub cells: usize,
    pub cell_borders: usize,
}

impl fmt::Display for SpriteStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} sheets, {} cells, {} root borders, {} cell borders",
            self.sprite_sheets, self.cells, self.bordered_sprites, self.cell_borders
        )
    }
}

/// Aggregate summary across many `LyShine` `.sprite` sidecars.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SpriteTotals {
    pub files: usize,
    pub sprites: usize,
    pub bordered_sprites: usize,
    pub sprite_sheets: usize,
    pub cells: usize,
    pub cell_borders: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteFileSummary {
    pub source: String,
    pub stats: SpriteStats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SpriteInspection {
    pub rows: Vec<SpriteFileSummary>,
    pub totals: SpriteTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct SpriteInspectionReport<'a> {
    inspection: &'a SpriteInspection,
    limit: usize,
}

impl SpriteTotals {
    pub const fn add(&mut self, stats: SpriteStats) {
        self.files += 1;
        self.sprites += stats.sprites;
        self.bordered_sprites += stats.bordered_sprites;
        self.sprite_sheets += stats.sprite_sheets;
        self.cells += stats.cells;
        self.cell_borders += stats.cell_borders;
    }
}

impl SpriteInspection {
    pub fn add_file_summary(&mut self, row: SpriteFileSummary) {
        self.totals.add(row.stats);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> SpriteInspectionReport<'_> {
        SpriteInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for SpriteTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  sprites: {}", self.sprites)?;
        writeln!(f, "  bordered sprites: {}", self.bordered_sprites)?;
        writeln!(f, "  sprite sheets: {}", self.sprite_sheets)?;
        writeln!(f, "  cells: {}", self.cells)?;
        writeln!(f, "  cell borders: {}", self.cell_borders)
    }
}

impl fmt::Display for SpriteInspectionReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.limit > 0 {
            for row in self.inspection.rows.iter().take(self.limit) {
                writeln!(f, "{}: {}", row.source, row.stats)?;
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

/// Parse a `LyShine` `.sprite` sidecar and return its deterministic summary.
///
/// # Errors
///
/// Returns any error [`visit_sprite`] returns.
pub fn summarize_sprite(bytes: &[u8]) -> Result<SpriteStats, SpriteParseError> {
    visit_sprite(bytes, |_| Ok(()))
}

/// Inspect one `LyShine` `.sprite` sidecar with its display source.
///
/// # Errors
///
/// Returns any error [`summarize_sprite`] returns.
pub fn inspect_sprite_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<SpriteFileSummary, SpriteParseError> {
    Ok(SpriteFileSummary {
        source: path.as_ref().display().to_string(),
        stats: summarize_sprite(bytes)?,
    })
}

/// Read and summarize one `.sprite` sidecar from disk.
///
/// # Errors
///
/// Returns [`SpriteInspectionError::Read`] when `path` cannot be read, and
/// [`SpriteInspectionError::Parse`] when its contents are not a valid
/// `LyShine` sprite sidecar.
pub fn inspect_sprite_path(
    path: impl AsRef<Path>,
) -> Result<SpriteFileSummary, SpriteInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| SpriteInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_sprite_file(path, &bytes).map_err(|source| SpriteInspectionError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Read and aggregate every `.sprite` sidecar in `paths`.
///
/// # Errors
///
/// Returns the first error [`inspect_sprite_path`] returns; the walk stops at
/// that path and the partial inspection is discarded.
pub fn inspect_sprite_files<I, P>(paths: I) -> Result<SpriteInspection, SpriteInspectionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut inspection = SpriteInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_sprite_path(path)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_sprite_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(SPRITE_EXTENSION)
}

#[must_use]
pub fn is_sprite_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_sprite_extension(extension))
}

#[must_use]
pub fn is_sprite_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_sprite_extension)
}

/// Item produced by the streaming sprite visitor.
#[derive(Debug, Clone)]
pub enum SpriteItem<'a> {
    Asset(SpriteAssetRef),
    Borders(SpriteBordersRef),
    Cell(SpriteCellRef<'a>),
}

/// Borrowed view of the root `<Sprite>` element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteAssetRef {
    pub version: u32,
}

/// Border data from a root sprite or a sprite-sheet cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteBordersRef {
    pub context: SpriteContext,
    pub borders: SpriteBorders,
}

/// Borrowed view of one `<SpriteSheet>/<Cell>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct SpriteCellRef<'a> {
    pub alias: Option<Cow<'a, str>>,
    pub uv: SpriteCellUv,
}

/// Source location for parsed border data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteContext {
    Root,
    Cell,
}

/// Normalized 9-slice border positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteBorders {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl SpriteBorders {
    pub const DEFAULT: Self = Self {
        left: 0.0,
        right: 1.0,
        top: 0.0,
        bottom: 1.0,
    };

    #[must_use]
    pub const fn new(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    #[must_use]
    pub const fn is_default(self) -> bool {
        self.left == Self::DEFAULT.left
            && self.right == Self::DEFAULT.right
            && self.top == Self::DEFAULT.top
            && self.bottom == Self::DEFAULT.bottom
    }
}

impl Default for SpriteBorders {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Normalized UV coordinates for one sprite-sheet cell.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SpriteCellUv {
    pub top_left: Vec2,
    pub top_right: Vec2,
    pub bottom_right: Vec2,
    pub bottom_left: Vec2,
}

impl SpriteCellUv {
    #[must_use]
    pub const fn new(
        top_left: Vec2,
        top_right: Vec2,
        bottom_right: Vec2,
        bottom_left: Vec2,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }
}

/// Parse a `LyShine` `.sprite` sidecar with a streaming visitor.
///
/// # Errors
///
/// Returns [`SpriteParseError::Utf8`] when `bytes` is not UTF-8,
/// [`SpriteParseError::Xml`] or [`SpriteParseError::Attribute`] for malformed
/// XML, [`SpriteParseError::MissingRoot`] when no `<Sprite>` root was seen,
/// [`SpriteParseError::UnsupportedVersion`] for a `versionNumber` outside the
/// range `1` to [`SPRITE_FILE_VERSION`], [`SpriteParseError::UnexpectedElement`] for
/// an element the sprite schema does not allow, and
/// [`SpriteParseError::InvalidFloat`], [`SpriteParseError::InvalidInteger`],
/// or [`SpriteParseError::InvalidVector`] for an attribute that does not parse.
/// Any error `visitor` returns is propagated unchanged.
pub fn visit_sprite<F>(bytes: &[u8], mut visitor: F) -> Result<SpriteStats, SpriteParseError>
where
    F: FnMut(SpriteItem<'_>) -> Result<(), SpriteParseError>,
{
    let xml = str::from_utf8(bytes)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut state = SpriteState::default();

    loop {
        match reader.read_event()? {
            Event::Start(event) => {
                let element = state.visit_start(&reader, &event, &mut visitor)?;
                state.stack.push(element);
            }
            Event::Empty(event) => {
                // An empty element opens and closes in one event, so it never
                // joins the stack.
                state.visit_start(&reader, &event, &mut visitor)?;
            }
            Event::End(_) => {
                state.stack.pop();
            }
            Event::Eof => break,
            Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::Comment(_)
            | Event::GeneralRef(_) => {}
        }
    }

    if state.stats.sprites == 0 {
        return Err(SpriteParseError::MissingRoot);
    }

    Ok(state.stats)
}

#[derive(Debug, Default)]
struct SpriteState {
    stack: Vec<ElementKind>,
    stats: SpriteStats,
}

impl SpriteState {
    fn visit_start<F>(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        visitor: &mut F,
    ) -> Result<ElementKind, SpriteParseError>
    where
        F: FnMut(SpriteItem<'_>) -> Result<(), SpriteParseError>,
    {
        let parent = self.stack.last().copied();

        match parent {
            None => self.visit_root(reader, event, visitor),
            Some(ElementKind::Root) => self.visit_root_child(reader, event, visitor),
            Some(ElementKind::SpriteSheet) => self.visit_cell(reader, event, visitor),
            Some(ElementKind::Cell) => self.visit_cell_child(reader, event, visitor),
            Some(ElementKind::RootBorders | ElementKind::CellBorders | ElementKind::Unknown) => {
                Ok(ElementKind::Unknown)
            }
        }
    }

    fn visit_root<F>(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        visitor: &mut F,
    ) -> Result<ElementKind, SpriteParseError>
    where
        F: FnMut(SpriteItem<'_>) -> Result<(), SpriteParseError>,
    {
        let name = event.local_name();
        ensure_element(b"<root>", SPRITE, name.as_ref())?;

        let version = attr_u32(reader, event, &[b"versionNumber"], "versionNumber")?
            .unwrap_or(SPRITE_FILE_VERSION);
        if !(1..=SPRITE_FILE_VERSION).contains(&version) {
            return Err(SpriteParseError::UnsupportedVersion(version));
        }

        self.stats.sprites += 1;
        visitor(SpriteItem::Asset(SpriteAssetRef { version }))?;
        Ok(ElementKind::Root)
    }

    fn visit_root_child<F>(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        visitor: &mut F,
    ) -> Result<ElementKind, SpriteParseError>
    where
        F: FnMut(SpriteItem<'_>) -> Result<(), SpriteParseError>,
    {
        let name = event.local_name();
        match name.as_ref() {
            SPRITE => {
                let borders = parse_borders(reader, event)?;
                self.stats.bordered_sprites += 1;
                visitor(SpriteItem::Borders(SpriteBordersRef {
                    context: SpriteContext::Root,
                    borders,
                }))?;
                Ok(ElementKind::RootBorders)
            }
            SPRITE_SHEET => {
                self.stats.sprite_sheets += 1;
                Ok(ElementKind::SpriteSheet)
            }
            _ => Err(SpriteParseError::UnexpectedElement {
                parent: "Sprite".to_string(),
                child: String::from_utf8_lossy(name.as_ref()).into_owned(),
            }),
        }
    }

    fn visit_cell<F>(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        visitor: &mut F,
    ) -> Result<ElementKind, SpriteParseError>
    where
        F: FnMut(SpriteItem<'_>) -> Result<(), SpriteParseError>,
    {
        let name = event.local_name();
        ensure_element(SPRITE_SHEET, CELL, name.as_ref())?;

        let cell = SpriteCellRef {
            alias: attr_value(reader, event, &[b"alias"])?,
            uv: SpriteCellUv {
                top_left: attr_vec2(reader, event, &[b"topLeft"], "topLeft")?.unwrap_or(Vec2::ZERO),
                top_right: attr_vec2(reader, event, &[b"topRight"], "topRight")?
                    .unwrap_or(Vec2::ZERO),
                bottom_right: attr_vec2(reader, event, &[b"bottomRight"], "bottomRight")?
                    .unwrap_or(Vec2::ZERO),
                bottom_left: attr_vec2(reader, event, &[b"bottomLeft"], "bottomLeft")?
                    .unwrap_or(Vec2::ZERO),
            },
        };
        self.stats.cells += 1;
        visitor(SpriteItem::Cell(cell))?;
        Ok(ElementKind::Cell)
    }

    fn visit_cell_child<F>(
        &mut self,
        reader: &Reader<&[u8]>,
        event: &BytesStart<'_>,
        visitor: &mut F,
    ) -> Result<ElementKind, SpriteParseError>
    where
        F: FnMut(SpriteItem<'_>) -> Result<(), SpriteParseError>,
    {
        let name = event.local_name();
        ensure_element(CELL, SPRITE, name.as_ref())?;

        let borders = parse_borders(reader, event)?;
        self.stats.cell_borders += 1;
        visitor(SpriteItem::Borders(SpriteBordersRef {
            context: SpriteContext::Cell,
            borders,
        }))?;
        Ok(ElementKind::CellBorders)
    }
}

#[derive(Debug, Clone, Copy)]
enum ElementKind {
    Root,
    RootBorders,
    SpriteSheet,
    Cell,
    CellBorders,
    Unknown,
}

fn parse_borders(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
) -> Result<SpriteBorders, SpriteParseError> {
    Ok(SpriteBorders {
        left: attr_f32(reader, event, &[b"m_left"], "m_left")?
            .unwrap_or(SpriteBorders::DEFAULT.left),
        right: attr_f32(reader, event, &[b"m_right"], "m_right")?
            .unwrap_or(SpriteBorders::DEFAULT.right),
        top: attr_f32(reader, event, &[b"m_top"], "m_top")?.unwrap_or(SpriteBorders::DEFAULT.top),
        bottom: attr_f32(reader, event, &[b"m_bottom"], "m_bottom")?
            .unwrap_or(SpriteBorders::DEFAULT.bottom),
    })
}

fn attr_value<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    keys: &[&[u8]],
) -> Result<Option<Cow<'a, str>>, SpriteParseError> {
    for attribute in event.attributes() {
        let attribute = attribute?;
        if keys
            .iter()
            .any(|key| attribute.key.as_ref().eq_ignore_ascii_case(key))
        {
            return Ok(Some(attribute.decoded_and_normalized_value(
                quick_xml::XmlVersion::default(),
                reader.decoder(),
            )?));
        }
    }
    Ok(None)
}

fn attr_f32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    keys: &[&[u8]],
    name: &'static str,
) -> Result<Option<f32>, SpriteParseError> {
    let Some(value) = attr_value(reader, event, keys)? else {
        return Ok(None);
    };
    parse_optional_f32(name, value.as_ref()).map(Some)
}

fn attr_u32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    keys: &[&[u8]],
    name: &'static str,
) -> Result<Option<u32>, SpriteParseError> {
    let Some(value) = attr_value(reader, event, keys)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|source| SpriteParseError::InvalidInteger {
            name,
            value: value.to_string(),
            source,
        })
}

fn attr_vec2(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    keys: &[&[u8]],
    name: &'static str,
) -> Result<Option<Vec2>, SpriteParseError> {
    let Some(value) = attr_value(reader, event, keys)? else {
        return Ok(None);
    };
    parse_vec2(name, value.as_ref()).map(Some)
}

fn parse_optional_f32(name: &'static str, value: &str) -> Result<f32, SpriteParseError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    value
        .parse()
        .map_err(|source| SpriteParseError::InvalidFloat {
            name,
            value: value.to_string(),
            source,
        })
}

fn parse_vec2(name: &'static str, value: &str) -> Result<Vec2, SpriteParseError> {
    let mut parts = value
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty());

    let Some(x) = parts.next() else {
        return Err(SpriteParseError::InvalidVector {
            name,
            value: value.to_string(),
        });
    };
    let Some(y) = parts.next() else {
        return Err(SpriteParseError::InvalidVector {
            name,
            value: value.to_string(),
        });
    };

    let x = x.parse().map_err(|source| SpriteParseError::InvalidFloat {
        name,
        value: value.to_string(),
        source,
    })?;
    let y = y.parse().map_err(|source| SpriteParseError::InvalidFloat {
        name,
        value: value.to_string(),
        source,
    })?;
    Ok(Vec2::new(x, y))
}

fn ensure_element(parent: &[u8], expected: &[u8], actual: &[u8]) -> Result<(), SpriteParseError> {
    if actual == expected {
        Ok(())
    } else {
        Err(SpriteParseError::UnexpectedElement {
            parent: String::from_utf8_lossy(parent).into_owned(),
            child: String::from_utf8_lossy(actual).into_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum SpriteParseError {
    #[error("expected Sprite root")]
    MissingRoot,
    #[error("unsupported sprite version `{0}`")]
    UnsupportedVersion(u32),
    #[error("unexpected element `{child}` under `{parent}`")]
    UnexpectedElement { parent: String, child: String },
    #[error("invalid float `{value}` in `{name}`")]
    InvalidFloat {
        name: &'static str,
        value: String,
        #[source]
        source: ParseFloatError,
    },
    #[error("invalid integer `{value}` in `{name}`")]
    InvalidInteger {
        name: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("invalid vector `{value}` in `{name}`")]
    InvalidVector { name: &'static str, value: String },
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
    fn visits_bordered_sprite() {
        let xml = br#"
            <Sprite versionNumber="2">
                <Sprite m_left="0.25" m_right="0.75" m_top="0.125" m_bottom="0.875"/>
            </Sprite>
        "#;

        let mut borders = None;
        let stats = visit_sprite(xml, |item| {
            if let SpriteItem::Borders(value) = item {
                borders = Some(value);
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(
            stats,
            SpriteStats {
                sprites: 1,
                bordered_sprites: 1,
                sprite_sheets: 0,
                cells: 0,
                cell_borders: 0,
            }
        );
        assert_eq!(
            borders,
            Some(SpriteBordersRef {
                context: SpriteContext::Root,
                borders: SpriteBorders::new(0.25, 0.75, 0.125, 0.875),
            })
        );

        let mut totals = SpriteTotals::default();
        totals.add(stats);
        assert_eq!(totals.files, 1);
        assert_eq!(totals.bordered_sprites, 1);
        assert_eq!(
            stats.to_string(),
            "0 sheets, 0 cells, 1 root borders, 0 cell borders"
        );
        assert_eq!(
            totals.to_string(),
            "  files: 1\n  sprites: 1\n  bordered sprites: 1\n  sprite sheets: 0\n  cells: 0\n  cell borders: 0\n"
        );
        assert_eq!(summarize_sprite(xml).unwrap(), stats);

        let row = inspect_sprite_file("ui/hud.sprite", xml).unwrap();
        let mut inspection = SpriteInspection::default();
        inspection.add_file_summary(row);
        assert_eq!(
            inspection.report(20).to_string(),
            "ui/hud.sprite: 0 sheets, 0 cells, 1 root borders, 0 cell borders\n  files: 1\n  sprites: 1\n  bordered sprites: 1\n  sprite sheets: 0\n  cells: 0\n  cell borders: 0\n"
        );
    }

    #[test]
    fn inspect_sprite_files_aggregates_file_results() {
        let path =
            std::env::temp_dir().join(format!("az-rs-lyshine-{}-hud.sprite", std::process::id()));
        std::fs::write(
            &path,
            br#"<Sprite versionNumber="2"><Sprite m_left="0.25" m_right="0.75"/></Sprite>"#,
        )
        .expect("write sprite");

        let inspection = inspect_sprite_files([&path]).expect("inspect sprite files");

        assert_eq!(inspection.rows.len(), 1);
        assert_eq!(inspection.totals.files, 1);
        assert_eq!(inspection.totals.sprites, 1);
        assert_eq!(inspection.totals.bordered_sprites, 1);

        std::fs::remove_file(path).expect("remove sprite");
    }

    #[test]
    fn visits_sprite_sheet_cells() {
        let xml = br#"
            <Sprite versionNumber="2">
                <SpriteSheet>
                    <Cell topRight="0.5,0" bottomRight="0.5 1" bottomLeft="0,1">
                        <Sprite m_right="1" m_bottom="1"/>
                    </Cell>
                </SpriteSheet>
            </Sprite>
        "#;

        let mut cell_uv = None;
        let mut cell_borders = None;
        let stats = visit_sprite(xml, |item| {
            match item {
                SpriteItem::Cell(value) => cell_uv = Some(value.uv),
                SpriteItem::Borders(value) if value.context == SpriteContext::Cell => {
                    cell_borders = Some(value.borders);
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(stats.sprite_sheets, 1);
        assert_eq!(stats.cells, 1);
        assert_eq!(stats.cell_borders, 1);

        let cell_uv = cell_uv.unwrap();
        assert_eq!(cell_uv.top_left, Vec2::ZERO);
        assert_eq!(cell_uv.top_right, Vec2::new(0.5, 0.0));
        assert_eq!(cell_uv.bottom_right, Vec2::new(0.5, 1.0));
        assert_eq!(cell_uv.bottom_left, Vec2::new(0.0, 1.0));
        assert_eq!(cell_borders, Some(SpriteBorders::DEFAULT));
    }

    #[test]
    fn rejects_unsupported_versions() {
        let xml = br#"<Sprite versionNumber="3"/>"#;
        let error = visit_sprite(xml, |_| Ok(())).unwrap_err();

        assert!(matches!(error, SpriteParseError::UnsupportedVersion(3)));
    }

    #[test]
    fn recognizes_sprite_paths() {
        assert!(is_sprite_name("button.SPRITE"));
        assert!(is_sprite_path(Path::new("lyshineui/button.sprite")));
        assert!(!is_sprite_name("button.png"));
    }
}
