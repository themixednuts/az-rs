//! Animation event list parsing.
//!
//! Follows Lumberyard's `dev/Gems/CryLegacy/Code/Source/CryAnimation/AnimEventLoader.cpp`
//! and `dev/Gems/CryLegacy/Code/Source/CryAnimation/AnimationManager.cpp`.

use std::{
    borrow::Cow,
    fmt, io,
    num::ParseFloatError,
    path::{Path, PathBuf},
    str,
};

use glam::Vec3;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use thiserror::Error;

use crate::{xml_general_reference_content, xml_text_content};

const ANIM_EVENT_LIST_ROOT: &[u8] = b"anim_event_list";
const ANIMATION_ELEMENT: &[u8] = b"animation";
const EVENT_ELEMENT: &[u8] = b"event";
const PARAMETER_ELEMENT: &[u8] = b"parameter";

/// Summary returned after visiting an animation event list.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AnimationEventListStats {
    pub animations: usize,
    pub events: usize,
}

impl fmt::Display for AnimationEventListStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  animations: {}", self.animations)?;
        writeln!(f, "  events: {}", self.events)
    }
}

/// Borrowed animation row from an `.animevents` asset.
#[derive(Debug, Clone)]
pub struct AnimationRef<'a> {
    pub name: Cow<'a, str>,
}

/// Borrowed event row from an `.animevents` asset.
#[derive(Debug, Clone)]
pub struct AnimationEventRef<'a> {
    pub name: Cow<'a, str>,
    pub time: f32,
    pub end_time: f32,
    pub parameter: Cow<'a, str>,
    pub bone: Cow<'a, str>,
    pub second_bone: Cow<'a, str>,
    pub offset: Vec3,
    pub direction: Vec3,
    pub model: Cow<'a, str>,
}

/// Item produced while visiting an animation event list.
#[derive(Debug, Clone)]
pub enum AnimationEventItem<'a> {
    Animation(AnimationRef<'a>),
    Event(AnimationEventRef<'a>),
}

/// One printable animation event row retained by an inspection.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationEventInspectionRow {
    pub animation: String,
    pub name: String,
    pub time: f32,
    pub end_time: f32,
    pub parameter: String,
}

/// Bounded report for one `.animevents` asset.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationEventListInspection {
    pub source: String,
    pub rows: Vec<AnimationEventInspectionRow>,
    pub stats: AnimationEventListStats,
}

impl fmt::Display for AnimationEventListInspection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.source)?;
        for row in &self.rows {
            writeln!(
                f,
                "    {} @ {:.3}-{:.3}: {} ({})",
                row.animation, row.time, row.end_time, row.name, row.parameter
            )?;
        }
        write!(f, "{}", self.stats)
    }
}

/// Inspect a `.animevents` XML asset while retaining only the first `limit` event rows.
///
/// Rows past `limit` are counted in the stats but not kept.
///
/// # Errors
///
/// Returns any error [`visit_animation_event_list`] returns —
/// [`ParseError::Utf8`] for non-UTF-8 bytes, [`ParseError::Xml`] or
/// [`ParseError::Attribute`] for malformed XML, [`ParseError::MissingRoot`] or
/// [`ParseError::UnexpectedRoot`] for the wrong document shape,
/// [`ParseError::UnexpectedText`], [`ParseError::EventWithoutAnimation`], and
/// [`ParseError::InvalidFloat`] or [`ParseError::InvalidVec3`] for an
/// unparseable attribute.
pub fn inspect_animation_event_list_file(
    source: impl Into<String>,
    bytes: &[u8],
    limit: usize,
) -> Result<AnimationEventListInspection, ParseError> {
    let mut rows = Vec::new();
    let mut current_animation = String::new();

    let stats = visit_animation_event_list(bytes, |item| {
        match item {
            AnimationEventItem::Animation(animation) => {
                current_animation.clear();
                current_animation.push_str(animation.name.as_ref());
            }
            AnimationEventItem::Event(event) if rows.len() < limit => {
                rows.push(AnimationEventInspectionRow {
                    animation: current_animation.clone(),
                    name: event.name.into_owned(),
                    time: event.time,
                    end_time: event.end_time,
                    parameter: event.parameter.into_owned(),
                });
            }
            AnimationEventItem::Event(_) => {}
        }
        Ok(())
    })?;

    Ok(AnimationEventListInspection {
        source: source.into(),
        rows,
        stats,
    })
}

#[derive(Debug, Error)]
pub enum AnimationEventListInspectionError {
    #[error("read animation event list {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse animation event list {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Reads a `.animevents` asset from disk and inspects it.
///
/// # Errors
///
/// Returns [`AnimationEventListInspectionError::Read`] if `path` cannot be
/// read (missing file, permissions), or
/// [`AnimationEventListInspectionError::Parse`] wrapping the [`ParseError`]
/// from a malformed document. Both variants carry the offending path.
pub fn inspect_animation_event_list_path(
    path: impl AsRef<Path>,
    limit: usize,
) -> Result<AnimationEventListInspection, AnimationEventListInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| AnimationEventListInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_animation_event_list_file(path.display().to_string(), &bytes, limit).map_err(|source| {
        AnimationEventListInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Visit a `.animevents` XML asset without materializing the full event list.
///
/// # Errors
///
/// Returns [`ParseError::Utf8`] if `bytes` is not valid UTF-8,
/// [`ParseError::Xml`] or [`ParseError::Attribute`] for a document the reader
/// rejects, [`ParseError::MissingRoot`] for an empty document,
/// [`ParseError::UnexpectedRoot`] when the root is not `anim_event_list`,
/// [`ParseError::UnexpectedText`] for character data between elements,
/// [`ParseError::EventWithoutAnimation`] for an `<event>` outside an
/// `<animation>` row, and [`ParseError::InvalidFloat`] or
/// [`ParseError::InvalidVec3`] when a time or vector attribute does not parse.
/// Any error `visitor` itself returns is propagated unchanged.
pub fn visit_animation_event_list<F>(
    bytes: &[u8],
    mut visitor: F,
) -> Result<AnimationEventListStats, ParseError>
where
    F: FnMut(AnimationEventItem<'_>) -> Result<(), ParseError>,
{
    let xml = str::from_utf8(bytes)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stats = AnimationEventListStats::default();
    let mut depth = 0usize;
    let mut saw_root = false;
    let mut in_animation = false;
    let mut event_depth = None;
    let mut ignored_text_depth = None;

    loop {
        match reader.read_event()? {
            Event::Start(event) => {
                let name = event.name();
                let name = name.as_ref();
                if depth == 0 {
                    ensure_root(name)?;
                    saw_root = true;
                } else if depth == 1 && name == ANIMATION_ELEMENT {
                    stats.animations += 1;
                    in_animation = true;
                    visitor(AnimationEventItem::Animation(parse_animation(
                        &reader, &event,
                    )?))?;
                } else if depth == 2 && in_animation && name == EVENT_ELEMENT {
                    stats.events += 1;
                    visitor(AnimationEventItem::Event(parse_event(&reader, &event)?))?;
                    event_depth = Some(depth + 1);
                } else if event_depth == Some(depth) && name == PARAMETER_ELEMENT {
                    ignored_text_depth = Some(depth + 1);
                }
                depth += 1;
            }
            Event::Empty(event) => {
                let name = event.name();
                let name = name.as_ref();
                if depth == 0 {
                    ensure_root(name)?;
                    saw_root = true;
                } else if depth == 1 && name == ANIMATION_ELEMENT {
                    stats.animations += 1;
                    visitor(AnimationEventItem::Animation(parse_animation(
                        &reader, &event,
                    )?))?;
                } else if depth == 2 && in_animation && name == EVENT_ELEMENT {
                    stats.events += 1;
                    visitor(AnimationEventItem::Event(parse_event(&reader, &event)?))?;
                }
            }
            Event::End(_) => {
                if ignored_text_depth == Some(depth) {
                    ignored_text_depth = None;
                }
                if event_depth == Some(depth) {
                    event_depth = None;
                }
                depth = depth.saturating_sub(1);
                if depth <= 1 {
                    in_animation = false;
                }
            }
            Event::Text(event) => {
                if ignored_text_depth != Some(depth)
                    && !is_harmless_xml_text(&xml_text_content(&event)?)
                {
                    return Err(ParseError::UnexpectedText);
                }
            }
            Event::GeneralRef(event) => {
                if ignored_text_depth != Some(depth)
                    && !is_harmless_xml_text(&xml_general_reference_content(&event)?)
                {
                    return Err(ParseError::UnexpectedText);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !saw_root {
        return Err(ParseError::MissingRoot);
    }

    Ok(stats)
}

/// Animation event parse error.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("asset is not UTF-8 XML")]
    Utf8(#[from] str::Utf8Error),
    #[error("read XML")]
    Xml(#[from] quick_xml::Error),
    #[error("read XML attribute")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("expected root `anim_event_list`, found `{0}`")]
    UnexpectedRoot(String),
    #[error("XML has no root element")]
    MissingRoot,
    #[error("XML text is not expected in animation event lists")]
    UnexpectedText,
    #[error("animation event appeared outside an animation row")]
    EventWithoutAnimation,
    #[error("attribute `{name}` has invalid float `{value}`")]
    InvalidFloat {
        name: &'static str,
        value: String,
        source: ParseFloatError,
    },
    #[error("attribute `{name}` has invalid Vec3 `{value}`")]
    InvalidVec3 { name: &'static str, value: String },
}

fn ensure_root(name: &[u8]) -> Result<(), ParseError> {
    if name == ANIM_EVENT_LIST_ROOT {
        Ok(())
    } else {
        Err(ParseError::UnexpectedRoot(
            String::from_utf8_lossy(name).into_owned(),
        ))
    }
}

fn is_harmless_xml_text(value: &str) -> bool {
    value
        .trim_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}' || ch == '\0')
        .is_empty()
}

fn parse_animation<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
) -> Result<AnimationRef<'a>, ParseError> {
    Ok(AnimationRef {
        name: attr_value(reader, event, b"name")?.unwrap_or(Cow::Borrowed("")),
    })
}

fn parse_event<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
) -> Result<AnimationEventRef<'a>, ParseError> {
    let time = attr_f32(reader, event, b"time", "time")?.unwrap_or(0.0);
    let end_time = attr_f32(reader, event, b"endTime", "endTime")?.unwrap_or(time);
    let offset = attr_vec3(reader, event, b"offset", "offset")?.unwrap_or(Vec3::ZERO);
    let direction = attr_vec3(reader, event, b"dir", "dir")?.unwrap_or(Vec3::ZERO);

    Ok(AnimationEventRef {
        name: attr_value(reader, event, b"name")?.unwrap_or(Cow::Borrowed("__unnamed__")),
        time,
        end_time,
        parameter: attr_value(reader, event, b"parameter")?.unwrap_or(Cow::Borrowed("")),
        bone: attr_value(reader, event, b"bone")?.unwrap_or(Cow::Borrowed("")),
        second_bone: attr_value(reader, event, b"secondBone")?.unwrap_or(Cow::Borrowed("")),
        offset,
        direction,
        model: attr_value(reader, event, b"model")?.unwrap_or(Cow::Borrowed("")),
    })
}

fn attr_value<'a>(
    reader: &Reader<&[u8]>,
    event: &'a BytesStart<'a>,
    key: &[u8],
) -> Result<Option<Cow<'a, str>>, ParseError> {
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

fn attr_f32(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Option<f32>, ParseError> {
    let Some(value) = attr_value(reader, event, key)? else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|source| ParseError::InvalidFloat {
            name,
            value: value.into_owned(),
            source,
        })
}

fn attr_vec3(
    reader: &Reader<&[u8]>,
    event: &BytesStart<'_>,
    key: &[u8],
    name: &'static str,
) -> Result<Option<Vec3>, ParseError> {
    let Some(value) = attr_value(reader, event, key)? else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    parse_vec3(&value)
        .map(Some)
        .map_err(|()| ParseError::InvalidVec3 {
            name,
            value: value.into_owned(),
        })
}

fn parse_vec3(value: &str) -> Result<Vec3, ()> {
    let mut parts = value.split(',').map(str::trim);
    let x = parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let y = parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let z = parts.next().ok_or(())?.parse().map_err(|_| ())?;
    if parts.next().is_some() {
        return Err(());
    }
    Ok(Vec3::new(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_animation_events() {
        let xml = br#"
            <anim_event_list>
              <animation name="animations/foo.caf">
                <event name="footstep" time="0.25" endTime="0.5" parameter="FTSP" bone="Bip01" secondBone="" offset="1,2,3" dir="0,1,0" model=""/>
              </animation>
            </anim_event_list>
        "#;
        let mut events = Vec::new();

        let stats = visit_animation_event_list(xml, |item| {
            if let AnimationEventItem::Event(event) = item {
                events.push((
                    event.name.into_owned(),
                    event.time,
                    event.end_time,
                    event.parameter.into_owned(),
                    event.offset,
                    event.direction,
                ));
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(
            stats,
            AnimationEventListStats {
                animations: 1,
                events: 1,
            }
        );
        assert_eq!(stats.to_string(), "  animations: 1\n  events: 1\n");
        assert_eq!(events[0].0, "footstep");
        // Bit-exact: both values are representable, so the parse must
        // reproduce them exactly rather than within a tolerance.
        assert_eq!(events[0].1.to_bits(), 0.25_f32.to_bits());
        assert_eq!(events[0].2.to_bits(), 0.5_f32.to_bits());
        assert_eq!(events[0].3, "FTSP");
        assert_eq!(events[0].4, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(events[0].5, Vec3::Y);

        let inspection =
            inspect_animation_event_list_file("animations/foo.animevents", xml, 20).unwrap();
        assert_eq!(
            inspection.to_string(),
            concat!(
                "animations/foo.animevents\n",
                "    animations/foo.caf @ 0.250-0.500: footstep (FTSP)\n",
                "  animations: 1\n",
                "  events: 1\n",
            )
        );
    }

    #[test]
    fn applies_lumberyard_event_defaults() {
        let xml = br"<anim_event_list><animation><event /></animation></anim_event_list>";
        let mut event_name = String::new();
        let mut time = -1.0;
        let mut end_time = -1.0;

        visit_animation_event_list(xml, |item| {
            if let AnimationEventItem::Event(event) = item {
                event_name = event.name.into_owned();
                time = event.time;
                end_time = event.end_time;
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(event_name, "__unnamed__");
        // Bit-exact: an absent time attribute must default to positive zero,
        // which `-0.0 == 0.0` would not distinguish.
        assert_eq!(time.to_bits(), 0.0_f32.to_bits());
        assert_eq!(end_time.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn ignores_harmless_text_nodes() {
        let xml = b"\xEF\xBB\xBF<anim_event_list>\0\n <animation name=\"animations/foo.caf\">\n  \0\n </animation>\0\n</anim_event_list>\0";

        let stats = visit_animation_event_list(xml, |_| Ok(())).unwrap();

        assert_eq!(
            stats,
            AnimationEventListStats {
                animations: 1,
                events: 0,
            }
        );
    }

    #[test]
    fn ignores_legacy_event_parameter_children() {
        let xml = br#"
            <anim_event_list>
              <animation name="animations/foo.caf">
                <event name="materialeffect" time="0.25" parameter="" bone="Hand_right" offset="0,0,0" dir="0,0,0" model="">
                  <parameter>interactables</parameter>
                  <parameter>chopping_1H</parameter>
                </event>
              </animation>
            </anim_event_list>
        "#;
        let mut parameter = String::from("missing");

        let stats = visit_animation_event_list(xml, |item| {
            if let AnimationEventItem::Event(event) = item {
                parameter = event.parameter.into_owned();
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(
            stats,
            AnimationEventListStats {
                animations: 1,
                events: 1,
            }
        );
        assert_eq!(parameter, "");
    }

    #[test]
    fn rejects_non_harmless_text_nodes() {
        let err = visit_animation_event_list(
            b"<anim_event_list><unsupported>payload</unsupported></anim_event_list>",
            |_| Ok(()),
        )
        .unwrap_err();

        assert!(matches!(err, ParseError::UnexpectedText));
    }

    #[test]
    fn rejects_wrong_root() {
        let err = visit_animation_event_list(b"<not_anim_events/>", |_| Ok(())).unwrap_err();

        assert!(matches!(err, ParseError::UnexpectedRoot(root) if root == "not_anim_events"));
    }
}
