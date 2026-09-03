//! Lumberyard `DataOverlayInfo` replacement semantics.
//!
//! Native `ObjectStream` loading deserializes the overlay descriptor, addresses
//! `DataOverlayProviderBus` by `ProviderId`, asks the provider to materialize
//! one reflected object, restores the placeholder's field identity on that
//! object, and resumes ordinary parent/ClassData validation.  The Rust API
//! models the same boundary without pretending that a captured native `EBus`
//! handler pointer is executable in this process.

use uuid::Uuid;

use crate::context::ContainerShape;
use crate::object::ObjectFields;
use crate::value::{ObjectStreamValueError, read_u8, require_container_shape, semantic_type_id};
use crate::{Element, ObjectStreamError, types};

/// Provider-specific token carried by Lumberyard's `DataOverlayToken::m_dataUri`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataOverlayToken {
    pub data_uri: Vec<u8>,
}

/// Fully decoded native overlay request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataOverlayRequest {
    pub provider_id: u32,
    pub token: DataOverlayToken,
}

/// Executable counterpart of one native `DataOverlayProviderBus` handler.
///
/// The returned element is an unresolved, programmatically constructed
/// reflected object.  `ObjectStreamReadContext` resolves and validates its
/// complete subtree before exposing it to a typed reader.
pub trait ObjectStreamDataOverlayProvider: std::fmt::Debug + Send + Sync {
    /// Materialize the reflected object named by `request`.
    ///
    /// # Errors
    ///
    /// Implementation-defined. A provider that cannot resolve `request.token`
    /// should report it as [`ObjectStreamError::InvalidDataOverlay`]; other
    /// [`ObjectStreamError`] variants are reserved for whatever backing store
    /// the provider reads through (for example
    /// [`ObjectStreamError::Io`]).
    fn fill_overlay_data(&self, request: &DataOverlayRequest)
    -> Result<Element, ObjectStreamError>;
}

pub(crate) fn decode_request(element: &Element) -> Result<DataOverlayRequest, ObjectStreamError> {
    require_type(element, types::DATA_OVERLAY_INFO, "AZ::DataOverlayInfo")?;

    let mut fields = ObjectFields::from_element(element);
    let provider_id = fields
        .required("ProviderId")
        .map_err(|source| invalid_overlay(&source))?;
    let data_token = fields
        .required_element("DataToken")
        .map_err(|source| invalid_overlay(&source))?;
    fields.finish().map_err(|source| invalid_overlay(&source))?;

    let mut token_fields = ObjectFields::from_element(data_token);
    let uri = token_fields
        .required_element("Uri")
        .map_err(|source| invalid_overlay(&source))?;
    token_fields
        .finish()
        .map_err(|source| invalid_overlay(&source))?;
    require_container_shape(
        uri,
        ContainerShape::Sequence,
        "DataOverlayToken Uri sequence IDataContainer",
    )
    .map_err(|source| invalid_overlay(&source))?;
    let data_uri = uri
        .children()
        .iter()
        .map(read_u8)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| invalid_overlay(&source))?;

    Ok(DataOverlayRequest {
        provider_id,
        token: DataOverlayToken { data_uri },
    })
}

fn require_type(
    element: &Element,
    expected: Uuid,
    expected_name: &'static str,
) -> Result<(), ObjectStreamError> {
    let actual = semantic_type_id(element).map_err(|source| invalid_overlay(&source))?;
    if actual == expected {
        Ok(())
    } else {
        Err(ObjectStreamError::InvalidDataOverlay(format!(
            "expected {expected_name} ({expected}), got {actual}"
        )))
    }
}

fn invalid_overlay(source: &ObjectStreamValueError) -> ObjectStreamError {
    ObjectStreamError::InvalidDataOverlay(source.to_string())
}
