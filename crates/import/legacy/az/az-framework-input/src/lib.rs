//! `AzFramework` input binding asset parsing.

use std::{
    fmt, io,
    path::{Path, PathBuf},
};

use az_derive::AzRtti;
use az_objectstream::context::{ContainerShape, ObjectStreamReadContext};
use az_objectstream::value::{self, FieldCursor, ObjectStreamValueError};
use az_objectstream::{Element, ObjectStream, ObjectStreamError};
use thiserror::Error;
use uuid::{Uuid, uuid};

pub mod source_transform;
pub use source_transform::{
    INPUT_BINDINGS_SOURCE_SCHEMA, InputBindingGeneratorSource, InputBindingsSource,
    InputBindingsSourceTransform, InputBindingsSourceTransformError, input_bindings_source_path,
    is_legacy_input_bindings_source,
};

const INPUT_EVENT_BINDINGS_ASSET: &str = "InputEventBindingsAsset";
const INPUT_EVENT_BINDINGS: &str = "InputEventBindings";
const INPUT_EVENT_GROUP: &str = "InputEventGroup";
const ANALOG: &str = "Analog";
const SINGLE_EVENT_TO_ACTION: &str = "SingleEventToAction";
const BINDINGS_FIELD: &str = "Bindings";
const INPUT_EVENT_GROUPS_FIELD: &str = "Input Event Groups";
const EVENT_NAME_FIELD: &str = "Event Name";
const EVENT_GENERATORS_FIELD: &str = "Event Generators";
const EXCLUDE_FROM_RELEASE_FIELD: &str = "Exclude From Release";
const BASE_CLASS_FIELD: &str = "BaseClass1";
const INPUT_DEVICE_TYPE_FIELD: &str = "Input Device Type";
const INPUT_NAME_FIELD: &str = "Input Name";
const EVENT_VALUE_MULTIPLIER_FIELD: &str = "Event Value Multiplier";
const SEND_CONTINUOUS_UPDATES_FIELD: &str = "Send Continuous Updates";
const DEAD_ZONE_FIELD: &str = "Dead Zone";

pub const INPUT_EVENT_BINDINGS_ASSET_TYPE_ID: Uuid = uuid!("25971c7a-26e2-4d08-a146-2efcc1c36b0c");
pub const INPUT_EVENT_BINDINGS_TYPE_ID: Uuid = uuid!("14ffd4a8-ae46-4e23-b45b-6a7c4f787a91");
pub const INPUT_EVENT_GROUP_TYPE_ID: Uuid = uuid!("25143b7e-2fec-4cc5-92fe-270b67e79734");
pub const ANALOG_TYPE_ID: Uuid = uuid!("806f21d9-11ea-47fc-8b89-fdb67aade4ff");
pub const SINGLE_EVENT_TO_ACTION_TYPE_ID: Uuid = uuid!("2c93824d-d011-459c-b12b-9f4a6148730c");
pub const AZSTD_VECTOR_LEGACY_XML_TYPE_ID: Uuid = uuid!("2bade35a-6f1b-4698-b2bc-3373d010020c");
pub const INPUT_BINDINGS_EXTENSION: &str = "inputbindings";

#[derive(AzRtti, Debug, Clone, PartialEq)]
#[az_rtti(
    name = "InputEventBindingsAsset",
    INPUT_EVENT_BINDINGS_ASSET_TYPE_ID,
    register
)]
pub struct InputEventBindingsAsset {
    bindings: InputEventBindings,
}

impl InputEventBindingsAsset {
    /// Parse an `.inputbindings` asset.
    ///
    /// # Errors
    ///
    /// Returns an error when the `ObjectStream` XML envelope, root type,
    /// or reflected input binding fields are invalid.
    pub fn parse(
        bytes: &[u8],
        context: &ObjectStreamReadContext,
    ) -> Result<Self, InputBindingsParseError> {
        let stream = ObjectStream::from_bytes_with_context(bytes, context)?;
        Self::from_stream(&stream)
    }

    fn from_stream(stream: &ObjectStream) -> Result<Self, InputBindingsParseError> {
        if stream.version != 1 {
            return Err(InputBindingsParseError::UnsupportedObjectStreamVersion {
                version: stream.version,
            });
        }
        let root = single_root(&stream.elements)?;
        expect_type(
            INPUT_EVENT_BINDINGS_ASSET,
            root,
            INPUT_EVENT_BINDINGS_ASSET_TYPE_ID,
        )?;
        let bindings = required_child(root, INPUT_EVENT_BINDINGS_ASSET, BINDINGS_FIELD)
            .and_then(parse_input_event_bindings)
            .map_err(|source| source.with_owner(INPUT_EVENT_BINDINGS_ASSET))?;
        Ok(Self::new(bindings))
    }

    #[inline]
    #[must_use]
    pub const fn new(bindings: InputEventBindings) -> Self {
        Self { bindings }
    }

    #[inline]
    #[must_use]
    pub const fn bindings(&self) -> &InputEventBindings {
        &self.bindings
    }

    #[must_use]
    pub fn summary(&self) -> InputEventBindingsSummary {
        InputEventBindingsSummary::from_asset(self)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputEventBindingsSummary {
    pub event_groups: usize,
    pub generators: usize,
    pub release_excluded_groups: usize,
}

impl InputEventBindingsSummary {
    #[must_use]
    pub fn from_asset(asset: &InputEventBindingsAsset) -> Self {
        let bindings = asset.bindings();
        Self {
            event_groups: bindings.event_groups().len(),
            generators: bindings.generator_count(),
            release_excluded_groups: bindings
                .event_groups()
                .iter()
                .filter(|group| group.exclude_from_release)
                .count(),
        }
    }
}

impl fmt::Display for InputEventBindingsSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} event groups, {} generators, {} release-excluded",
            self.event_groups, self.generators, self.release_excluded_groups
        )
    }
}

/// Counts the event groups, generators and release-excluded groups in an
/// `.inputbindings` `ObjectStream`.
///
/// # Errors
///
/// Returns any error [`InputEventBindingsAsset::parse`] returns —
/// [`InputBindingsParseError::ObjectStream`] or
/// [`InputBindingsParseError::Value`] when the underlying stream or one of its
/// values will not decode,
/// [`InputBindingsParseError::UnsupportedObjectStreamVersion`] for a stream
/// version this reader does not accept,
/// [`InputBindingsParseError::MissingRoot`] or
/// [`InputBindingsParseError::MultipleRoots`] when the document does not hold
/// exactly one bindings root, and
/// [`InputBindingsParseError::UnexpectedType`],
/// [`InputBindingsParseError::MissingField`] or
/// [`InputBindingsParseError::UnexpectedField`] when a reflected class does not
/// match the layout this reader expects.
pub fn summarize_input_event_bindings(
    bytes: &[u8],
    context: &ObjectStreamReadContext,
) -> Result<InputEventBindingsSummary, InputBindingsParseError> {
    InputEventBindingsAsset::parse(bytes, context).map(|asset| asset.summary())
}

/// The AZ types this crate registers, for a host contribution to register.
///
/// These are the legacy `inputbindings` `ObjectStream` classes, so a host that
/// reads one needs the identities to resolve what a stream names.
#[must_use]
pub const fn types() -> [az_core::AzTypeRegistration; 5] {
    [
        AnalogInputEventGenerator::REGISTRATION,
        InputEventBindings::REGISTRATION,
        InputEventBindingsAsset::REGISTRATION,
        InputEventGroup::REGISTRATION,
        SingleEventToAction::REGISTRATION,
    ]
}

/// Register this crate's AZ types into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<az_core::AzTypeRegistration>()
        .register_many(types());
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputEventBindingsTotals {
    pub files: usize,
    pub event_groups: usize,
    pub generators: usize,
    pub release_excluded_groups: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEventBindingsFileSummary {
    pub source: String,
    pub summary: InputEventBindingsSummary,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InputEventBindingsInspection {
    pub rows: Vec<InputEventBindingsFileSummary>,
    pub totals: InputEventBindingsTotals,
}

#[derive(Debug, Clone, Copy)]
pub struct InputEventBindingsInspectionReport<'a> {
    inspection: &'a InputEventBindingsInspection,
    limit: usize,
}

impl InputEventBindingsTotals {
    pub const fn add_summary(&mut self, summary: InputEventBindingsSummary) {
        self.files += 1;
        self.event_groups += summary.event_groups;
        self.generators += summary.generators;
        self.release_excluded_groups += summary.release_excluded_groups;
    }
}

impl InputEventBindingsInspection {
    pub fn add_file_summary(&mut self, row: InputEventBindingsFileSummary) {
        self.totals.add_summary(row.summary);
        self.rows.push(row);
    }

    #[must_use]
    pub const fn report(&self, limit: usize) -> InputEventBindingsInspectionReport<'_> {
        InputEventBindingsInspectionReport {
            inspection: self,
            limit,
        }
    }
}

impl fmt::Display for InputEventBindingsTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  files: {}", self.files)?;
        writeln!(f, "  event groups: {}", self.event_groups)?;
        writeln!(f, "  generators: {}", self.generators)?;
        writeln!(
            f,
            "  release-excluded groups: {}",
            self.release_excluded_groups
        )
    }
}

impl fmt::Display for InputEventBindingsInspectionReport<'_> {
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

/// Summarises one bindings document's bytes, labelling the row with `path`.
///
/// `path` is only the display label; it is not read from disk.
///
/// # Errors
///
/// Returns any error [`summarize_input_event_bindings`] returns — the
/// [`InputBindingsParseError`] variants for an undecodable stream, an
/// unsupported version, a missing or duplicated root, or a reflected class
/// whose type or fields do not match the expected layout.
pub fn inspect_input_event_bindings_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
    context: &ObjectStreamReadContext,
) -> Result<InputEventBindingsFileSummary, InputBindingsParseError> {
    Ok(InputEventBindingsFileSummary {
        source: path.as_ref().display().to_string(),
        summary: summarize_input_event_bindings(bytes, context)?,
    })
}

#[derive(Debug, Error)]
pub enum InputEventBindingsInspectionError {
    #[error("read input event bindings {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse input event bindings {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: InputBindingsParseError,
    },
}

/// Reads a bindings document from disk and summarises it.
///
/// # Errors
///
/// Returns [`InputEventBindingsInspectionError::Read`] if `path` cannot be
/// read (missing file, permissions), or
/// [`InputEventBindingsInspectionError::Parse`] wrapping the
/// [`InputBindingsParseError`] from a malformed document. Both variants carry
/// the offending path.
pub fn inspect_input_event_bindings_path(
    path: impl AsRef<Path>,
    context: &ObjectStreamReadContext,
) -> Result<InputEventBindingsFileSummary, InputEventBindingsInspectionError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| InputEventBindingsInspectionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_input_event_bindings_file(path, &bytes, context).map_err(|source| {
        InputEventBindingsInspectionError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Reads and summarises every bindings document in `paths`, accumulating
/// totals.
///
/// Stops at the first failing path; earlier rows are discarded with it.
///
/// # Errors
///
/// Returns any error [`inspect_input_event_bindings_path`] returns for the
/// first path that fails — [`InputEventBindingsInspectionError::Read`] for an
/// unreadable file, or [`InputEventBindingsInspectionError::Parse`] for a
/// malformed document.
pub fn inspect_input_event_bindings_files(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
    context: &ObjectStreamReadContext,
) -> Result<InputEventBindingsInspection, InputEventBindingsInspectionError> {
    let mut inspection = InputEventBindingsInspection::default();
    for path in paths {
        inspection.add_file_summary(inspect_input_event_bindings_path(path, context)?);
    }
    Ok(inspection)
}

#[must_use]
pub const fn is_input_bindings_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(INPUT_BINDINGS_EXTENSION)
}

#[must_use]
pub fn is_input_bindings_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| is_input_bindings_extension(extension))
}

#[must_use]
pub fn is_input_bindings_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_input_bindings_extension)
}

#[derive(AzRtti, Debug, Clone, PartialEq)]
#[az_rtti(INPUT_EVENT_BINDINGS_TYPE_ID, register)]
pub struct InputEventBindings {
    event_groups: Vec<InputEventGroup>,
}

impl InputEventBindings {
    #[inline]
    #[must_use]
    pub const fn new(event_groups: Vec<InputEventGroup>) -> Self {
        Self { event_groups }
    }

    #[inline]
    #[must_use]
    pub fn event_groups(&self) -> &[InputEventGroup] {
        &self.event_groups
    }

    #[inline]
    #[must_use]
    pub fn generator_count(&self) -> usize {
        self.event_groups
            .iter()
            .map(|group| group.event_generators.len())
            .sum()
    }
}

#[derive(AzRtti, Debug, Clone, PartialEq)]
#[az_rtti(INPUT_EVENT_GROUP_TYPE_ID, register)]
pub struct InputEventGroup {
    pub event_name: Box<str>,
    pub event_generators: Vec<InputEventGenerator>,
    pub exclude_from_release: bool,
}

impl InputEventGroup {
    #[inline]
    #[must_use]
    pub const fn new(
        event_name: Box<str>,
        event_generators: Vec<InputEventGenerator>,
        exclude_from_release: bool,
    ) -> Self {
        Self {
            event_name,
            event_generators,
            exclude_from_release,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEventGenerator {
    Analog(AnalogInputEventGenerator),
}

impl InputEventGenerator {
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> InputEventGeneratorKind {
        match self {
            Self::Analog(_) => InputEventGeneratorKind::Analog,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEventGeneratorKind {
    Analog,
}

#[derive(AzRtti, Debug, Clone, PartialEq)]
#[az_rtti(name = "Analog", ANALOG_TYPE_ID, register)]
pub struct AnalogInputEventGenerator {
    pub action: SingleEventToAction,
    pub send_continuous_updates: bool,
    pub dead_zone: f32,
}

impl AnalogInputEventGenerator {
    #[inline]
    #[must_use]
    pub const fn new(
        action: SingleEventToAction,
        send_continuous_updates: bool,
        dead_zone: f32,
    ) -> Self {
        Self {
            action,
            send_continuous_updates,
            dead_zone,
        }
    }
}

#[derive(AzRtti, Debug, Clone, PartialEq)]
#[az_rtti(SINGLE_EVENT_TO_ACTION_TYPE_ID, register)]
pub struct SingleEventToAction {
    pub input_device_type: Box<str>,
    pub input_name: Box<str>,
    pub event_value_multiplier: f32,
}

impl SingleEventToAction {
    #[inline]
    #[must_use]
    pub const fn new(
        input_device_type: Box<str>,
        input_name: Box<str>,
        event_value_multiplier: f32,
    ) -> Self {
        Self {
            input_device_type,
            input_name,
            event_value_multiplier,
        }
    }
}

#[derive(Debug, Error)]
pub enum InputBindingsParseError {
    #[error("objectstream parse error")]
    ObjectStream(#[from] ObjectStreamError),

    #[error("objectstream value error")]
    Value(#[from] ObjectStreamValueError),

    #[error("unsupported ObjectStream version {version}")]
    UnsupportedObjectStreamVersion { version: u32 },

    #[error("missing {asset} root")]
    MissingRoot { asset: &'static str },

    #[error("multiple {asset} roots")]
    MultipleRoots { asset: &'static str },

    #[error("{owner} has unexpected type {type_id}")]
    UnexpectedType { owner: &'static str, type_id: Uuid },

    #[error("{owner} is missing {field}")]
    MissingField {
        owner: &'static str,
        field: &'static str,
    },

    #[error("{owner} has unexpected field {field:?} with type {type_id}")]
    UnexpectedField {
        owner: &'static str,
        field: Option<Box<str>>,
        type_id: Uuid,
    },
}

impl InputBindingsParseError {
    fn with_owner(self, owner: &'static str) -> Self {
        match self {
            Self::MissingField { field, .. } => Self::MissingField { owner, field },
            Self::UnexpectedField { field, type_id, .. } => Self::UnexpectedField {
                owner,
                field,
                type_id,
            },
            Self::UnexpectedType { type_id, .. } => Self::UnexpectedType { owner, type_id },
            other => other,
        }
    }
}

fn parse_input_event_bindings(
    element: &Element,
) -> Result<InputEventBindings, InputBindingsParseError> {
    expect_type(INPUT_EVENT_BINDINGS, element, INPUT_EVENT_BINDINGS_TYPE_ID)?;
    let groups = required_child(element, INPUT_EVENT_BINDINGS, INPUT_EVENT_GROUPS_FIELD)
        .and_then(parse_input_event_groups)
        .map_err(|source| source.with_owner(INPUT_EVENT_BINDINGS))?;
    Ok(InputEventBindings::new(groups))
}

fn parse_input_event_groups(
    element: &Element,
) -> Result<Vec<InputEventGroup>, InputBindingsParseError> {
    expect_raw_type(
        INPUT_EVENT_GROUPS_FIELD,
        element,
        AZSTD_VECTOR_LEGACY_XML_TYPE_ID,
    )?;
    value::require_container_shape(
        element,
        ContainerShape::Sequence,
        "AZStd::vector<InputEventGroup>",
    )?;
    element
        .children()
        .iter()
        .map(parse_input_event_group)
        .collect()
}

fn parse_input_event_group(element: &Element) -> Result<InputEventGroup, InputBindingsParseError> {
    expect_type(INPUT_EVENT_GROUP, element, INPUT_EVENT_GROUP_TYPE_ID)?;
    let mut fields = FieldCursor::from_element(element);
    let event_name = required_field(&mut fields, EVENT_NAME_FIELD)
        .and_then(read_box_str)
        .map_err(|source| source.with_owner(INPUT_EVENT_GROUP))?;
    let event_generators = required_field(&mut fields, EVENT_GENERATORS_FIELD)
        .and_then(parse_event_generators)
        .map_err(|source| source.with_owner(INPUT_EVENT_GROUP))?;
    let exclude_from_release = optional_field(&mut fields, EXCLUDE_FROM_RELEASE_FIELD)
        .map(read_bool)
        .transpose()
        .map_err(|source| source.with_owner(INPUT_EVENT_GROUP))?
        .unwrap_or(false);
    expect_no_remaining(INPUT_EVENT_GROUP, fields.remaining())?;
    Ok(InputEventGroup::new(
        event_name,
        event_generators,
        exclude_from_release,
    ))
}

fn parse_event_generators(
    element: &Element,
) -> Result<Vec<InputEventGenerator>, InputBindingsParseError> {
    expect_raw_type(
        EVENT_GENERATORS_FIELD,
        element,
        AZSTD_VECTOR_LEGACY_XML_TYPE_ID,
    )?;
    value::require_container_shape(
        element,
        ContainerShape::Sequence,
        "AZStd::vector<InputEventGenerator>",
    )?;
    element
        .children()
        .iter()
        .map(parse_event_generator)
        .collect()
}

fn parse_event_generator(
    element: &Element,
) -> Result<InputEventGenerator, InputBindingsParseError> {
    match value::semantic_type_id(element)? {
        ANALOG_TYPE_ID => parse_analog(element).map(InputEventGenerator::Analog),
        type_id => Err(InputBindingsParseError::UnexpectedType {
            owner: EVENT_GENERATORS_FIELD,
            type_id,
        }),
    }
}

fn parse_analog(element: &Element) -> Result<AnalogInputEventGenerator, InputBindingsParseError> {
    expect_type(ANALOG, element, ANALOG_TYPE_ID)?;
    let mut fields = FieldCursor::from_element(element);
    let action = required_field(&mut fields, BASE_CLASS_FIELD)
        .and_then(parse_single_event_to_action)
        .map_err(|source| source.with_owner(ANALOG))?;
    let send_continuous_updates = required_field(&mut fields, SEND_CONTINUOUS_UPDATES_FIELD)
        .and_then(read_bool)
        .map_err(|source| source.with_owner(ANALOG))?;
    let dead_zone = required_field(&mut fields, DEAD_ZONE_FIELD)
        .and_then(read_f32)
        .map_err(|source| source.with_owner(ANALOG))?;
    expect_no_remaining(ANALOG, fields.remaining())?;
    Ok(AnalogInputEventGenerator::new(
        action,
        send_continuous_updates,
        dead_zone,
    ))
}

fn parse_single_event_to_action(
    element: &Element,
) -> Result<SingleEventToAction, InputBindingsParseError> {
    expect_type(
        SINGLE_EVENT_TO_ACTION,
        element,
        SINGLE_EVENT_TO_ACTION_TYPE_ID,
    )?;
    let mut fields = FieldCursor::from_element(element);
    let input_device_type = required_field(&mut fields, INPUT_DEVICE_TYPE_FIELD)
        .and_then(read_box_str)
        .map_err(|source| source.with_owner(SINGLE_EVENT_TO_ACTION))?;
    let input_name = required_field(&mut fields, INPUT_NAME_FIELD)
        .and_then(read_box_str)
        .map_err(|source| source.with_owner(SINGLE_EVENT_TO_ACTION))?;
    let event_value_multiplier = required_field(&mut fields, EVENT_VALUE_MULTIPLIER_FIELD)
        .and_then(read_f32)
        .map_err(|source| source.with_owner(SINGLE_EVENT_TO_ACTION))?;
    expect_no_remaining(SINGLE_EVENT_TO_ACTION, fields.remaining())?;
    Ok(SingleEventToAction::new(
        input_device_type,
        input_name,
        event_value_multiplier,
    ))
}

fn single_root(elements: &[Element]) -> Result<&Element, InputBindingsParseError> {
    match elements {
        [] => Err(InputBindingsParseError::MissingRoot {
            asset: INPUT_EVENT_BINDINGS_ASSET,
        }),
        [root] => Ok(root),
        _ => Err(InputBindingsParseError::MultipleRoots {
            asset: INPUT_EVENT_BINDINGS_ASSET,
        }),
    }
}

fn required_child<'a>(
    element: &'a Element,
    owner: &'static str,
    field: &'static str,
) -> Result<&'a Element, InputBindingsParseError> {
    element
        .children()
        .iter()
        .find(|child| child.field().is_some_and(|value| value.as_str() == field))
        .ok_or(InputBindingsParseError::MissingField { owner, field })
}

fn required_field<'a>(
    fields: &mut FieldCursor<'a>,
    field: &'static str,
) -> Result<&'a Element, InputBindingsParseError> {
    fields
        .find(field)
        .ok_or(InputBindingsParseError::MissingField {
            owner: "<field cursor>",
            field,
        })
}

fn optional_field<'a>(fields: &mut FieldCursor<'a>, field: &'static str) -> Option<&'a Element> {
    fields.find(field)
}

fn expect_raw_type(
    owner: &'static str,
    element: &Element,
    expected: Uuid,
) -> Result<(), InputBindingsParseError> {
    let actual = *element.raw_type_id();
    if actual != expected {
        return Err(InputBindingsParseError::UnexpectedType {
            owner,
            type_id: actual,
        });
    }
    Ok(())
}

fn expect_type(
    owner: &'static str,
    element: &Element,
    expected: Uuid,
) -> Result<(), InputBindingsParseError> {
    let actual = value::semantic_type_id(element)?;
    if actual != expected {
        return Err(InputBindingsParseError::UnexpectedType {
            owner,
            type_id: actual,
        });
    }
    Ok(())
}

fn expect_no_remaining(
    owner: &'static str,
    remaining: &[Element],
) -> Result<(), InputBindingsParseError> {
    if let Some(element) = remaining.first() {
        let type_id = value::semantic_type_id(element)?;
        return Err(InputBindingsParseError::UnexpectedField {
            owner,
            field: element
                .field()
                .map(|field| Box::<str>::from(field.as_str())),
            type_id,
        });
    }
    Ok(())
}

fn read_box_str(element: &Element) -> Result<Box<str>, InputBindingsParseError> {
    element
        .decode::<&str>()
        .map(Box::<str>::from)
        .map_err(InputBindingsParseError::Value)
}

fn read_bool(element: &Element) -> Result<bool, InputBindingsParseError> {
    element.decode().map_err(InputBindingsParseError::Value)
}

fn read_f32(element: &Element) -> Result<f32, InputBindingsParseError> {
    element.decode().map_err(InputBindingsParseError::Value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_raw_fixture(bytes: &[u8]) -> Result<InputEventBindingsAsset, InputBindingsParseError> {
        let stream = ObjectStream::from_bytes(bytes, None)?;
        InputEventBindingsAsset::from_stream(&stream)
    }

    #[test]
    fn parses_input_bindings_asset() {
        let asset = parse_raw_fixture(sample_inputbindings()).unwrap();

        assert_eq!(asset.bindings().event_groups().len(), 2);
        assert_eq!(asset.bindings().generator_count(), 2);
        let first = &asset.bindings().event_groups()[0];
        assert_eq!(first.event_name.as_ref(), "rotateyaw");
        let InputEventGenerator::Analog(generator) = &first.event_generators[0];
        assert_eq!(generator.action.input_device_type.as_ref(), "mouse");
        assert_eq!(generator.action.input_name.as_ref(), "maxis_x");
        // Bit-exact: both decimals round to a single nearest float, so parsing
        // the attribute must land on exactly the literal's bit pattern.
        assert_eq!(
            generator.action.event_value_multiplier.to_bits(),
            0.5_f32.to_bits()
        );
        assert!(!generator.send_continuous_updates);
        assert_eq!(generator.dead_zone.to_bits(), 0.000_000_1_f32.to_bits());
        assert!(!first.exclude_from_release);
    }

    #[test]
    fn rejects_wrong_root_type() {
        let bytes = br#"<ObjectStream version="1"><Class name="Wrong" type="{72039442-EB38-4D42-A1AD-CB68F7E0EEF6}"/></ObjectStream>"#;

        assert!(matches!(
            parse_raw_fixture(bytes),
            Err(InputBindingsParseError::UnexpectedType {
                owner: INPUT_EVENT_BINDINGS_ASSET,
                ..
            })
        ));
    }

    #[test]
    fn input_bindings_authoring_source_preserves_model() {
        let asset = parse_raw_fixture(sample_inputbindings()).unwrap();
        let bytes =
            InputBindingsSource::from_asset("Libs/Config/DefaultProfile.inputbindings", &asset)
                .to_ron_bytes()
                .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let source = ron::from_str::<InputBindingsSource>(text).unwrap();
        assert_eq!(
            source.source_path,
            "libs/config/defaultprofile.inputbindings"
        );
        assert_eq!(source.groups.len(), 2);
        assert_eq!(source.groups[0].event_name, "rotateyaw");
        assert_eq!(
            source.groups[0].generators[0],
            InputBindingGeneratorSource::Analog {
                device_type: "mouse".to_string(),
                input_name: "maxis_x".to_string(),
                value_multiplier: 0.5,
                send_continuous_updates: false,
                dead_zone: 0.000_000_1,
            }
        );
    }

    fn sample_inputbindings() -> &'static [u8] {
        br#"<ObjectStream version="1">
            <Class name="InputEventBindingsAsset" type="{25971C7A-26E2-4D08-A146-2EFCC1C36B0C}">
                <Class name="InputEventBindings" field="Bindings" version="1" type="{14FFD4A8-AE46-4E23-B45B-6A7C4F787A91}">
                    <Class name="AZStd::vector" field="Input Event Groups" type="{2BADE35A-6F1B-4698-B2BC-3373D010020C}">
                        <Class name="InputEventGroup" field="element" version="1" type="{25143B7E-2FEC-4CC5-92FE-270B67E79734}">
                            <Class name="AZStd::string" field="Event Name" value="rotateyaw" type="{EF8FF807-DDEE-4EB0-B678-4CA3A2C490A4}"/>
                            <Class name="AZStd::vector" field="Event Generators" type="{2BADE35A-6F1B-4698-B2BC-3373D010020C}">
                                <Class name="Analog" field="element" version="1" type="{806F21D9-11EA-47FC-8B89-FDB67AADE4FF}">
                                    <Class name="SingleEventToAction" field="BaseClass1" version="1" type="{2C93824D-D011-459C-B12B-9F4A6148730C}">
                                        <Class name="AZStd::string" field="Input Device Type" value="mouse" type="{EF8FF807-DDEE-4EB0-B678-4CA3A2C490A4}"/>
                                        <Class name="AZStd::string" field="Input Name" value="maxis_x" type="{EF8FF807-DDEE-4EB0-B678-4CA3A2C490A4}"/>
                                        <Class name="float" field="Event Value Multiplier" value="0.5000000" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
                                    </Class>
                                    <Class name="bool" field="Send Continuous Updates" value="false" type="{A0CA880C-AFE4-43CB-926C-59AC48496112}"/>
                                    <Class name="float" field="Dead Zone" value="0.0000001" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
                                </Class>
                            </Class>
                        </Class>
                        <Class name="InputEventGroup" field="element" version="1" type="{25143B7E-2FEC-4CC5-92FE-270B67E79734}">
                            <Class name="AZStd::string" field="Event Name" value="rotatepitch" type="{EF8FF807-DDEE-4EB0-B678-4CA3A2C490A4}"/>
                            <Class name="AZStd::vector" field="Event Generators" type="{2BADE35A-6F1B-4698-B2BC-3373D010020C}">
                                <Class name="Analog" field="element" version="1" type="{806F21D9-11EA-47FC-8B89-FDB67AADE4FF}">
                                    <Class name="SingleEventToAction" field="BaseClass1" version="1" type="{2C93824D-D011-459C-B12B-9F4A6148730C}">
                                        <Class name="AZStd::string" field="Input Device Type" value="mouse" type="{EF8FF807-DDEE-4EB0-B678-4CA3A2C490A4}"/>
                                        <Class name="AZStd::string" field="Input Name" value="maxis_y" type="{EF8FF807-DDEE-4EB0-B678-4CA3A2C490A4}"/>
                                        <Class name="float" field="Event Value Multiplier" value="0.5000000" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
                                    </Class>
                                    <Class name="bool" field="Send Continuous Updates" value="false" type="{A0CA880C-AFE4-43CB-926C-59AC48496112}"/>
                                    <Class name="float" field="Dead Zone" value="0.0000001" type="{EA2C3E90-AFBE-44D4-A90D-FAAF79BAF93D}"/>
                                </Class>
                            </Class>
                        </Class>
                    </Class>
                </Class>
            </Class>
        </ObjectStream>"#
    }
}
