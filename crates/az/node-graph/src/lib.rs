//! Engine-owned visual graph descriptors and document model.
//!
//! This crate owns the durable graph authoring contract: node type catalogs,
//! graph documents, stable ports, graph commands, and validation. It has no
//! editor UI, project-host, runtime-host, Bevy, GPUI, or Cap'n Proto dependency.
//! Those layers adapt this model over their own process boundaries.

extern crate self as az_node_graph;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use az_core::reflect::{ReflectedTypePath, ReflectedValueEnvelope};
use az_gem_contract::{Registries, RegistryEntry, Unconditional};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type NodeCatalogHash = [u8; blake3::OUT_LEN];
pub type GraphTypeCatalogHash = [u8; blake3::OUT_LEN];
pub const VISUAL_GRAPH_DOCUMENT_SCHEMA: &str = "azoth.visual_graph_document/v1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeTypeId(String);

impl NodeTypeId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NodeTypeId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for NodeTypeId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphTypeId(String);

impl GraphTypeId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for GraphTypeId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for GraphTypeId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodePortId(pub u32);

impl NodePortId {
    pub const INVALID: Self = Self(0);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn is_reserved(self) -> bool {
        self.0 == Self::INVALID.0
    }
}

impl fmt::Display for NodePortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphNodeId(Uuid);

impl GraphNodeId {
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for GraphNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphConnectionId(Uuid);

impl GraphConnectionId {
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for GraphConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphRouteAnchorId(Uuid);

impl GraphRouteAnchorId {
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for GraphRouteAnchorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphCommentId(Uuid);

impl GraphCommentId {
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for GraphCommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodePortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodePortCapacity {
    Single,
    Multiple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodePortSide {
    North,
    East,
    South,
    West,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodePortLayout {
    pub side: NodePortSide,
    pub order: Option<u32>,
    #[serde(default = "default_node_port_attachment")]
    pub attachment: NodePortAttachment,
}

impl NodePortLayout {
    #[must_use]
    pub const fn new(side: NodePortSide) -> Self {
        Self {
            side,
            order: None,
            attachment: NodePortAttachment::EvenlySpaced,
        }
    }

    #[must_use]
    pub const fn input() -> Self {
        Self::new(NodePortSide::West)
    }

    #[must_use]
    pub const fn output() -> Self {
        Self::new(NodePortSide::East)
    }

    #[must_use]
    pub const fn with_order(mut self, order: u32) -> Self {
        self.order = Some(order);
        self
    }

    #[must_use]
    pub const fn with_attachment(mut self, attachment: NodePortAttachment) -> Self {
        self.attachment = attachment;
        self
    }

    #[must_use]
    pub const fn with_fixed_fraction(mut self, per_mille: u16) -> Self {
        self.attachment = NodePortAttachment::FixedFraction { per_mille };
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodePortAttachment {
    EvenlySpaced,
    FixedFraction { per_mille: u16 },
}

impl NodePortDirection {
    #[must_use]
    pub const fn default_layout(self) -> NodePortLayout {
        match self {
            Self::Input => NodePortLayout::input(),
            Self::Output => NodePortLayout::output(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodePortValue {
    Execution,
    Data {
        schema_type: String,
    },
    DynamicData {
        group: String,
        accepted_schema_types: Vec<String>,
    },
}

impl NodePortValue {
    #[must_use]
    pub const fn is_execution(&self) -> bool {
        matches!(self, Self::Execution)
    }

    #[must_use]
    pub const fn is_data(&self) -> bool {
        !self.is_execution()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePortDescriptor {
    pub id: NodePortId,
    pub name: String,
    pub direction: NodePortDirection,
    pub value: NodePortValue,
    pub capacity: NodePortCapacity,
    #[serde(default = "default_node_port_layout")]
    pub layout: NodePortLayout,
    pub description: Option<String>,
    pub default_value: Option<ReflectedValueEnvelope>,
}

impl NodePortDescriptor {
    #[must_use]
    pub fn new(
        id: NodePortId,
        name: impl Into<String>,
        direction: NodePortDirection,
        value: NodePortValue,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            direction,
            value,
            capacity: NodePortCapacity::Single,
            layout: direction.default_layout(),
            description: None,
            default_value: None,
        }
    }

    #[must_use]
    pub fn execution_input(id: NodePortId, name: impl Into<String>) -> Self {
        Self::new(id, name, NodePortDirection::Input, NodePortValue::Execution)
    }

    #[must_use]
    pub fn execution_output(id: NodePortId, name: impl Into<String>) -> Self {
        Self::new(
            id,
            name,
            NodePortDirection::Output,
            NodePortValue::Execution,
        )
    }

    #[must_use]
    pub fn data_input<T: ReflectedTypePath>(id: NodePortId, name: impl Into<String>) -> Self {
        Self::new(
            id,
            name,
            NodePortDirection::Input,
            NodePortValue::Data {
                schema_type: T::reflected_type_path().to_string(),
            },
        )
    }

    #[must_use]
    pub fn data_output<T: ReflectedTypePath>(id: NodePortId, name: impl Into<String>) -> Self {
        Self::new(
            id,
            name,
            NodePortDirection::Output,
            NodePortValue::Data {
                schema_type: T::reflected_type_path().to_string(),
            },
        )
    }

    #[must_use]
    pub fn dynamic_data_input(
        id: NodePortId,
        name: impl Into<String>,
        group: impl Into<String>,
        accepted_schema_types: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::new(
            id,
            name,
            NodePortDirection::Input,
            NodePortValue::DynamicData {
                group: group.into(),
                accepted_schema_types: accepted_schema_types.into_iter().collect(),
            },
        )
    }

    #[must_use]
    pub fn dynamic_data_output(
        id: NodePortId,
        name: impl Into<String>,
        group: impl Into<String>,
        accepted_schema_types: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::new(
            id,
            name,
            NodePortDirection::Output,
            NodePortValue::DynamicData {
                group: group.into(),
                accepted_schema_types: accepted_schema_types.into_iter().collect(),
            },
        )
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub const fn with_capacity(mut self, capacity: NodePortCapacity) -> Self {
        self.capacity = capacity;
        self
    }

    #[must_use]
    pub const fn with_layout(mut self, layout: NodePortLayout) -> Self {
        self.layout = layout;
        self
    }

    #[must_use]
    pub const fn with_side(mut self, side: NodePortSide) -> Self {
        self.layout.side = side;
        self
    }

    #[must_use]
    pub const fn with_order(mut self, order: u32) -> Self {
        self.layout.order = Some(order);
        self
    }

    #[must_use]
    pub fn with_default_value(mut self, value: ReflectedValueEnvelope) -> Self {
        self.default_value = Some(value);
        self
    }
}

const fn default_node_port_layout() -> NodePortLayout {
    NodePortLayout::input()
}

const fn default_node_port_attachment() -> NodePortAttachment {
    NodePortAttachment::EvenlySpaced
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCapability {
    pub id: String,
    pub markers: Vec<String>,
}

impl NodeCapability {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_marker(mut self, marker: impl Into<String>) -> Self {
        self.markers.push(marker.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRuntimeBinding {
    RustSymbol {
        package: String,
        symbol: String,
        call_abi: RustNodeCallAbi,
    },
    AssetBuilder {
        builder_id: String,
    },
    RuntimeComponent {
        component_type: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

impl NodeRuntimeBinding {
    #[must_use]
    pub fn rust_symbol(package: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self::RustSymbol {
            package: package.into(),
            symbol: symbol.into(),
            call_abi: RustNodeCallAbi::ContextSchedule,
        }
    }

    #[must_use]
    pub fn rust_typed_dataflow(
        package: impl Into<String>,
        symbol: impl Into<String>,
        dataflow: RustTypedDataflowNodeCall,
    ) -> Self {
        Self::RustSymbol {
            package: package.into(),
            symbol: symbol.into(),
            call_abi: RustNodeCallAbi::TypedDataflow(dataflow),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustNodeCallAbi {
    ContextSchedule,
    TypedDataflow(RustTypedDataflowNodeCall),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustTypedDataflowNodeCall {
    pub parameters: Vec<RustDataflowParameter>,
    pub output: RustDataflowOutput,
    pub result: RustCallResult,
}

impl RustTypedDataflowNodeCall {
    #[must_use]
    pub const fn new(output: RustDataflowOutput) -> Self {
        Self {
            parameters: Vec::new(),
            output,
            result: RustCallResult::Result,
        }
    }

    #[must_use]
    pub fn with_parameter(mut self, parameter: RustDataflowParameter) -> Self {
        self.parameters.push(parameter);
        self
    }

    #[must_use]
    pub const fn with_result(mut self, result: RustCallResult) -> Self {
        self.result = result;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustDataflowParameter {
    pub source: RustDataflowParameterSource,
    pub rust_type: String,
    pub passing: RustValuePassing,
}

impl RustDataflowParameter {
    #[must_use]
    pub fn input(
        port: NodePortId,
        rust_type: impl Into<String>,
        passing: RustValuePassing,
    ) -> Self {
        Self {
            source: RustDataflowParameterSource::InputPort { port },
            rust_type: rust_type.into(),
            passing,
        }
    }

    #[must_use]
    pub fn runtime_context(rust_type: impl Into<String>, passing: RustValuePassing) -> Self {
        Self {
            source: RustDataflowParameterSource::RuntimeContext,
            rust_type: rust_type.into(),
            passing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustDataflowParameterSource {
    RuntimeContext,
    InputPort { port: NodePortId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustValuePassing {
    ByValue,
    BySharedRef,
    ByMutableRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustDataflowOutput {
    None,
    Single {
        port: NodePortId,
        rust_type: String,
    },
    StructFields {
        rust_type: String,
        fields: Vec<RustDataflowOutputField>,
    },
}

impl RustDataflowOutput {
    #[must_use]
    pub fn single(port: NodePortId, rust_type: impl Into<String>) -> Self {
        Self::Single {
            port,
            rust_type: rust_type.into(),
        }
    }

    #[must_use]
    pub fn struct_fields(
        rust_type: impl Into<String>,
        fields: impl IntoIterator<Item = RustDataflowOutputField>,
    ) -> Self {
        Self::StructFields {
            rust_type: rust_type.into(),
            fields: fields.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustDataflowOutputField {
    pub port: NodePortId,
    pub field: String,
    pub rust_type: String,
}

impl RustDataflowOutputField {
    #[must_use]
    pub fn new(port: NodePortId, field: impl Into<String>, rust_type: impl Into<String>) -> Self {
        Self {
            port,
            field: field.into(),
            rust_type: rust_type.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustCallResult {
    Plain,
    Result,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSourceLink {
    pub package: Option<String>,
    pub module_path: Option<String>,
    pub symbol_path: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub docs_url: Option<String>,
}

impl NodeSourceLink {
    #[must_use]
    pub fn rust_symbol(
        package: impl Into<String>,
        module_path: impl Into<String>,
        symbol_path: impl Into<String>,
        file: impl Into<String>,
        line: u32,
        column: u32,
    ) -> Self {
        let file: String = file.into();
        Self {
            package: Some(package.into()),
            module_path: Some(module_path.into()),
            symbol_path: Some(symbol_path.into()),
            file: Some(normalize_source_file_path(&file)),
            line: Some(line),
            column: Some(column),
            docs_url: None,
        }
    }

    #[must_use]
    pub fn docs_url(docs_url: impl Into<String>) -> Self {
        Self {
            package: None,
            module_path: None,
            symbol_path: None,
            file: None,
            line: None,
            column: None,
            docs_url: Some(docs_url.into()),
        }
    }
}

#[macro_export]
macro_rules! node_source_link {
    ($symbol:path) => {
        $crate::NodeSourceLink::rust_symbol(
            env!("CARGO_PKG_NAME"),
            module_path!(),
            stringify!($symbol),
            file!(),
            line!(),
            column!(),
        )
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeTypeDescriptor {
    pub id: NodeTypeId,
    pub version: u32,
    pub display_name: String,
    pub category_path: Vec<String>,
    pub description: Option<String>,
    pub ports: Vec<NodePortDescriptor>,
    pub capabilities: Vec<NodeCapability>,
    pub runtime_binding: Option<NodeRuntimeBinding>,
    pub source_links: Vec<NodeSourceLink>,
    pub tags: Vec<String>,
}

impl NodeTypeDescriptor {
    #[must_use]
    pub fn new(id: impl Into<NodeTypeId>, version: u32, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version,
            display_name: display_name.into(),
            category_path: Vec::new(),
            description: None,
            ports: Vec::new(),
            capabilities: Vec::new(),
            runtime_binding: None,
            source_links: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_category_path(mut self, category_path: impl IntoIterator<Item = String>) -> Self {
        self.category_path = category_path.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_port(mut self, port: NodePortDescriptor) -> Self {
        self.ports.push(port);
        self
    }

    #[must_use]
    pub fn with_capability(mut self, capability: NodeCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    #[must_use]
    pub fn with_runtime_binding(mut self, runtime_binding: NodeRuntimeBinding) -> Self {
        self.runtime_binding = Some(runtime_binding);
        self
    }

    #[must_use]
    pub fn with_source_link(mut self, source_link: NodeSourceLink) -> Self {
        self.source_links.push(source_link);
        self
    }

    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeTypeCatalog {
    pub catalog_version: u32,
    pub generated_unix_ms: u64,
    pub node_types: Vec<NodeTypeDescriptor>,
}

impl NodeTypeCatalog {
    /// # Panics
    ///
    /// Panics if the node types do not form a valid catalog. Use [`Self::try_new`] to handle that instead of unwinding.
    #[must_use]
    pub fn new(
        catalog_version: u32,
        generated_unix_ms: u64,
        node_types: Vec<NodeTypeDescriptor>,
    ) -> Self {
        Self::try_new(catalog_version, generated_unix_ms, node_types)
            .expect("node type catalog must validate")
    }

    /// # Errors
    ///
    /// Returns [`NodeTypeCatalogError`] if any node type has a malformed id, uses reserved version 0, duplicates another entry, or has an empty display name or category segment.
    pub fn try_new(
        catalog_version: u32,
        generated_unix_ms: u64,
        node_types: Vec<NodeTypeDescriptor>,
    ) -> Result<Self, NodeTypeCatalogError> {
        let mut catalog = Self {
            catalog_version,
            generated_unix_ms,
            node_types,
        };
        catalog.node_types.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });
        catalog.validate()?;
        Ok(catalog)
    }

    /// The catalog a host serves from its composed registries.
    ///
    /// # Errors
    ///
    /// Returns [`NodeTypeCatalogError`] if the node types collected from `registries` do not form a valid catalog, for the same reasons as [`Self::try_new`].
    pub fn compose(
        catalog_version: u32,
        generated_unix_ms: u64,
        registries: &Registries,
    ) -> Result<Self, NodeTypeCatalogError> {
        Self::try_new(catalog_version, generated_unix_ms, node_types(registries))
    }

    #[must_use]
    pub fn node_type(&self, id: &NodeTypeId) -> Option<&NodeTypeDescriptor> {
        self.node_types.iter().find(|node_type| &node_type.id == id)
    }

    #[must_use]
    pub fn node_type_version(&self, id: &NodeTypeId, version: u32) -> Option<&NodeTypeDescriptor> {
        self.node_types
            .iter()
            .find(|node_type| &node_type.id == id && node_type.version == version)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.node_types.is_empty()
    }

    /// # Errors
    ///
    /// Returns [`NodeTypeCatalogError`] describing the first node type in this catalog that fails validation.
    pub fn validate(&self) -> Result<(), NodeTypeCatalogError> {
        validate_node_type_catalog(self)
    }

    /// # Errors
    ///
    /// Returns [`NodeTypeCatalogHashError::Encode`] if the catalog cannot be serialized to JSON for hashing.
    pub fn content_hash(&self) -> Result<NodeCatalogHash, NodeTypeCatalogHashError> {
        let bytes = serde_json::to_vec(self)?;
        Ok(*blake3::hash(&bytes).as_bytes())
    }
}

#[derive(Debug, Error)]
pub enum NodeTypeCatalogHashError {
    #[error("failed to encode node type catalog for hashing: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NodeTypeCatalogError {
    #[error("node type `{id}` has invalid identity: {reason}")]
    InvalidNodeTypeId { id: String, reason: String },
    #[error("node type `{id}` uses reserved version 0")]
    ReservedNodeTypeVersion { id: String },
    #[error("duplicate node type `{id}` version {version}")]
    DuplicateNodeType { id: String, version: u32 },
    #[error("node type `{id}` has empty display name")]
    EmptyDisplayName { id: String },
    #[error("node type `{id}` category segment is empty")]
    EmptyCategorySegment { id: String },
    #[error("node type `{id}` port `{port}` uses reserved port id 0")]
    ReservedPortId { id: String, port: String },
    #[error("node type `{id}` has duplicate port id {port_id}")]
    DuplicatePortId { id: String, port_id: NodePortId },
    #[error("node type `{id}` has duplicate port layout order {order} on {side:?}")]
    DuplicatePortLayoutOrder {
        id: String,
        side: NodePortSide,
        order: u32,
    },
    #[error(
        "node type `{id}` port `{port}` has invalid fixed attachment {per_mille}; expected 0..=1000"
    )]
    InvalidPortAttachment {
        id: String,
        port: String,
        per_mille: u16,
    },
    #[error("node type `{id}` port `{port}` has invalid identity: {reason}")]
    InvalidPortName {
        id: String,
        port: String,
        reason: String,
    },
    #[error("node type `{id}` port `{port}` has empty data schema")]
    EmptyPortSchema { id: String, port: String },
    #[error("node type `{id}` port `{port}` has empty dynamic data group")]
    EmptyDynamicGroup { id: String, port: String },
    #[error("node type `{id}` execution port `{port}` cannot declare a default value")]
    ExecutionPortDefaultValue { id: String, port: String },
    #[error("node type `{id}` output port `{port}` cannot declare a default value")]
    OutputPortDefaultValue { id: String, port: String },
    #[error("node type `{id}` capability has invalid identity: {reason}")]
    InvalidCapability { id: String, reason: String },
    #[error("node type `{id}` runtime binding is invalid: {reason}")]
    InvalidRuntimeBinding { id: String, reason: String },
    #[error("node type `{id}` source link is invalid: {reason}")]
    InvalidSourceLink { id: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTypeCatalog {
    pub catalog_version: u32,
    pub generated_unix_ms: u64,
    pub graph_types: Vec<GraphTypeDescriptor>,
}

impl GraphTypeCatalog {
    /// # Panics
    ///
    /// Panics if the graph types do not form a valid catalog. Use [`Self::try_new`] to handle that instead of unwinding.
    #[must_use]
    pub fn new(
        catalog_version: u32,
        generated_unix_ms: u64,
        graph_types: Vec<GraphTypeDescriptor>,
    ) -> Self {
        Self::try_new(catalog_version, generated_unix_ms, graph_types)
            .expect("graph type catalog must validate")
    }

    /// # Errors
    ///
    /// Returns [`GraphTypeCatalogError`] if any graph type has a malformed id, uses reserved version 0, duplicates another entry, has an empty display name or category segment, or declares an invalid source workflow.
    pub fn try_new(
        catalog_version: u32,
        generated_unix_ms: u64,
        graph_types: Vec<GraphTypeDescriptor>,
    ) -> Result<Self, GraphTypeCatalogError> {
        let mut catalog = Self {
            catalog_version,
            generated_unix_ms,
            graph_types,
        };
        catalog.graph_types.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });
        catalog.validate()?;
        Ok(catalog)
    }

    /// The catalog a host serves from its composed registries.
    ///
    /// # Errors
    ///
    /// Returns [`GraphTypeCatalogError`] if the graph types collected from `registries` do not form a valid catalog, for the same reasons as [`Self::try_new`].
    pub fn compose(
        catalog_version: u32,
        generated_unix_ms: u64,
        registries: &Registries,
    ) -> Result<Self, GraphTypeCatalogError> {
        Self::try_new(catalog_version, generated_unix_ms, graph_types(registries))
    }

    #[must_use]
    pub fn graph_type(&self, id: &GraphTypeId) -> Option<&GraphTypeDescriptor> {
        self.graph_types
            .iter()
            .find(|graph_type| &graph_type.id == id)
    }

    #[must_use]
    pub fn graph_type_version(
        &self,
        id: &GraphTypeId,
        version: u32,
    ) -> Option<&GraphTypeDescriptor> {
        self.graph_types
            .iter()
            .find(|graph_type| &graph_type.id == id && graph_type.version == version)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.graph_types.is_empty()
    }

    /// # Errors
    ///
    /// Returns [`GraphTypeCatalogError`] describing the first graph type in this catalog that fails validation.
    pub fn validate(&self) -> Result<(), GraphTypeCatalogError> {
        validate_graph_type_catalog(self)
    }

    /// # Errors
    ///
    /// Returns [`GraphTypeCatalogHashError::Encode`] if the catalog cannot be serialized to JSON for hashing.
    pub fn content_hash(&self) -> Result<GraphTypeCatalogHash, GraphTypeCatalogHashError> {
        let bytes = serde_json::to_vec(self)?;
        Ok(*blake3::hash(&bytes).as_bytes())
    }
}

#[derive(Debug, Error)]
pub enum GraphTypeCatalogHashError {
    #[error("failed to encode graph type catalog for hashing: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTypeDescriptor {
    pub id: GraphTypeId,
    pub version: u32,
    pub display_name: String,
    pub description: Option<String>,
    pub category_path: Vec<String>,
    pub source_workflow: GraphSourceWorkflow,
    pub template: GraphDocumentTemplate,
    pub allowed_node_catalogs: Vec<GraphNodeCatalogRequirement>,
    pub compiler_backend: Option<GraphCompilerBackendDescriptor>,
    pub runtime_product: Option<RuntimeGraphProductDescriptor>,
    pub execution_mode: GraphExecutionMode,
    pub palette_policy: GraphPalettePolicy,
    pub tags: Vec<String>,
}

impl GraphTypeDescriptor {
    #[must_use]
    pub fn runtime_compiled(
        id: impl Into<GraphTypeId>,
        version: u32,
        display_name: impl Into<String>,
        source_workflow: GraphSourceWorkflow,
        compiler_backend: GraphCompilerBackendDescriptor,
        runtime_product: RuntimeGraphProductDescriptor,
    ) -> Self {
        let id = id.into();
        Self {
            template: GraphDocumentTemplate::empty(id.as_str()),
            id,
            version,
            display_name: display_name.into(),
            description: None,
            category_path: Vec::new(),
            source_workflow,
            allowed_node_catalogs: Vec::new(),
            compiler_backend: Some(compiler_backend),
            runtime_product: Some(runtime_product),
            execution_mode: GraphExecutionMode::RuntimeCompiled,
            palette_policy: GraphPalettePolicy::default(),
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn editor_interpreted(
        id: impl Into<GraphTypeId>,
        version: u32,
        display_name: impl Into<String>,
        source_workflow: GraphSourceWorkflow,
    ) -> Self {
        let id = id.into();
        Self {
            template: GraphDocumentTemplate::empty(id.as_str()),
            id,
            version,
            display_name: display_name.into(),
            description: None,
            category_path: Vec::new(),
            source_workflow,
            allowed_node_catalogs: Vec::new(),
            compiler_backend: None,
            runtime_product: None,
            execution_mode: GraphExecutionMode::EditorInterpreted,
            palette_policy: GraphPalettePolicy::default(),
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_category_path(mut self, category_path: impl IntoIterator<Item = String>) -> Self {
        self.category_path = category_path.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_template(mut self, template: GraphDocumentTemplate) -> Self {
        self.template = template;
        self
    }

    #[must_use]
    pub fn with_node_catalog(mut self, requirement: GraphNodeCatalogRequirement) -> Self {
        self.allowed_node_catalogs.push(requirement);
        self
    }

    #[must_use]
    pub const fn with_execution_mode(mut self, execution_mode: GraphExecutionMode) -> Self {
        self.execution_mode = execution_mode;
        self
    }

    #[must_use]
    pub fn with_compiler_backend(mut self, backend: GraphCompilerBackendDescriptor) -> Self {
        self.compiler_backend = Some(backend);
        self
    }

    #[must_use]
    pub fn with_runtime_product(mut self, runtime_product: RuntimeGraphProductDescriptor) -> Self {
        self.runtime_product = Some(runtime_product);
        self
    }

    #[must_use]
    pub fn with_palette_policy(mut self, palette_policy: GraphPalettePolicy) -> Self {
        self.palette_policy = palette_policy;
        self
    }

    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSourceWorkflow {
    pub workflow_id: String,
    pub kind: GraphSourceWorkflowKind,
    pub source_schema: Option<String>,
    pub source_root: Option<String>,
    pub default_path_prefix: Option<String>,
    pub default_extension: Option<String>,
}

impl GraphSourceWorkflow {
    #[must_use]
    pub fn project_document(
        workflow_id: impl Into<String>,
        source_schema: impl Into<String>,
    ) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            kind: GraphSourceWorkflowKind::ProjectDocument,
            source_schema: Some(source_schema.into()),
            source_root: None,
            default_path_prefix: None,
            default_extension: None,
        }
    }

    #[must_use]
    pub fn file(workflow_id: impl Into<String>, default_extension: impl Into<String>) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            kind: GraphSourceWorkflowKind::File,
            source_schema: None,
            source_root: None,
            default_path_prefix: None,
            default_extension: Some(default_extension.into()),
        }
    }

    #[must_use]
    pub fn with_source_root(mut self, source_root: impl Into<String>) -> Self {
        self.source_root = Some(source_root.into());
        self
    }

    #[must_use]
    pub fn with_default_path_prefix(mut self, path_prefix: impl Into<String>) -> Self {
        self.default_path_prefix = Some(path_prefix.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphSourceWorkflowKind {
    ProjectDocument,
    File,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphDocumentTemplate {
    pub document: VisualGraphDocument,
}

impl GraphDocumentTemplate {
    #[must_use]
    pub fn empty(graph_type: impl Into<String>) -> Self {
        Self {
            document: VisualGraphDocument::new(graph_type),
        }
    }

    #[must_use]
    pub const fn document(&self) -> &VisualGraphDocument {
        &self.document
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNodeCatalogRequirement {
    pub catalog_id: String,
    pub minimum_version: Option<u32>,
    pub required_hash: Option<Vec<u8>>,
}

impl GraphNodeCatalogRequirement {
    #[must_use]
    pub fn new(catalog_id: impl Into<String>) -> Self {
        Self {
            catalog_id: catalog_id.into(),
            minimum_version: None,
            required_hash: None,
        }
    }

    #[must_use]
    pub const fn with_minimum_version(mut self, version: u32) -> Self {
        self.minimum_version = Some(version);
        self
    }

    #[must_use]
    pub fn with_required_hash(mut self, hash: impl Into<Vec<u8>>) -> Self {
        self.required_hash = Some(hash.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCompilerBackendDescriptor {
    pub id: String,
    pub kind: GraphCompilerBackendKind,
    pub capability_markers: Vec<String>,
}

impl GraphCompilerBackendDescriptor {
    #[must_use]
    pub fn generated_rust_context_schedule(
        id: impl Into<String>,
        package: impl Into<String>,
        entry_symbol: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: GraphCompilerBackendKind::GeneratedRust {
                package: package.into(),
                entry_symbol: entry_symbol.into(),
                abi: GeneratedRustGraphAbi::ContextSchedule,
            },
            capability_markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn generated_rust_typed_dataflow(
        id: impl Into<String>,
        package: impl Into<String>,
        entry_symbol: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: GraphCompilerBackendKind::GeneratedRust {
                package: package.into(),
                entry_symbol: entry_symbol.into(),
                abi: GeneratedRustGraphAbi::TypedDataflow,
            },
            capability_markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn packed_ir(id: impl Into<String>, ir_schema: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: GraphCompilerBackendKind::PackedIr {
                ir_schema: ir_schema.into(),
            },
            capability_markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn shader_pipeline(id: impl Into<String>, pipeline_kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: GraphCompilerBackendKind::ShaderPipeline {
                pipeline_kind: pipeline_kind.into(),
            },
            capability_markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn external(
        id: impl Into<String>,
        kind: impl Into<String>,
        locator: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: GraphCompilerBackendKind::External {
                kind: kind.into(),
                locator: locator.into(),
            },
            capability_markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_capability_marker(mut self, marker: impl Into<String>) -> Self {
        self.capability_markers.push(marker.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphCompilerBackendKind {
    GeneratedRust {
        package: String,
        entry_symbol: String,
        abi: GeneratedRustGraphAbi,
    },
    PackedIr {
        ir_schema: String,
    },
    ShaderPipeline {
        pipeline_kind: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeneratedRustGraphAbi {
    ContextSchedule,
    TypedDataflow,
}

impl GeneratedRustGraphAbi {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContextSchedule => "context-schedule",
            Self::TypedDataflow => "typed-dataflow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeGraphProductDescriptor {
    pub asset_type: String,
    pub product_kind: String,
    pub streamable: bool,
    pub diffable_chunks: bool,
    pub execution_strategy: RuntimeGraphExecutionStrategy,
}

impl RuntimeGraphProductDescriptor {
    #[must_use]
    pub fn new(
        asset_type: impl Into<String>,
        product_kind: impl Into<String>,
        execution_strategy: RuntimeGraphExecutionStrategy,
    ) -> Self {
        Self {
            asset_type: asset_type.into(),
            product_kind: product_kind.into(),
            streamable: true,
            diffable_chunks: true,
            execution_strategy,
        }
    }

    #[must_use]
    pub const fn with_streamable(mut self, streamable: bool) -> Self {
        self.streamable = streamable;
        self
    }

    #[must_use]
    pub const fn with_diffable_chunks(mut self, diffable_chunks: bool) -> Self {
        self.diffable_chunks = diffable_chunks;
        self
    }

    #[must_use]
    pub fn with_execution_strategy(
        mut self,
        execution_strategy: RuntimeGraphExecutionStrategy,
    ) -> Self {
        self.execution_strategy = execution_strategy;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeGraphExecutionStrategy {
    PackedIr,
    AotCompiledCode {
        language: String,
        package: String,
        entry_symbol: String,
        context_type: String,
    },
    HotReloadedCompiledModule {
        abi: String,
        entry_symbol: String,
    },
    ShaderPipeline {
        pipeline_kind: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

impl RuntimeGraphExecutionStrategy {
    #[must_use]
    pub fn aot_compiled_rust(
        package: impl Into<String>,
        entry_symbol: impl Into<String>,
        context_type: impl Into<String>,
    ) -> Self {
        Self::AotCompiledCode {
            language: "rust".to_string(),
            package: package.into(),
            entry_symbol: entry_symbol.into(),
            context_type: context_type.into(),
        }
    }

    #[must_use]
    pub fn hot_reloaded_compiled_module(
        abi: impl Into<String>,
        entry_symbol: impl Into<String>,
    ) -> Self {
        Self::HotReloadedCompiledModule {
            abi: abi.into(),
            entry_symbol: entry_symbol.into(),
        }
    }

    #[must_use]
    pub fn shader_pipeline(pipeline_kind: impl Into<String>) -> Self {
        Self::ShaderPipeline {
            pipeline_kind: pipeline_kind.into(),
        }
    }

    #[must_use]
    pub fn external(kind: impl Into<String>, locator: impl Into<String>) -> Self {
        Self::External {
            kind: kind.into(),
            locator: locator.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphExecutionMode {
    RuntimeCompiled,
    EditorInterpreted,
    RuntimeCompiledAndEditorInterpreted,
}

impl GraphExecutionMode {
    #[must_use]
    pub const fn requires_runtime_product(self) -> bool {
        matches!(
            self,
            Self::RuntimeCompiled | Self::RuntimeCompiledAndEditorInterpreted
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPalettePolicy {
    pub root_categories: Vec<String>,
    pub required_node_capabilities: Vec<String>,
    pub hidden_node_tags: Vec<String>,
}

impl GraphPalettePolicy {
    #[must_use]
    pub fn with_root_category(mut self, category: impl Into<String>) -> Self {
        self.root_categories.push(category.into());
        self
    }

    #[must_use]
    pub fn with_required_node_capability(mut self, capability: impl Into<String>) -> Self {
        self.required_node_capabilities.push(capability.into());
        self
    }

    #[must_use]
    pub fn with_hidden_node_tag(mut self, tag: impl Into<String>) -> Self {
        self.hidden_node_tags.push(tag.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphTypeCatalogError {
    #[error("graph type `{id}` has invalid identity: {reason}")]
    InvalidGraphTypeId { id: String, reason: String },
    #[error("graph type `{id}` uses reserved version 0")]
    ReservedGraphTypeVersion { id: String },
    #[error("duplicate graph type `{id}` version {version}")]
    DuplicateGraphType { id: String, version: u32 },
    #[error("graph type `{id}` has empty display name")]
    EmptyDisplayName { id: String },
    #[error("graph type `{id}` category segment is empty")]
    EmptyCategorySegment { id: String },
    #[error("graph type `{id}` source workflow is invalid: {reason}")]
    InvalidSourceWorkflow { id: String, reason: String },
    #[error("graph type `{id}` template is invalid: {reason}")]
    InvalidTemplate { id: String, reason: String },
    #[error("graph type `{id}` node catalog requirement is invalid: {reason}")]
    InvalidNodeCatalogRequirement { id: String, reason: String },
    #[error("graph type `{id}` compiler backend is invalid: {reason}")]
    InvalidCompilerBackend { id: String, reason: String },
    #[error("graph type `{id}` runtime product is invalid: {reason}")]
    InvalidRuntimeProduct { id: String, reason: String },
    #[error("graph type `{id}` requires a compiler backend for runtime execution")]
    MissingCompilerBackend { id: String },
    #[error("graph type `{id}` requires a runtime product descriptor for runtime execution")]
    MissingRuntimeProduct { id: String },
    #[error("graph type `{id}` palette policy is invalid: {reason}")]
    InvalidPalettePolicy { id: String, reason: String },
    #[error("graph type `{id}` tag is invalid: {reason}")]
    InvalidTag { id: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualGraphDocument {
    pub document_version: u32,
    pub graph_type: String,
    pub required_catalog_hash: Option<Vec<u8>>,
    pub nodes: Vec<GraphNode>,
    pub connections: Vec<GraphConnection>,
    pub comments: Vec<GraphComment>,
}

impl VisualGraphDocument {
    #[must_use]
    pub fn new(graph_type: impl Into<String>) -> Self {
        Self {
            document_version: 1,
            graph_type: graph_type.into(),
            required_catalog_hash: None,
            nodes: Vec::new(),
            connections: Vec::new(),
            comments: Vec::new(),
        }
    }

    /// # Errors
    ///
    /// Returns [`VisualGraphValidationError`] if the document references a node type absent from `catalog`, or if any node, port, or connection in it is inconsistent with the type that catalog declares.
    pub fn validate_against(
        &self,
        catalog: &NodeTypeCatalog,
    ) -> Result<(), VisualGraphValidationError> {
        validate_visual_graph_document(self, catalog)
    }

    /// # Errors
    ///
    /// Returns [`VisualGraphValidationError`] if `command` does not apply to the current document under `catalog`. The document is left unchanged when this happens.
    pub fn apply_command(
        &mut self,
        command: GraphCommand,
        catalog: &NodeTypeCatalog,
    ) -> Result<(), GraphCommandApplyError> {
        let mut next = self.clone();
        next.apply_command_unchecked(command)?;
        next.validate_against(catalog)?;
        *self = next;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`VisualGraphValidationError`] from the first command that does not apply. Commands before it have already been applied, so the document is left partially updated.
    pub fn apply_commands(
        &mut self,
        commands: impl IntoIterator<Item = GraphCommand>,
        catalog: &NodeTypeCatalog,
    ) -> Result<(), GraphCommandApplyError> {
        let mut next = self.clone();
        for command in commands {
            next.apply_command_unchecked(command)?;
        }
        next.validate_against(catalog)?;
        *self = next;
        Ok(())
    }

    fn apply_command_unchecked(
        &mut self,
        command: GraphCommand,
    ) -> Result<(), GraphCommandApplyError> {
        match command {
            GraphCommand::AddNode { node } => self.nodes.push(node),
            GraphCommand::RemoveNode { node_id } => {
                remove_node(&mut self.nodes, node_id)?;
                self.connections.retain(|connection| {
                    connection.from.node_id != node_id && connection.to.node_id != node_id
                });
            }
            GraphCommand::SetInputValue {
                node_id,
                port_id,
                value,
            } => {
                let node = self
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == node_id)
                    .ok_or(GraphCommandApplyError::UnknownNode { node_id })?;
                if let Some(value) = value {
                    node.input_values.insert(port_id, value);
                } else {
                    node.input_values.remove(&port_id);
                }
            }
            GraphCommand::MoveNode { node_id, layout } => {
                let node = self
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == node_id)
                    .ok_or(GraphCommandApplyError::UnknownNode { node_id })?;
                node.layout = layout;
            }
            GraphCommand::Connect { connection } => self.connections.push(connection),
            GraphCommand::SetConnectionRoute {
                connection_id,
                route,
            } => {
                let connection = self
                    .connections
                    .iter_mut()
                    .find(|connection| connection.id == connection_id)
                    .ok_or(GraphCommandApplyError::UnknownConnection { connection_id })?;
                connection.route = route;
            }
            GraphCommand::Disconnect { connection_id } => {
                remove_connection(&mut self.connections, connection_id)?;
            }
            GraphCommand::UpsertComment { comment } => {
                if let Some(existing) = self
                    .comments
                    .iter_mut()
                    .find(|existing| existing.id == comment.id)
                {
                    *existing = comment;
                } else {
                    self.comments.push(comment);
                }
            }
            GraphCommand::RemoveComment { comment_id } => {
                remove_comment(&mut self.comments, comment_id)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VisualGraphDocumentFile {
    schema: String,
    document: VisualGraphDocument,
}

#[derive(Debug, Error)]
pub enum VisualGraphDocumentIoError {
    #[error("visual graph document source uses schema `{actual}`, expected `{expected}`")]
    UnsupportedSchema { actual: String, expected: String },

    #[error("failed to parse visual graph document source: {source}")]
    Deserialize {
        #[from]
        source: ron::error::SpannedError,
    },

    #[error("failed to serialize visual graph document source: {source}")]
    Serialize {
        #[from]
        source: ron::Error,
    },
}

/// # Errors
///
/// Returns [`VisualGraphDocumentIoError::Serialize`] if the document cannot be encoded as RON.
pub fn encode_visual_graph_document_ron(
    document: &VisualGraphDocument,
) -> Result<String, VisualGraphDocumentIoError> {
    let file = VisualGraphDocumentFile {
        schema: VISUAL_GRAPH_DOCUMENT_SCHEMA.to_string(),
        document: document.clone(),
    };
    let mut text = ron::ser::to_string_pretty(&file, PrettyConfig::default())?;
    text.push('\n');
    Ok(text)
}

/// # Errors
///
/// Returns [`VisualGraphDocumentIoError::UnsupportedSchema`] if the source declares a schema this build does not accept, or [`VisualGraphDocumentIoError::Deserialize`] if the text is not well-formed RON for a visual graph document.
pub fn decode_visual_graph_document_ron(
    text: &str,
) -> Result<VisualGraphDocument, VisualGraphDocumentIoError> {
    let file = ron::from_str::<VisualGraphDocumentFile>(text)?;
    if file.schema != VISUAL_GRAPH_DOCUMENT_SCHEMA {
        return Err(VisualGraphDocumentIoError::UnsupportedSchema {
            actual: file.schema,
            expected: VISUAL_GRAPH_DOCUMENT_SCHEMA.to_string(),
        });
    }
    Ok(file.document)
}

fn remove_node(
    nodes: &mut Vec<GraphNode>,
    node_id: GraphNodeId,
) -> Result<(), GraphCommandApplyError> {
    remove_at(nodes, |node| node.id == node_id)
        .map(drop)
        .ok_or(GraphCommandApplyError::UnknownNode { node_id })
}

fn remove_connection(
    connections: &mut Vec<GraphConnection>,
    connection_id: GraphConnectionId,
) -> Result<(), GraphCommandApplyError> {
    remove_at(connections, |connection| connection.id == connection_id)
        .map(drop)
        .ok_or(GraphCommandApplyError::UnknownConnection { connection_id })
}

fn remove_comment(
    comments: &mut Vec<GraphComment>,
    comment_id: GraphCommentId,
) -> Result<(), GraphCommandApplyError> {
    remove_at(comments, |comment| comment.id == comment_id)
        .map(drop)
        .ok_or(GraphCommandApplyError::UnknownComment { comment_id })
}

fn remove_at<T>(values: &mut Vec<T>, mut predicate: impl FnMut(&T) -> bool) -> Option<T> {
    let index = values.iter().position(&mut predicate)?;
    Some(values.remove(index))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: GraphNodeId,
    pub node_type: NodeTypeId,
    pub node_type_version: u32,
    pub input_values: BTreeMap<NodePortId, ReflectedValueEnvelope>,
    pub layout: GraphNodeLayout,
}

impl GraphNode {
    #[must_use]
    pub fn new(id: GraphNodeId, node_type: impl Into<NodeTypeId>, node_type_version: u32) -> Self {
        Self {
            id,
            node_type: node_type.into(),
            node_type_version,
            input_values: BTreeMap::new(),
            layout: GraphNodeLayout::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphNodeLayout {
    pub x: f32,
    pub y: f32,
}

impl Default for GraphNodeLayout {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GraphPortRef {
    pub node_id: GraphNodeId,
    pub port_id: NodePortId,
}

impl GraphPortRef {
    #[must_use]
    pub const fn new(node_id: GraphNodeId, port_id: NodePortId) -> Self {
        Self { node_id, port_id }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphConnection {
    pub id: GraphConnectionId,
    pub from: GraphPortRef,
    pub to: GraphPortRef,
    #[serde(default)]
    pub route: GraphConnectionRoute,
}

impl GraphConnection {
    #[must_use]
    pub fn new(id: GraphConnectionId, from: GraphPortRef, to: GraphPortRef) -> Self {
        Self {
            id,
            from,
            to,
            route: GraphConnectionRoute::default(),
        }
    }

    #[must_use]
    pub fn with_route(mut self, route: GraphConnectionRoute) -> Self {
        self.route = route;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphConnectionRoute {
    pub style: GraphRouteStyle,
    pub anchors: Vec<GraphRouteAnchor>,
}

impl Default for GraphConnectionRoute {
    fn default() -> Self {
        Self {
            style: GraphRouteStyle::Orthogonal,
            anchors: Vec::new(),
        }
    }
}

impl GraphConnectionRoute {
    #[must_use]
    pub fn orthogonal() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_anchor(mut self, anchor: GraphRouteAnchor) -> Self {
        self.anchors.push(anchor);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphRouteStyle {
    Orthogonal,
    Polyline,
    Spline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRouteAnchor {
    pub id: GraphRouteAnchorId,
    pub position: GraphPoint,
    pub kind: GraphRouteAnchorKind,
    pub outgoing_segment: GraphRouteSegmentConstraint,
}

impl GraphRouteAnchor {
    #[must_use]
    pub const fn user_waypoint(id: GraphRouteAnchorId, position: GraphPoint) -> Self {
        Self {
            id,
            position,
            kind: GraphRouteAnchorKind::UserWaypoint,
            outgoing_segment: GraphRouteSegmentConstraint::Flexible,
        }
    }

    #[must_use]
    pub const fn with_outgoing_segment(mut self, constraint: GraphRouteSegmentConstraint) -> Self {
        self.outgoing_segment = constraint;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphRouteAnchorKind {
    UserWaypoint,
    SolverWaypoint,
    Junction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphRouteSegmentConstraint {
    Flexible,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphPoint {
    pub x: f32,
    pub y: f32,
}

impl GraphPoint {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphComment {
    pub id: GraphCommentId,
    pub text: String,
    pub bounds: GraphCommentBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphCommentBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphCommand {
    AddNode {
        node: GraphNode,
    },
    RemoveNode {
        node_id: GraphNodeId,
    },
    SetInputValue {
        node_id: GraphNodeId,
        port_id: NodePortId,
        value: Option<ReflectedValueEnvelope>,
    },
    MoveNode {
        node_id: GraphNodeId,
        layout: GraphNodeLayout,
    },
    Connect {
        connection: GraphConnection,
    },
    SetConnectionRoute {
        connection_id: GraphConnectionId,
        route: GraphConnectionRoute,
    },
    Disconnect {
        connection_id: GraphConnectionId,
    },
    UpsertComment {
        comment: GraphComment,
    },
    RemoveComment {
        comment_id: GraphCommentId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphCommandApplyError {
    #[error("graph command references unknown node {node_id}")]
    UnknownNode { node_id: GraphNodeId },
    #[error("graph command references unknown connection {connection_id}")]
    UnknownConnection { connection_id: GraphConnectionId },
    #[error("graph command references unknown comment {comment_id}")]
    UnknownComment { comment_id: GraphCommentId },
    #[error("graph command would make the document invalid: {0}")]
    Validation(#[from] VisualGraphValidationError),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VisualGraphValidationError {
    #[error("graph document version 0 is reserved")]
    ReservedDocumentVersion,
    #[error("graph type is invalid: {reason}")]
    InvalidGraphType { reason: String },
    #[error("graph requires catalog hash with {expected} bytes, got {actual}")]
    InvalidCatalogHashLength { expected: usize, actual: usize },
    #[error("duplicate graph node id {node_id:?}")]
    DuplicateNodeId { node_id: GraphNodeId },
    #[error("graph node {node_id:?} references unknown node type `{node_type}` version {version}")]
    UnknownNodeType {
        node_id: GraphNodeId,
        node_type: String,
        version: u32,
    },
    #[error("graph node {node_id:?} layout contains non-finite coordinates")]
    NonFiniteNodeLayout { node_id: GraphNodeId },
    #[error("graph node {node_id:?} input value references unknown port id {port_id:?}")]
    UnknownInputValuePort {
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error("graph node {node_id:?} value port {port_id:?} is not an input data port")]
    NonInputDataValue {
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error("duplicate graph connection id {connection_id:?}")]
    DuplicateConnectionId { connection_id: GraphConnectionId },
    #[error("graph connection {connection_id:?} has duplicate route anchor id {anchor_id:?}")]
    DuplicateRouteAnchorId {
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
    },
    #[error(
        "graph connection {connection_id:?} route anchor {anchor_id:?} contains non-finite coordinates"
    )]
    NonFiniteRouteAnchor {
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
    },
    #[error("graph connection {connection_id:?} references unknown node {node_id:?}")]
    UnknownConnectionNode {
        connection_id: GraphConnectionId,
        node_id: GraphNodeId,
    },
    #[error(
        "graph connection {connection_id:?} references unknown port {port_id:?} on node {node_id:?}"
    )]
    UnknownConnectionPort {
        connection_id: GraphConnectionId,
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error("graph connection {connection_id:?} must flow from an output port to an input port")]
    InvalidConnectionDirection { connection_id: GraphConnectionId },
    #[error("graph connection {connection_id:?} connects incompatible ports: {reason}")]
    IncompatibleConnection {
        connection_id: GraphConnectionId,
        reason: String,
    },
    #[error("port {port_id:?} on node {node_id:?} accepts only one connection")]
    PortCapacityExceeded {
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error("duplicate graph comment id {comment_id:?}")]
    DuplicateCommentId { comment_id: GraphCommentId },
    #[error("graph comment {comment_id:?} bounds contain non-finite values")]
    NonFiniteCommentBounds { comment_id: GraphCommentId },
}

/// Registry identity of a node type: its id at a version.
///
/// Two versions of one node type coexist in a catalog — graph documents pin
/// `node_type` and `node_type_version` together — so the version is part of
/// the key. Two registrations of the same id at the same version are a
/// composition error naming both contributions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeTypeKey {
    pub id: NodeTypeId,
    pub version: u32,
}

impl fmt::Display for NodeTypeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.id.as_str(), self.version)
    }
}

/// Registry identity of a graph type: its id at a version. Same discipline as
/// [`NodeTypeKey`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphTypeKey {
    pub id: GraphTypeId,
    pub version: u32,
}

impl fmt::Display for GraphTypeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.id.as_str(), self.version)
    }
}

/// One node type contributed to a host's node catalog. The universal editor
/// consumes published catalogs over IPC; hosts compose this registry.
pub struct NodeTypeRegistration {
    descriptor: NodeTypeDescriptor,
}

/// One graph type/template contributed to a host's graph type catalog.
pub struct GraphTypeRegistration {
    descriptor: GraphTypeDescriptor,
}

pub trait VisualNode {
    fn node_type_descriptor() -> NodeTypeDescriptor;
}

pub trait VisualGraphType {
    fn graph_type_descriptor() -> GraphTypeDescriptor;
}

impl NodeTypeRegistration {
    #[must_use]
    pub const fn new(descriptor: NodeTypeDescriptor) -> Self {
        Self { descriptor }
    }

    #[must_use]
    pub fn of<T: VisualNode>() -> Self {
        Self {
            descriptor: T::node_type_descriptor(),
        }
    }

    #[must_use]
    pub const fn descriptor(&self) -> &NodeTypeDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn into_descriptor(self) -> NodeTypeDescriptor {
        self.descriptor
    }
}

impl RegistryEntry for NodeTypeRegistration {
    type Key = NodeTypeKey;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "node-type"
    }

    fn key(&self) -> NodeTypeKey {
        NodeTypeKey {
            id: self.descriptor.id.clone(),
            version: self.descriptor.version,
        }
    }
}

impl GraphTypeRegistration {
    #[must_use]
    pub const fn new(descriptor: GraphTypeDescriptor) -> Self {
        Self { descriptor }
    }

    #[must_use]
    pub fn of<T: VisualGraphType>() -> Self {
        Self {
            descriptor: T::graph_type_descriptor(),
        }
    }

    #[must_use]
    pub const fn descriptor(&self) -> &GraphTypeDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn into_descriptor(self) -> GraphTypeDescriptor {
        self.descriptor
    }
}

impl RegistryEntry for GraphTypeRegistration {
    type Key = GraphTypeKey;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "graph-type"
    }

    fn key(&self) -> GraphTypeKey {
        GraphTypeKey {
            id: self.descriptor.id.clone(),
            version: self.descriptor.version,
        }
    }
}

/// Descriptors composed into `registries`, in composition order.
#[must_use]
pub fn node_types(registries: &Registries) -> Vec<NodeTypeDescriptor> {
    registries
        .get::<NodeTypeRegistration>()
        .map(|registry| {
            registry
                .entries()
                .map(|registration| registration.descriptor.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Descriptors composed into `registries`, in composition order.
#[must_use]
pub fn graph_types(registries: &Registries) -> Vec<GraphTypeDescriptor> {
    registries
        .get::<GraphTypeRegistration>()
        .map(|registry| {
            registry
                .entries()
                .map(|registration| registration.descriptor.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn validate_node_type_catalog(catalog: &NodeTypeCatalog) -> Result<(), NodeTypeCatalogError> {
    let mut seen_node_types = BTreeSet::new();
    for node_type in &catalog.node_types {
        validate_node_type_descriptor(node_type)?;
        let key = (node_type.id.clone(), node_type.version);
        if !seen_node_types.insert(key) {
            return Err(NodeTypeCatalogError::DuplicateNodeType {
                id: node_type.id.as_str().to_string(),
                version: node_type.version,
            });
        }
    }
    Ok(())
}

fn validate_graph_type_catalog(catalog: &GraphTypeCatalog) -> Result<(), GraphTypeCatalogError> {
    let mut seen_graph_types = BTreeSet::new();
    for graph_type in &catalog.graph_types {
        validate_graph_type_descriptor(graph_type)?;
        let key = (graph_type.id.clone(), graph_type.version);
        if !seen_graph_types.insert(key) {
            return Err(GraphTypeCatalogError::DuplicateGraphType {
                id: graph_type.id.as_str().to_string(),
                version: graph_type.version,
            });
        }
    }
    Ok(())
}

fn validate_graph_type_descriptor(
    descriptor: &GraphTypeDescriptor,
) -> Result<(), GraphTypeCatalogError> {
    let id = descriptor.id.as_str();
    validate_stable_identifier("graph type id", id).map_err(|reason| {
        GraphTypeCatalogError::InvalidGraphTypeId {
            id: id.to_string(),
            reason,
        }
    })?;
    if descriptor.version == 0 {
        return Err(GraphTypeCatalogError::ReservedGraphTypeVersion { id: id.to_string() });
    }
    if descriptor.display_name.trim().is_empty() {
        return Err(GraphTypeCatalogError::EmptyDisplayName { id: id.to_string() });
    }
    for segment in &descriptor.category_path {
        if segment.trim().is_empty() {
            return Err(GraphTypeCatalogError::EmptyCategorySegment { id: id.to_string() });
        }
    }

    validate_graph_source_workflow(id, &descriptor.source_workflow)?;
    validate_graph_document_template(id, &descriptor.template)?;
    for requirement in &descriptor.allowed_node_catalogs {
        validate_graph_node_catalog_requirement(id, requirement)?;
    }
    if let Some(backend) = &descriptor.compiler_backend {
        validate_graph_compiler_backend(id, backend)?;
    }
    if let Some(product) = &descriptor.runtime_product {
        validate_runtime_graph_product(id, product)?;
    }
    if let (Some(backend), Some(product)) =
        (&descriptor.compiler_backend, &descriptor.runtime_product)
    {
        validate_compiler_backend_runtime_strategy(id, backend, &product.execution_strategy)?;
    }
    if descriptor.execution_mode.requires_runtime_product() {
        if descriptor.compiler_backend.is_none() {
            return Err(GraphTypeCatalogError::MissingCompilerBackend { id: id.to_string() });
        }
        if descriptor.runtime_product.is_none() {
            return Err(GraphTypeCatalogError::MissingRuntimeProduct { id: id.to_string() });
        }
    }
    validate_graph_palette_policy(id, &descriptor.palette_policy)?;
    for tag in &descriptor.tags {
        validate_stable_identifier("graph type tag", tag).map_err(|reason| {
            GraphTypeCatalogError::InvalidTag {
                id: id.to_string(),
                reason,
            }
        })?;
    }
    Ok(())
}

fn validate_graph_source_workflow(
    graph_type_id: &str,
    workflow: &GraphSourceWorkflow,
) -> Result<(), GraphTypeCatalogError> {
    validate_stable_identifier("graph source workflow id", &workflow.workflow_id).map_err(
        |reason| GraphTypeCatalogError::InvalidSourceWorkflow {
            id: graph_type_id.to_string(),
            reason,
        },
    )?;
    match workflow.kind {
        GraphSourceWorkflowKind::ProjectDocument => {
            let Some(source_schema) = workflow.source_schema.as_deref() else {
                return Err(GraphTypeCatalogError::InvalidSourceWorkflow {
                    id: graph_type_id.to_string(),
                    reason: "project document workflow requires a source schema".to_string(),
                });
            };
            validate_stable_identifier("graph source schema", source_schema).map_err(|reason| {
                GraphTypeCatalogError::InvalidSourceWorkflow {
                    id: graph_type_id.to_string(),
                    reason,
                }
            })?;
        }
        GraphSourceWorkflowKind::File => {
            let Some(extension) = workflow.default_extension.as_deref() else {
                return Err(GraphTypeCatalogError::InvalidSourceWorkflow {
                    id: graph_type_id.to_string(),
                    reason: "file workflow requires a default extension".to_string(),
                });
            };
            validate_path_token("graph source extension", extension).map_err(|reason| {
                GraphTypeCatalogError::InvalidSourceWorkflow {
                    id: graph_type_id.to_string(),
                    reason,
                }
            })?;
        }
    }
    if let Some(path_prefix) = workflow.default_path_prefix.as_deref() {
        validate_path_token("graph source path prefix", path_prefix).map_err(|reason| {
            GraphTypeCatalogError::InvalidSourceWorkflow {
                id: graph_type_id.to_string(),
                reason,
            }
        })?;
    }
    Ok(())
}

fn validate_graph_document_template(
    graph_type_id: &str,
    template: &GraphDocumentTemplate,
) -> Result<(), GraphTypeCatalogError> {
    if template.document.document_version == 0 {
        return Err(GraphTypeCatalogError::InvalidTemplate {
            id: graph_type_id.to_string(),
            reason: "document version 0 is reserved".to_string(),
        });
    }
    if template.document.graph_type != graph_type_id {
        return Err(GraphTypeCatalogError::InvalidTemplate {
            id: graph_type_id.to_string(),
            reason: format!(
                "template graph type `{}` does not match descriptor id `{graph_type_id}`",
                template.document.graph_type
            ),
        });
    }
    validate_stable_identifier("template graph type", &template.document.graph_type).map_err(
        |reason| GraphTypeCatalogError::InvalidTemplate {
            id: graph_type_id.to_string(),
            reason,
        },
    )?;
    if let Some(hash) = &template.document.required_catalog_hash
        && hash.len() != blake3::OUT_LEN
    {
        return Err(GraphTypeCatalogError::InvalidTemplate {
            id: graph_type_id.to_string(),
            reason: format!(
                "template requires catalog hash with {} bytes, got {}",
                blake3::OUT_LEN,
                hash.len()
            ),
        });
    }
    Ok(())
}

fn validate_graph_node_catalog_requirement(
    graph_type_id: &str,
    requirement: &GraphNodeCatalogRequirement,
) -> Result<(), GraphTypeCatalogError> {
    validate_stable_identifier("graph node catalog id", &requirement.catalog_id).map_err(
        |reason| GraphTypeCatalogError::InvalidNodeCatalogRequirement {
            id: graph_type_id.to_string(),
            reason,
        },
    )?;
    if requirement.minimum_version == Some(0) {
        return Err(GraphTypeCatalogError::InvalidNodeCatalogRequirement {
            id: graph_type_id.to_string(),
            reason: "minimum node catalog version 0 is reserved".to_string(),
        });
    }
    if let Some(hash) = &requirement.required_hash
        && hash.len() != blake3::OUT_LEN
    {
        return Err(GraphTypeCatalogError::InvalidNodeCatalogRequirement {
            id: graph_type_id.to_string(),
            reason: format!(
                "required node catalog hash must be {} bytes, got {}",
                blake3::OUT_LEN,
                hash.len()
            ),
        });
    }
    Ok(())
}

fn validate_graph_compiler_backend(
    graph_type_id: &str,
    backend: &GraphCompilerBackendDescriptor,
) -> Result<(), GraphTypeCatalogError> {
    validate_stable_identifier("graph compiler backend id", &backend.id).map_err(|reason| {
        GraphTypeCatalogError::InvalidCompilerBackend {
            id: graph_type_id.to_string(),
            reason,
        }
    })?;
    let invalid = match &backend.kind {
        GraphCompilerBackendKind::GeneratedRust {
            package,
            entry_symbol,
            abi: _,
        } => package
            .trim()
            .is_empty()
            .then_some("empty generated Rust package")
            .or_else(|| {
                entry_symbol
                    .trim()
                    .is_empty()
                    .then_some("empty generated Rust entry symbol")
            }),
        GraphCompilerBackendKind::PackedIr { ir_schema } => ir_schema
            .trim()
            .is_empty()
            .then_some("empty packed IR schema"),
        GraphCompilerBackendKind::ShaderPipeline { pipeline_kind } => pipeline_kind
            .trim()
            .is_empty()
            .then_some("empty shader pipeline kind"),
        GraphCompilerBackendKind::External { kind, locator } => kind
            .trim()
            .is_empty()
            .then_some("empty external compiler kind")
            .or_else(|| {
                locator
                    .trim()
                    .is_empty()
                    .then_some("empty external locator")
            }),
    };
    if let Some(reason) = invalid {
        return Err(GraphTypeCatalogError::InvalidCompilerBackend {
            id: graph_type_id.to_string(),
            reason: reason.to_string(),
        });
    }
    for marker in &backend.capability_markers {
        validate_stable_identifier("graph compiler capability marker", marker).map_err(
            |reason| GraphTypeCatalogError::InvalidCompilerBackend {
                id: graph_type_id.to_string(),
                reason,
            },
        )?;
    }
    Ok(())
}

fn validate_runtime_graph_product(
    graph_type_id: &str,
    product: &RuntimeGraphProductDescriptor,
) -> Result<(), GraphTypeCatalogError> {
    validate_stable_identifier("runtime graph asset type", &product.asset_type).map_err(
        |reason| GraphTypeCatalogError::InvalidRuntimeProduct {
            id: graph_type_id.to_string(),
            reason,
        },
    )?;
    validate_stable_identifier("runtime graph product kind", &product.product_kind).map_err(
        |reason| GraphTypeCatalogError::InvalidRuntimeProduct {
            id: graph_type_id.to_string(),
            reason,
        },
    )?;
    validate_runtime_graph_execution_strategy(graph_type_id, &product.execution_strategy)
}

fn validate_runtime_graph_execution_strategy(
    graph_type_id: &str,
    strategy: &RuntimeGraphExecutionStrategy,
) -> Result<(), GraphTypeCatalogError> {
    let invalid = match strategy {
        RuntimeGraphExecutionStrategy::PackedIr => None,
        RuntimeGraphExecutionStrategy::AotCompiledCode {
            language,
            package,
            entry_symbol,
            context_type,
        } => invalid_runtime_strategy_field("AOT compiled language", language)
            .or_else(|| invalid_runtime_strategy_field("AOT compiled package", package))
            .or_else(|| invalid_runtime_strategy_field("AOT compiled entry symbol", entry_symbol))
            .or_else(|| invalid_runtime_strategy_field("AOT compiled context type", context_type)),
        RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { abi, entry_symbol } => {
            invalid_runtime_strategy_field("hot-reloaded module ABI", abi).or_else(|| {
                invalid_runtime_strategy_field("hot-reloaded module entry symbol", entry_symbol)
            })
        }
        RuntimeGraphExecutionStrategy::ShaderPipeline { pipeline_kind } => {
            invalid_runtime_strategy_field("shader pipeline kind", pipeline_kind)
        }
        RuntimeGraphExecutionStrategy::External { kind, locator } => {
            invalid_runtime_strategy_field("external runtime strategy kind", kind).or_else(|| {
                invalid_runtime_strategy_field("external runtime strategy locator", locator)
            })
        }
    };
    if let Some(reason) = invalid {
        return Err(GraphTypeCatalogError::InvalidRuntimeProduct {
            id: graph_type_id.to_string(),
            reason,
        });
    }
    Ok(())
}

fn invalid_runtime_strategy_field(label: &'static str, value: &str) -> Option<String> {
    validate_stable_identifier(label, value).err()
}

fn validate_compiler_backend_runtime_strategy(
    graph_type_id: &str,
    backend: &GraphCompilerBackendDescriptor,
    strategy: &RuntimeGraphExecutionStrategy,
) -> Result<(), GraphTypeCatalogError> {
    let reason = match (&backend.kind, strategy) {
        (GraphCompilerBackendKind::PackedIr { .. }, RuntimeGraphExecutionStrategy::PackedIr) => {
            None
        }
        (
            GraphCompilerBackendKind::PackedIr { .. },
            RuntimeGraphExecutionStrategy::AotCompiledCode { .. }
            | RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { .. }
            | RuntimeGraphExecutionStrategy::ShaderPipeline { .. }
            | RuntimeGraphExecutionStrategy::External { .. },
        ) => Some("packed IR compiler backend must use packed IR runtime strategy".to_string()),
        (
            GraphCompilerBackendKind::GeneratedRust {
                abi:
                    GeneratedRustGraphAbi::ContextSchedule
                    | GeneratedRustGraphAbi::TypedDataflow,
                ..
            },
            RuntimeGraphExecutionStrategy::AotCompiledCode { language, .. },
        ) if language == "rust" => None,
        (
            GraphCompilerBackendKind::GeneratedRust {
                abi:
                    GeneratedRustGraphAbi::ContextSchedule
                    | GeneratedRustGraphAbi::TypedDataflow,
                ..
            },
            RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { .. },
        ) => None,
        (
            GraphCompilerBackendKind::GeneratedRust { .. },
            RuntimeGraphExecutionStrategy::AotCompiledCode { language, .. },
        ) => Some(format!(
            "generated Rust compiler backend cannot declare AOT `{language}` runtime strategy"
        )),
        (
            GraphCompilerBackendKind::GeneratedRust { .. },
            RuntimeGraphExecutionStrategy::PackedIr
            | RuntimeGraphExecutionStrategy::ShaderPipeline { .. }
            | RuntimeGraphExecutionStrategy::External { .. },
        ) => Some("generated Rust compiler backend must declare Rust AOT or hot-reloaded module runtime strategy".to_string()),
        (
            GraphCompilerBackendKind::ShaderPipeline { pipeline_kind },
            RuntimeGraphExecutionStrategy::ShaderPipeline {
                pipeline_kind: runtime_pipeline_kind,
            },
        ) if pipeline_kind == runtime_pipeline_kind => None,
        (
            GraphCompilerBackendKind::ShaderPipeline { pipeline_kind },
            RuntimeGraphExecutionStrategy::ShaderPipeline {
                pipeline_kind: runtime_pipeline_kind,
            },
        ) => Some(format!(
            "shader compiler pipeline `{pipeline_kind}` does not match runtime shader pipeline `{runtime_pipeline_kind}`"
        )),
        (
            GraphCompilerBackendKind::ShaderPipeline { .. },
            RuntimeGraphExecutionStrategy::PackedIr
            | RuntimeGraphExecutionStrategy::AotCompiledCode { .. }
            | RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { .. }
            | RuntimeGraphExecutionStrategy::External { .. },
        ) => Some("shader compiler backend must declare shader pipeline runtime strategy".to_string()),
        (
            GraphCompilerBackendKind::External { kind, .. },
            RuntimeGraphExecutionStrategy::External {
                kind: runtime_kind, ..
            },
        ) if kind == runtime_kind => None,
        (
            GraphCompilerBackendKind::External { kind, .. },
            RuntimeGraphExecutionStrategy::External {
                kind: runtime_kind, ..
            },
        ) => Some(format!(
            "external compiler kind `{kind}` does not match external runtime kind `{runtime_kind}`"
        )),
        (
            GraphCompilerBackendKind::External { .. },
            RuntimeGraphExecutionStrategy::PackedIr
            | RuntimeGraphExecutionStrategy::AotCompiledCode { .. }
            | RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { .. }
            | RuntimeGraphExecutionStrategy::ShaderPipeline { .. },
        ) => Some("external compiler backend must declare an external runtime strategy".to_string()),
    };
    if let Some(reason) = reason {
        return Err(GraphTypeCatalogError::InvalidRuntimeProduct {
            id: graph_type_id.to_string(),
            reason,
        });
    }
    Ok(())
}

fn validate_graph_palette_policy(
    graph_type_id: &str,
    policy: &GraphPalettePolicy,
) -> Result<(), GraphTypeCatalogError> {
    for category in &policy.root_categories {
        validate_stable_identifier("graph palette root category", category).map_err(|reason| {
            GraphTypeCatalogError::InvalidPalettePolicy {
                id: graph_type_id.to_string(),
                reason,
            }
        })?;
    }
    for capability in &policy.required_node_capabilities {
        validate_stable_identifier("graph palette required node capability", capability).map_err(
            |reason| GraphTypeCatalogError::InvalidPalettePolicy {
                id: graph_type_id.to_string(),
                reason,
            },
        )?;
    }
    for tag in &policy.hidden_node_tags {
        validate_stable_identifier("graph palette hidden node tag", tag).map_err(|reason| {
            GraphTypeCatalogError::InvalidPalettePolicy {
                id: graph_type_id.to_string(),
                reason,
            }
        })?;
    }
    Ok(())
}

fn validate_node_type_descriptor(
    descriptor: &NodeTypeDescriptor,
) -> Result<(), NodeTypeCatalogError> {
    validate_stable_identifier("node type id", descriptor.id.as_str()).map_err(|reason| {
        NodeTypeCatalogError::InvalidNodeTypeId {
            id: descriptor.id.as_str().to_string(),
            reason,
        }
    })?;
    if descriptor.version == 0 {
        return Err(NodeTypeCatalogError::ReservedNodeTypeVersion {
            id: descriptor.id.as_str().to_string(),
        });
    }
    if descriptor.display_name.trim().is_empty() {
        return Err(NodeTypeCatalogError::EmptyDisplayName {
            id: descriptor.id.as_str().to_string(),
        });
    }
    for segment in &descriptor.category_path {
        if segment.trim().is_empty() {
            return Err(NodeTypeCatalogError::EmptyCategorySegment {
                id: descriptor.id.as_str().to_string(),
            });
        }
    }

    let mut seen_ports = BTreeSet::new();
    let mut seen_port_orders = BTreeSet::new();
    for port in &descriptor.ports {
        validate_port_descriptor(descriptor, port)?;
        if !seen_ports.insert(port.id) {
            return Err(NodeTypeCatalogError::DuplicatePortId {
                id: descriptor.id.as_str().to_string(),
                port_id: port.id,
            });
        }
        if let Some(order) = port.layout.order
            && !seen_port_orders.insert((port.layout.side, order))
        {
            return Err(NodeTypeCatalogError::DuplicatePortLayoutOrder {
                id: descriptor.id.as_str().to_string(),
                side: port.layout.side,
                order,
            });
        }
    }

    for capability in &descriptor.capabilities {
        validate_stable_identifier("node capability", &capability.id).map_err(|reason| {
            NodeTypeCatalogError::InvalidCapability {
                id: descriptor.id.as_str().to_string(),
                reason,
            }
        })?;
        for marker in &capability.markers {
            validate_stable_identifier("node capability marker", marker).map_err(|reason| {
                NodeTypeCatalogError::InvalidCapability {
                    id: descriptor.id.as_str().to_string(),
                    reason,
                }
            })?;
        }
    }

    if let Some(binding) = &descriptor.runtime_binding {
        validate_runtime_binding(descriptor, binding)?;
    }
    for link in &descriptor.source_links {
        validate_source_link(descriptor, link)?;
    }
    Ok(())
}

fn validate_port_descriptor(
    descriptor: &NodeTypeDescriptor,
    port: &NodePortDescriptor,
) -> Result<(), NodeTypeCatalogError> {
    if port.id.is_reserved() {
        return Err(NodeTypeCatalogError::ReservedPortId {
            id: descriptor.id.as_str().to_string(),
            port: port.name.clone(),
        });
    }
    validate_stable_identifier("node port name", &port.name).map_err(|reason| {
        NodeTypeCatalogError::InvalidPortName {
            id: descriptor.id.as_str().to_string(),
            port: port.name.clone(),
            reason,
        }
    })?;
    if let NodePortAttachment::FixedFraction { per_mille } = port.layout.attachment
        && per_mille > 1000
    {
        return Err(NodeTypeCatalogError::InvalidPortAttachment {
            id: descriptor.id.as_str().to_string(),
            port: port.name.clone(),
            per_mille,
        });
    }
    match &port.value {
        NodePortValue::Execution => {
            if port.default_value.is_some() {
                return Err(NodeTypeCatalogError::ExecutionPortDefaultValue {
                    id: descriptor.id.as_str().to_string(),
                    port: port.name.clone(),
                });
            }
        }
        NodePortValue::Data { schema_type } => {
            if schema_type.trim().is_empty() {
                return Err(NodeTypeCatalogError::EmptyPortSchema {
                    id: descriptor.id.as_str().to_string(),
                    port: port.name.clone(),
                });
            }
        }
        NodePortValue::DynamicData {
            group,
            accepted_schema_types,
        } => {
            if group.trim().is_empty() {
                return Err(NodeTypeCatalogError::EmptyDynamicGroup {
                    id: descriptor.id.as_str().to_string(),
                    port: port.name.clone(),
                });
            }
            for schema_type in accepted_schema_types {
                if schema_type.trim().is_empty() {
                    return Err(NodeTypeCatalogError::EmptyPortSchema {
                        id: descriptor.id.as_str().to_string(),
                        port: port.name.clone(),
                    });
                }
            }
        }
    }
    if port.direction == NodePortDirection::Output && port.default_value.is_some() {
        return Err(NodeTypeCatalogError::OutputPortDefaultValue {
            id: descriptor.id.as_str().to_string(),
            port: port.name.clone(),
        });
    }
    Ok(())
}

fn validate_runtime_binding(
    descriptor: &NodeTypeDescriptor,
    binding: &NodeRuntimeBinding,
) -> Result<(), NodeTypeCatalogError> {
    let invalid = match binding {
        NodeRuntimeBinding::RustSymbol {
            package,
            symbol,
            call_abi,
        } => package
            .trim()
            .is_empty()
            .then_some("empty Rust package".to_string())
            .or_else(|| {
                symbol
                    .trim()
                    .is_empty()
                    .then_some("empty Rust symbol".to_string())
            })
            .or_else(|| invalid_rust_node_call_abi(descriptor, call_abi)),
        NodeRuntimeBinding::AssetBuilder { builder_id } => builder_id
            .trim()
            .is_empty()
            .then_some("empty asset-builder id".to_string()),
        NodeRuntimeBinding::RuntimeComponent { component_type } => component_type
            .trim()
            .is_empty()
            .then_some("empty runtime component type".to_string()),
        NodeRuntimeBinding::External { kind, locator } => kind
            .trim()
            .is_empty()
            .then_some("empty external binding kind".to_string())
            .or_else(|| {
                locator
                    .trim()
                    .is_empty()
                    .then_some("empty external locator".to_string())
            }),
    };

    if let Some(reason) = invalid {
        return Err(NodeTypeCatalogError::InvalidRuntimeBinding {
            id: descriptor.id.as_str().to_string(),
            reason,
        });
    }
    Ok(())
}

fn invalid_rust_node_call_abi(
    descriptor: &NodeTypeDescriptor,
    call_abi: &RustNodeCallAbi,
) -> Option<String> {
    match call_abi {
        RustNodeCallAbi::ContextSchedule => None,
        RustNodeCallAbi::TypedDataflow(dataflow) => {
            invalid_rust_dataflow_node_call(descriptor, dataflow)
        }
    }
}

fn invalid_rust_dataflow_node_call(
    descriptor: &NodeTypeDescriptor,
    dataflow: &RustTypedDataflowNodeCall,
) -> Option<String> {
    let mut mapped_inputs = BTreeSet::new();
    for parameter in &dataflow.parameters {
        if parameter.rust_type.trim().is_empty() {
            return Some("typed dataflow parameter has empty Rust type".to_string());
        }
        if let RustDataflowParameterSource::InputPort { port } = parameter.source {
            let Some(port_descriptor) = descriptor
                .ports
                .iter()
                .find(|candidate| candidate.id == port)
            else {
                return Some(format!(
                    "typed dataflow parameter references unknown input port {port}"
                ));
            };
            if port_descriptor.direction != NodePortDirection::Input
                || !port_descriptor.value.is_data()
            {
                return Some(format!(
                    "typed dataflow parameter port {port} is not an input data port"
                ));
            }
            if !mapped_inputs.insert(port) {
                return Some(format!(
                    "typed dataflow maps input port {port} more than once"
                ));
            }
        }
    }

    let mut mapped_outputs = BTreeSet::new();
    match &dataflow.output {
        RustDataflowOutput::None => {}
        RustDataflowOutput::Single { port, rust_type } => {
            if rust_type.trim().is_empty() {
                return Some("typed dataflow single output has empty Rust type".to_string());
            }
            if let Some(reason) =
                invalid_rust_dataflow_output_port(descriptor, *port, &mut mapped_outputs)
            {
                return Some(reason);
            }
        }
        RustDataflowOutput::StructFields { rust_type, fields } => {
            if rust_type.trim().is_empty() {
                return Some("typed dataflow struct output has empty Rust type".to_string());
            }
            if fields.is_empty() {
                return Some("typed dataflow struct output has no fields".to_string());
            }
            for field in fields {
                if field.rust_type.trim().is_empty() {
                    return Some("typed dataflow struct field has empty Rust type".to_string());
                }
                if field.field.trim().is_empty() {
                    return Some(
                        "typed dataflow struct field has empty field accessor".to_string(),
                    );
                }
                if let Some(reason) =
                    invalid_rust_dataflow_output_port(descriptor, field.port, &mut mapped_outputs)
                {
                    return Some(reason);
                }
            }
        }
    }
    None
}

fn invalid_rust_dataflow_output_port(
    descriptor: &NodeTypeDescriptor,
    port: NodePortId,
    mapped_outputs: &mut BTreeSet<NodePortId>,
) -> Option<String> {
    let Some(port_descriptor) = descriptor
        .ports
        .iter()
        .find(|candidate| candidate.id == port)
    else {
        return Some(format!(
            "typed dataflow output references unknown output port {port}"
        ));
    };
    if port_descriptor.direction != NodePortDirection::Output || !port_descriptor.value.is_data() {
        return Some(format!(
            "typed dataflow output port {port} is not an output data port"
        ));
    }
    if !mapped_outputs.insert(port) {
        return Some(format!(
            "typed dataflow maps output port {port} more than once"
        ));
    }
    None
}

fn validate_source_link(
    descriptor: &NodeTypeDescriptor,
    link: &NodeSourceLink,
) -> Result<(), NodeTypeCatalogError> {
    let has_target = link
        .package
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty())
        || link
            .module_path
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        || link
            .symbol_path
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        || link
            .file
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        || link
            .docs_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
    if !has_target {
        return Err(NodeTypeCatalogError::InvalidSourceLink {
            id: descriptor.id.as_str().to_string(),
            reason: "source link must include at least one target".to_string(),
        });
    }
    if link.line == Some(0) {
        return Err(NodeTypeCatalogError::InvalidSourceLink {
            id: descriptor.id.as_str().to_string(),
            reason: "line number 0 is reserved".to_string(),
        });
    }
    if link.column == Some(0) {
        return Err(NodeTypeCatalogError::InvalidSourceLink {
            id: descriptor.id.as_str().to_string(),
            reason: "column number 0 is reserved".to_string(),
        });
    }
    Ok(())
}

fn normalize_source_file_path(file: &str) -> String {
    file.replace('\\', "/")
}

fn validate_visual_graph_document(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
) -> Result<(), VisualGraphValidationError> {
    if document.document_version == 0 {
        return Err(VisualGraphValidationError::ReservedDocumentVersion);
    }
    validate_stable_identifier("graph type", &document.graph_type)
        .map_err(|reason| VisualGraphValidationError::InvalidGraphType { reason })?;
    if let Some(hash) = &document.required_catalog_hash
        && hash.len() != blake3::OUT_LEN
    {
        return Err(VisualGraphValidationError::InvalidCatalogHashLength {
            expected: blake3::OUT_LEN,
            actual: hash.len(),
        });
    }

    let mut nodes_by_id = BTreeMap::new();
    for node in &document.nodes {
        if nodes_by_id.insert(node.id, node).is_some() {
            return Err(VisualGraphValidationError::DuplicateNodeId { node_id: node.id });
        }
        let descriptor = catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .ok_or_else(|| VisualGraphValidationError::UnknownNodeType {
                node_id: node.id,
                node_type: node.node_type.as_str().to_string(),
                version: node.node_type_version,
            })?;
        if !node.layout.x.is_finite() || !node.layout.y.is_finite() {
            return Err(VisualGraphValidationError::NonFiniteNodeLayout { node_id: node.id });
        }
        validate_node_values(node, descriptor)?;
    }

    let mut connection_ids = BTreeSet::new();
    let mut connection_counts = BTreeMap::<GraphPortRef, usize>::new();
    for connection in &document.connections {
        if !connection_ids.insert(connection.id) {
            return Err(VisualGraphValidationError::DuplicateConnectionId {
                connection_id: connection.id,
            });
        }
        validate_connection_route(connection)?;
        let from_node =
            node_for_connection(document, &nodes_by_id, connection.id, &connection.from)?;
        let to_node = node_for_connection(document, &nodes_by_id, connection.id, &connection.to)?;
        let from_type = catalog
            .node_type_version(&from_node.node_type, from_node.node_type_version)
            .expect("node type was validated before connections");
        let to_type = catalog
            .node_type_version(&to_node.node_type, to_node.node_type_version)
            .expect("node type was validated before connections");
        let from_port = port_for_connection(connection.id, from_node, from_type, &connection.from)?;
        let to_port = port_for_connection(connection.id, to_node, to_type, &connection.to)?;
        validate_connection_shape(connection.id, from_port, to_port)?;

        *connection_counts
            .entry(connection.from.clone())
            .or_default() += 1;
        *connection_counts.entry(connection.to.clone()).or_default() += 1;
    }
    validate_connection_capacities(document, catalog, &nodes_by_id, &connection_counts)?;
    validate_comments(document)?;
    Ok(())
}

fn validate_node_values(
    node: &GraphNode,
    descriptor: &NodeTypeDescriptor,
) -> Result<(), VisualGraphValidationError> {
    for port_id in node.input_values.keys() {
        let Some(port) = descriptor.ports.iter().find(|port| port.id == *port_id) else {
            return Err(VisualGraphValidationError::UnknownInputValuePort {
                node_id: node.id,
                port_id: *port_id,
            });
        };
        if port.direction != NodePortDirection::Input || !port.value.is_data() {
            return Err(VisualGraphValidationError::NonInputDataValue {
                node_id: node.id,
                port_id: *port_id,
            });
        }
    }
    Ok(())
}

fn node_for_connection<'a>(
    document: &'a VisualGraphDocument,
    nodes_by_id: &BTreeMap<GraphNodeId, &'a GraphNode>,
    connection_id: GraphConnectionId,
    port_ref: &GraphPortRef,
) -> Result<&'a GraphNode, VisualGraphValidationError> {
    nodes_by_id.get(&port_ref.node_id).copied().ok_or_else(|| {
        debug_assert!(
            !document
                .nodes
                .iter()
                .any(|node| node.id == port_ref.node_id),
            "node map must include every graph node"
        );
        VisualGraphValidationError::UnknownConnectionNode {
            connection_id,
            node_id: port_ref.node_id,
        }
    })
}

fn port_for_connection<'a>(
    connection_id: GraphConnectionId,
    node: &GraphNode,
    node_type: &'a NodeTypeDescriptor,
    port_ref: &GraphPortRef,
) -> Result<&'a NodePortDescriptor, VisualGraphValidationError> {
    node_type
        .ports
        .iter()
        .find(|port| port.id == port_ref.port_id)
        .ok_or(VisualGraphValidationError::UnknownConnectionPort {
            connection_id,
            node_id: node.id,
            port_id: port_ref.port_id,
        })
}

fn validate_connection_shape(
    connection_id: GraphConnectionId,
    from_port: &NodePortDescriptor,
    to_port: &NodePortDescriptor,
) -> Result<(), VisualGraphValidationError> {
    if from_port.direction != NodePortDirection::Output
        || to_port.direction != NodePortDirection::Input
    {
        return Err(VisualGraphValidationError::InvalidConnectionDirection { connection_id });
    }

    match (&from_port.value, &to_port.value) {
        (NodePortValue::Execution, NodePortValue::Execution) => Ok(()),
        (NodePortValue::Data { schema_type: left }, NodePortValue::Data { schema_type: right })
            if left == right =>
        {
            Ok(())
        }
        (
            NodePortValue::DynamicData {
                accepted_schema_types,
                ..
            },
            NodePortValue::Data { schema_type },
        )
        | (
            NodePortValue::Data { schema_type },
            NodePortValue::DynamicData {
                accepted_schema_types,
                ..
            },
        ) if accepted_schema_types.is_empty() || accepted_schema_types.contains(schema_type) => {
            Ok(())
        }
        (
            NodePortValue::DynamicData {
                group: left_group, ..
            },
            NodePortValue::DynamicData {
                group: right_group, ..
            },
        ) if left_group == right_group => Ok(()),
        _ => Err(VisualGraphValidationError::IncompatibleConnection {
            connection_id,
            reason: format!(
                "output `{}` is not compatible with input `{}`",
                port_value_label(&from_port.value),
                port_value_label(&to_port.value)
            ),
        }),
    }
}

fn validate_connection_route(
    connection: &GraphConnection,
) -> Result<(), VisualGraphValidationError> {
    let mut route_anchor_ids = BTreeSet::new();
    for anchor in &connection.route.anchors {
        if !route_anchor_ids.insert(anchor.id) {
            return Err(VisualGraphValidationError::DuplicateRouteAnchorId {
                connection_id: connection.id,
                anchor_id: anchor.id,
            });
        }
        if !anchor.position.x.is_finite() || !anchor.position.y.is_finite() {
            return Err(VisualGraphValidationError::NonFiniteRouteAnchor {
                connection_id: connection.id,
                anchor_id: anchor.id,
            });
        }
    }
    Ok(())
}

fn validate_connection_capacities(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
    nodes_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
    connection_counts: &BTreeMap<GraphPortRef, usize>,
) -> Result<(), VisualGraphValidationError> {
    for (port_ref, count) in connection_counts {
        let node = nodes_by_id
            .get(&port_ref.node_id)
            .copied()
            .expect("connection counts are built from known graph nodes");
        let node_type = catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .expect("node type was validated before connection capacities");
        let port = node_type
            .ports
            .iter()
            .find(|port| port.id == port_ref.port_id)
            .expect("connection counts are built from known graph ports");
        if port.capacity == NodePortCapacity::Single && *count > 1 {
            return Err(VisualGraphValidationError::PortCapacityExceeded {
                node_id: port_ref.node_id,
                port_id: port_ref.port_id,
            });
        }
    }

    debug_assert!(
        document
            .connections
            .iter()
            .all(
                |connection| connection_counts.contains_key(&connection.from)
                    && connection_counts.contains_key(&connection.to)
            ),
        "every connection endpoint must have a capacity count"
    );
    Ok(())
}

fn validate_comments(document: &VisualGraphDocument) -> Result<(), VisualGraphValidationError> {
    let mut comment_ids = BTreeSet::new();
    for comment in &document.comments {
        if !comment_ids.insert(comment.id) {
            return Err(VisualGraphValidationError::DuplicateCommentId {
                comment_id: comment.id,
            });
        }
        let bounds = comment.bounds;
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
        {
            return Err(VisualGraphValidationError::NonFiniteCommentBounds {
                comment_id: comment.id,
            });
        }
    }
    Ok(())
}

fn port_value_label(value: &NodePortValue) -> String {
    match value {
        NodePortValue::Execution => "execution".to_string(),
        NodePortValue::Data { schema_type } => format!("data:{schema_type}"),
        NodePortValue::DynamicData { group, .. } => format!("dynamic:{group}"),
    }
}

fn validate_stable_identifier(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} is empty"));
    }
    if value.trim() != value {
        return Err(format!("{label} has leading or trailing whitespace"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} contains a control character"));
    }
    if value.contains('\\') {
        return Err(format!("{label} contains a backslash"));
    }
    Ok(())
}

fn validate_path_token(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} is empty"));
    }
    if value.trim() != value {
        return Err(format!("{label} has leading or trailing whitespace"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} contains a control character"));
    }
    if value.contains('\\') {
        return Err(format!("{label} contains a backslash"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, GEM_INFRASTRUCTURE_DEPENDENCIES,
        PROJECT_OR_FORMAT_PATH_SEGMENTS, PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
        collect_production_rust_sources, forbidden_production_dependencies,
    };
    use az_gem_contract::{
        ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
        GemId, GemTargetRole, ProductActivation, declare_caps,
    };
    use toml::Value;

    const STRING_SCHEMA: &str = "core.string";
    const FLOAT_SCHEMA: &str = "core.f32";

    #[test]
    fn node_catalog_hash_is_stable_after_descriptor_sorting() {
        let left = NodeTypeCatalog::try_new(1, 100, vec![float_node(), string_node()]).unwrap();
        let right = NodeTypeCatalog::try_new(1, 100, vec![string_node(), float_node()]).unwrap();

        assert_eq!(left.node_types[0].id.as_str(), "azoth.test.float");
        assert_eq!(left.content_hash().unwrap(), right.content_hash().unwrap());
    }

    #[test]
    fn node_catalog_rejects_duplicate_node_type_versions() {
        let error = NodeTypeCatalog::try_new(1, 100, vec![float_node(), float_node()]).unwrap_err();

        assert_eq!(
            error,
            NodeTypeCatalogError::DuplicateNodeType {
                id: "azoth.test.float".to_string(),
                version: 1,
            }
        );
    }

    #[test]
    fn graph_type_catalog_hash_is_stable_after_descriptor_sorting() {
        let left = GraphTypeCatalog::try_new(
            1,
            100,
            vec![editor_debug_graph_type(), runtime_logic_graph_type()],
        )
        .unwrap();
        let right = GraphTypeCatalog::try_new(
            1,
            100,
            vec![runtime_logic_graph_type(), editor_debug_graph_type()],
        )
        .unwrap();

        assert_eq!(left.graph_types[0].id.as_str(), "azoth.test.debug-graph");
        assert_eq!(left.content_hash().unwrap(), right.content_hash().unwrap());
    }

    #[test]
    fn graph_type_catalog_rejects_duplicate_graph_type_versions() {
        let error = GraphTypeCatalog::try_new(
            1,
            100,
            vec![runtime_logic_graph_type(), runtime_logic_graph_type()],
        )
        .unwrap_err();

        assert_eq!(
            error,
            GraphTypeCatalogError::DuplicateGraphType {
                id: "azoth.test.logic-graph".to_string(),
                version: 1,
            }
        );
    }

    #[test]
    fn runtime_compiled_graph_type_requires_compiler_and_runtime_product() {
        let mut missing_product = runtime_logic_graph_type();
        missing_product.runtime_product = None;
        assert!(matches!(
            GraphTypeCatalog::try_new(1, 100, vec![missing_product]),
            Err(GraphTypeCatalogError::MissingRuntimeProduct { .. })
        ));

        let mut missing_compiler = runtime_logic_graph_type();
        missing_compiler.compiler_backend = None;
        assert!(matches!(
            GraphTypeCatalog::try_new(1, 100, vec![missing_compiler]),
            Err(GraphTypeCatalogError::MissingCompilerBackend { .. })
        ));
    }

    #[test]
    fn graph_type_template_must_match_descriptor_graph_type() {
        let descriptor = runtime_logic_graph_type()
            .with_template(GraphDocumentTemplate::empty("azoth.test.other-graph"));

        assert!(matches!(
            GraphTypeCatalog::try_new(1, 100, vec![descriptor]),
            Err(GraphTypeCatalogError::InvalidTemplate { .. })
        ));
    }

    #[test]
    fn graph_type_descriptor_records_zero_cost_runtime_compiler_contract() {
        let descriptor = runtime_logic_graph_type();

        assert_eq!(
            descriptor.execution_mode,
            GraphExecutionMode::RuntimeCompiled
        );
        assert!(descriptor.execution_mode.requires_runtime_product());
        assert!(matches!(
            descriptor.compiler_backend.as_ref().map(|backend| &backend.kind),
            Some(GraphCompilerBackendKind::PackedIr { ir_schema }) if ir_schema == "azoth.graph.logic-ir/v1"
        ));
        let product = descriptor.runtime_product.as_ref().unwrap();
        assert_eq!(product.asset_type, "azoth.graph.packed-ir");
        assert!(product.streamable);
        assert!(product.diffable_chunks);
        assert_eq!(
            product.execution_strategy,
            RuntimeGraphExecutionStrategy::PackedIr
        );
    }

    #[test]
    fn generated_rust_graph_type_records_context_schedule_abi() {
        let descriptor = GraphTypeDescriptor::runtime_compiled(
            "azoth.test.generated-rust-context",
            1,
            "Generated Rust Context Graph",
            GraphSourceWorkflow::file("azoth.test.generated-rust-context.source", "azgraph.ron"),
            GraphCompilerBackendDescriptor::generated_rust_context_schedule(
                "azoth.test.generated-rust-context.compiler",
                "azoth-tests",
                "azoth_tests::compile",
            ),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.generated-rust",
                "azoth.graph.generated-rust",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "azoth-tests",
                    "azoth_tests::execute",
                    "azoth_tests::RuntimeContext",
                ),
            ),
        );

        let catalog = GraphTypeCatalog::try_new(1, 100, vec![descriptor]).unwrap();
        let backend = catalog.graph_types[0].compiler_backend.as_ref().unwrap();

        assert!(matches!(
            backend.kind,
            GraphCompilerBackendKind::GeneratedRust {
                abi: GeneratedRustGraphAbi::ContextSchedule,
                ..
            }
        ));
    }

    #[test]
    fn generated_rust_typed_dataflow_is_a_distinct_compiler_abi() {
        let descriptor = GraphTypeDescriptor::runtime_compiled(
            "azoth.test.generated-rust-dataflow",
            1,
            "Generated Rust Dataflow Graph",
            GraphSourceWorkflow::file("azoth.test.generated-rust-dataflow.source", "azgraph.ron"),
            GraphCompilerBackendDescriptor::generated_rust_typed_dataflow(
                "azoth.test.generated-rust-dataflow.compiler",
                "azoth-tests",
                "azoth_tests::compile_dataflow",
            ),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.generated-rust-dataflow",
                "azoth.graph.generated-rust-dataflow",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "azoth-tests",
                    "azoth_tests::execute_dataflow",
                    "azoth_tests::RuntimeContext",
                ),
            ),
        );

        let catalog = GraphTypeCatalog::try_new(1, 100, vec![descriptor]).unwrap();
        let backend = catalog.graph_types[0].compiler_backend.as_ref().unwrap();

        assert!(matches!(
            backend.kind,
            GraphCompilerBackendKind::GeneratedRust {
                abi: GeneratedRustGraphAbi::TypedDataflow,
                ..
            }
        ));
    }

    #[test]
    fn graph_type_rejects_packed_backend_with_non_packed_runtime_strategy() {
        let descriptor =
            runtime_logic_graph_type().with_runtime_product(RuntimeGraphProductDescriptor::new(
                "azoth.graph.generated-code",
                "azoth.graph.generated-code",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "azoth_tests",
                    "execute_graph",
                    "azoth_tests::RuntimeContext",
                ),
            ));

        let error = GraphTypeCatalog::try_new(1, 100, vec![descriptor]).unwrap_err();

        match error {
            GraphTypeCatalogError::InvalidRuntimeProduct { reason, .. } => {
                assert!(reason.contains("packed IR compiler backend"));
            }
            other => panic!("unexpected graph type catalog error: {other:?}"),
        }
    }

    #[test]
    fn graph_type_rejects_generated_rust_backend_with_packed_runtime_strategy() {
        let descriptor = runtime_logic_graph_type().with_compiler_backend(
            GraphCompilerBackendDescriptor::generated_rust_context_schedule(
                "azoth.test.logic-graph.generated-rust",
                "azoth_tests",
                "execute_graph",
            ),
        );

        let error = GraphTypeCatalog::try_new(1, 100, vec![descriptor]).unwrap_err();

        match error {
            GraphTypeCatalogError::InvalidRuntimeProduct { reason, .. } => {
                assert!(reason.contains("generated Rust compiler backend"));
            }
            other => panic!("unexpected graph type catalog error: {other:?}"),
        }
    }

    #[test]
    fn node_catalog_rejects_reserved_and_duplicate_ports() {
        let reserved = NodeTypeDescriptor::new("azoth.test.reserved", 1, "Reserved").with_port(
            NodePortDescriptor::new(
                NodePortId::INVALID,
                "value",
                NodePortDirection::Input,
                NodePortValue::Data {
                    schema_type: STRING_SCHEMA.to_string(),
                },
            ),
        );
        assert!(matches!(
            NodeTypeCatalog::try_new(1, 100, vec![reserved]),
            Err(NodeTypeCatalogError::ReservedPortId { .. })
        ));

        let duplicate = NodeTypeDescriptor::new("azoth.test.duplicate", 1, "Duplicate")
            .with_port(data_input(1, STRING_SCHEMA))
            .with_port(data_output(1, STRING_SCHEMA));
        assert!(matches!(
            NodeTypeCatalog::try_new(1, 100, vec![duplicate]),
            Err(NodeTypeCatalogError::DuplicatePortId { .. })
        ));
    }

    #[test]
    fn node_catalog_rejects_execution_and_output_defaults() {
        let execution_default =
            NodeTypeDescriptor::new("azoth.test.exec-default", 1, "Exec Default").with_port(
                NodePortDescriptor::new(
                    NodePortId::new(1),
                    "in",
                    NodePortDirection::Input,
                    NodePortValue::Execution,
                )
                .with_default_value(ReflectedValueEnvelope::typed_ron(STRING_SCHEMA, "true")),
            );
        assert!(matches!(
            NodeTypeCatalog::try_new(1, 100, vec![execution_default]),
            Err(NodeTypeCatalogError::ExecutionPortDefaultValue { .. })
        ));

        let output_default = NodeTypeDescriptor::new("azoth.test.output-default", 1, "Output")
            .with_port(
                data_output(1, STRING_SCHEMA)
                    .with_default_value(ReflectedValueEnvelope::typed_ron(STRING_SCHEMA, r#""x""#)),
            );
        assert!(matches!(
            NodeTypeCatalog::try_new(1, 100, vec![output_default]),
            Err(NodeTypeCatalogError::OutputPortDefaultValue { .. })
        ));
    }

    #[test]
    fn node_catalog_rejects_invalid_fixed_port_attachment() {
        let descriptor = NodeTypeDescriptor::new("azoth.test.fixed-port", 1, "Fixed Port")
            .with_port(
                data_input(1, STRING_SCHEMA)
                    .with_layout(NodePortLayout::input().with_fixed_fraction(1001)),
            );

        assert!(matches!(
            NodeTypeCatalog::try_new(1, 100, vec![descriptor]),
            Err(NodeTypeCatalogError::InvalidPortAttachment { .. })
        ));
    }

    #[test]
    fn visual_graph_validates_matching_data_connections() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node(), string_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let mut target = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        target.input_values.insert(
            NodePortId::new(1),
            ReflectedValueEnvelope::typed_ron(FLOAT_SCHEMA, "1.0"),
        );
        let connection = GraphConnection::new(
            test_connection_uuid(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        );
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source, target],
            connections: vec![connection],
            comments: Vec::new(),
        };

        document.validate_against(&catalog).unwrap();
    }

    #[test]
    fn visual_graph_rejects_unknown_ports_and_schema_mismatch() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node(), string_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.string", 1);
        let bad_port_document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source.clone(), target.clone()],
            connections: vec![GraphConnection::new(
                test_connection_uuid(1),
                GraphPortRef::new(source.id, NodePortId::new(99)),
                GraphPortRef::new(target.id, NodePortId::new(1)),
            )],
            comments: Vec::new(),
        };
        assert!(matches!(
            bad_port_document.validate_against(&catalog),
            Err(VisualGraphValidationError::UnknownConnectionPort { .. })
        ));

        let mismatch_document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source.clone(), target.clone()],
            connections: vec![GraphConnection::new(
                test_connection_uuid(2),
                GraphPortRef::new(source.id, NodePortId::new(2)),
                GraphPortRef::new(target.id, NodePortId::new(1)),
            )],
            comments: Vec::new(),
        };
        assert!(matches!(
            mismatch_document.validate_against(&catalog),
            Err(VisualGraphValidationError::IncompatibleConnection { .. })
        ));
    }

    #[test]
    fn visual_graph_rejects_multiple_connections_to_single_input() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node()]);
        let source_a = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let source_b = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(3), "azoth.test.float", 1);
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source_a.clone(), source_b.clone(), target.clone()],
            connections: vec![
                GraphConnection::new(
                    test_connection_uuid(1),
                    GraphPortRef::new(source_a.id, NodePortId::new(2)),
                    GraphPortRef::new(target.id, NodePortId::new(1)),
                ),
                GraphConnection::new(
                    test_connection_uuid(2),
                    GraphPortRef::new(source_b.id, NodePortId::new(2)),
                    GraphPortRef::new(target.id, NodePortId::new(1)),
                ),
            ],
            comments: Vec::new(),
        };

        assert!(matches!(
            document.validate_against(&catalog),
            Err(VisualGraphValidationError::PortCapacityExceeded {
                node_id,
                port_id
            }) if node_id == target.id && port_id == NodePortId::new(1)
        ));
    }

    #[test]
    fn graph_commands_apply_as_one_validated_transaction() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        let connection = GraphConnection::new(
            test_connection_uuid(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        );
        let mut document = VisualGraphDocument::new("azoth.graph.test");

        document
            .apply_commands(
                [
                    GraphCommand::AddNode { node: source },
                    GraphCommand::AddNode {
                        node: target.clone(),
                    },
                    GraphCommand::SetInputValue {
                        node_id: target.id,
                        port_id: NodePortId::new(1),
                        value: Some(ReflectedValueEnvelope::typed_ron(FLOAT_SCHEMA, "1.0")),
                    },
                    GraphCommand::MoveNode {
                        node_id: target.id,
                        layout: GraphNodeLayout { x: 12.0, y: 34.0 },
                    },
                    GraphCommand::Connect {
                        connection: connection.clone(),
                    },
                ],
                &catalog,
            )
            .unwrap();

        assert_eq!(document.nodes.len(), 2);
        assert_eq!(document.connections, vec![connection]);
        let target = document
            .nodes
            .iter()
            .find(|node| node.id == target.id)
            .unwrap();
        assert_eq!(target.layout, GraphNodeLayout { x: 12.0, y: 34.0 });
        assert!(target.input_values.contains_key(&NodePortId::new(1)));
    }

    #[test]
    fn visual_graph_document_ron_round_trips_with_schema_marker() {
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        let connection = GraphConnection::new(
            test_connection_uuid(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        )
        .with_route(GraphConnectionRoute::orthogonal().with_anchor(
            GraphRouteAnchor::user_waypoint(
                test_route_anchor_uuid(1),
                GraphPoint::new(128.0, 64.0),
            ),
        ));
        let document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source, target],
            connections: vec![connection],
            comments: Vec::new(),
        };

        let ron = encode_visual_graph_document_ron(&document).unwrap();
        assert!(ron.contains(VISUAL_GRAPH_DOCUMENT_SCHEMA));
        let decoded = decode_visual_graph_document_ron(&ron).unwrap();

        assert_eq!(decoded, document);
    }

    #[test]
    fn connection_routes_preserve_user_waypoints_and_segment_constraints() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        let connection = GraphConnection::new(
            test_connection_uuid(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        );
        let route = GraphConnectionRoute::orthogonal().with_anchor(
            GraphRouteAnchor::user_waypoint(
                test_route_anchor_uuid(1),
                GraphPoint::new(128.0, 64.0),
            )
            .with_outgoing_segment(GraphRouteSegmentConstraint::Fixed),
        );
        let mut document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source, target],
            connections: vec![connection.clone()],
            comments: Vec::new(),
        };

        document
            .apply_command(
                GraphCommand::SetConnectionRoute {
                    connection_id: connection.id,
                    route: route.clone(),
                },
                &catalog,
            )
            .unwrap();

        assert_eq!(document.connections[0].route, route);
    }

    #[test]
    fn invalid_connection_route_leaves_document_unchanged() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        let connection = GraphConnection::new(
            test_connection_uuid(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        );
        let mut document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source, target],
            connections: vec![connection.clone()],
            comments: Vec::new(),
        };
        let original = document.clone();
        let duplicate_anchor = test_route_anchor_uuid(1);
        let bad_route = GraphConnectionRoute::orthogonal()
            .with_anchor(GraphRouteAnchor::user_waypoint(
                duplicate_anchor,
                GraphPoint::new(1.0, 2.0),
            ))
            .with_anchor(GraphRouteAnchor::user_waypoint(
                duplicate_anchor,
                GraphPoint::new(f32::NAN, 4.0),
            ));

        let error = document
            .apply_command(
                GraphCommand::SetConnectionRoute {
                    connection_id: connection.id,
                    route: bad_route,
                },
                &catalog,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            GraphCommandApplyError::Validation(
                VisualGraphValidationError::DuplicateRouteAnchorId { .. }
            )
        ));
        assert_eq!(document, original);
    }

    #[test]
    fn remove_node_command_removes_incident_connections() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.float", 1);
        let mut document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source.clone(), target.clone()],
            connections: vec![GraphConnection::new(
                test_connection_uuid(1),
                GraphPortRef::new(source.id, NodePortId::new(2)),
                GraphPortRef::new(target.id, NodePortId::new(1)),
            )],
            comments: Vec::new(),
        };

        document
            .apply_command(GraphCommand::RemoveNode { node_id: source.id }, &catalog)
            .unwrap();

        assert_eq!(document.nodes, vec![target]);
        assert!(document.connections.is_empty());
    }

    #[test]
    fn invalid_graph_command_leaves_document_unchanged() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node(), string_node()]);
        let source = GraphNode::new(test_uuid(1), "azoth.test.float", 1);
        let target = GraphNode::new(test_uuid(2), "azoth.test.string", 1);
        let mut document = VisualGraphDocument {
            document_version: 1,
            graph_type: "azoth.graph.test".to_string(),
            required_catalog_hash: None,
            nodes: vec![source.clone(), target.clone()],
            connections: Vec::new(),
            comments: Vec::new(),
        };
        let original = document.clone();

        let error = document
            .apply_command(
                GraphCommand::Connect {
                    connection: GraphConnection::new(
                        test_connection_uuid(1),
                        GraphPortRef::new(source.id, NodePortId::new(2)),
                        GraphPortRef::new(target.id, NodePortId::new(1)),
                    ),
                },
                &catalog,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            GraphCommandApplyError::Validation(
                VisualGraphValidationError::IncompatibleConnection { .. }
            )
        ));
        assert_eq!(document, original);
    }

    #[test]
    fn unknown_graph_command_targets_leave_document_unchanged() {
        let catalog = NodeTypeCatalog::new(1, 100, vec![float_node()]);
        let mut document = VisualGraphDocument::new("azoth.graph.test");
        let original = document.clone();

        let error = document
            .apply_command(
                GraphCommand::MoveNode {
                    node_id: test_uuid(99),
                    layout: GraphNodeLayout { x: 1.0, y: 2.0 },
                },
                &catalog,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            GraphCommandApplyError::UnknownNode { node_id } if node_id == test_uuid(99)
        ));
        assert_eq!(document, original);

        let error = document
            .apply_command(
                GraphCommand::Disconnect {
                    connection_id: test_connection_uuid(99),
                },
                &catalog,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            GraphCommandApplyError::UnknownConnection {
                connection_id
            } if connection_id == test_connection_uuid(99)
        ));
        assert_eq!(document, original);

        let error = document
            .apply_command(
                GraphCommand::RemoveComment {
                    comment_id: test_comment_uuid(99),
                },
                &catalog,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            GraphCommandApplyError::UnknownComment { comment_id }
                if comment_id == test_comment_uuid(99)
        ));
        assert_eq!(document, original);
    }

    #[test]
    fn visual_node_trait_registration_publishes_schema_ports_and_source_links() {
        let descriptor = RegisteredPrintNode::node_type_descriptor();

        assert_eq!(descriptor.id.as_str(), "azoth.test.registered-print");
        assert!(matches!(
            &descriptor.ports[0].value,
            NodePortValue::Data { schema_type }
                if schema_type == <String as ReflectedTypePath>::reflected_type_path()
        ));
        assert_eq!(
            descriptor.runtime_binding,
            Some(NodeRuntimeBinding::RustSymbol {
                package: env!("CARGO_PKG_NAME").to_string(),
                symbol: "RegisteredPrintNode::run".to_string(),
                call_abi: RustNodeCallAbi::ContextSchedule,
            })
        );
        let link = descriptor.source_links.first().unwrap();
        assert_eq!(link.package.as_deref(), Some(env!("CARGO_PKG_NAME")));
        assert_eq!(
            link.symbol_path.as_deref(),
            Some("RegisteredPrintNode::run")
        );
        assert!(
            link.file
                .as_deref()
                .is_some_and(|file| file.ends_with("src/lib.rs")),
            "{link:?}"
        );
        assert!(link.line.is_some_and(|line| line > 0));
        assert!(link.column.is_some_and(|column| column > 0));

        NodeTypeCatalog::try_new(1, 100, vec![descriptor]).unwrap();
    }

    #[test]
    fn composed_node_types_include_visual_node_trait_registrations() {
        let composer = compose_graphs();
        let registered = node_types(composer.registries());
        let descriptor = registered
            .iter()
            .find(|descriptor| descriptor.id.as_str() == "azoth.test.registered-print")
            .expect("visual node trait registration should be in the composed catalog");

        assert_eq!(descriptor.display_name, "Registered Print");
        assert!(descriptor.tags.iter().any(|tag| tag == "test"));

        let report = composer.finalize().expect("composition is valid");
        let entry = report
            .entries
            .iter()
            .find(|entry| entry.registry == "node-type")
            .expect("node type entry is reported");
        assert_eq!(entry.key, "azoth.test.registered-print@1");
        assert_eq!(entry.instance.gem.as_str(), "azoth.node-graph-tests");
    }

    #[test]
    fn composed_graph_types_include_visual_graph_type_registrations() {
        let composer = compose_graphs();
        let registered = graph_types(composer.registries());
        let descriptor = registered
            .iter()
            .find(|descriptor| descriptor.id.as_str() == "azoth.test.registered-logic-graph")
            .expect("visual graph type registration should be in the composed catalog");

        assert_eq!(descriptor.display_name, "Registered Logic Graph");
        assert_eq!(
            descriptor.execution_mode,
            GraphExecutionMode::RuntimeCompiled
        );
        let product = descriptor
            .runtime_product
            .as_ref()
            .expect("registered graph type has runtime product");
        assert_eq!(product.product_kind, "azoth.graph.generated-code");
        assert!(matches!(
            &product.execution_strategy,
            RuntimeGraphExecutionStrategy::AotCompiledCode {
                language,
                entry_symbol,
                ..
            } if language == "rust" && entry_symbol == "RegisteredLogicGraph::execute"
        ));
        assert!(descriptor.tags.iter().any(|tag| tag == "test"));
        GraphTypeCatalog::try_new(1, 100, vec![descriptor.clone()]).unwrap();
    }

    #[test]
    fn composed_node_types_keep_composition_order() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(TestGraphs, ProductActivation::default())
            .expect("test graphs require nothing");
        composer
            .add(TestPalette, ProductActivation::default())
            .expect("test palette requires nothing");

        let ids = node_types(composer.registries())
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                NodeTypeId::new("azoth.test.registered-print"),
                NodeTypeId::new("azoth.test.float"),
                NodeTypeId::new("azoth.test.string"),
            ],
            "iteration order is composition order, not sorted id order"
        );
    }

    #[test]
    fn a_node_type_registered_twice_fails_composition_naming_both_culprits() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(TestPalette, ProductActivation::default())
            .unwrap();
        composer
            .add(TestPalette, ProductActivation::default())
            .unwrap();

        let error = composer.finalize().unwrap_err();
        let ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = error
        else {
            panic!("expected duplicate error, got {error}");
        };
        assert_eq!(registry, "node-type");
        assert_eq!(key, "azoth.test.float@1");
        assert_eq!(first.generation, 0);
        assert_eq!(second.generation, 1);
    }

    #[test]
    fn one_node_type_at_two_versions_composes() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(TestVersions, ProductActivation::default())
            .expect("versions require nothing");

        composer
            .finalize()
            .expect("two versions of one node type are distinct keys");
        assert_eq!(node_types(composer.registries()).len(), 2);
    }

    #[test]
    fn node_graph_crate_stays_model_only() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-node-graph Cargo.toml");
        let manifest = toml::from_str::<Value>(&manifest).expect("parse az-node-graph Cargo.toml");
        let workspace_manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
                .expect("read workspace Cargo.toml");
        let workspace_manifest =
            toml::from_str::<Value>(&workspace_manifest).expect("parse workspace Cargo.toml");
        let violations = forbidden_node_graph_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-node-graph must stay an engine graph model/catalog crate; forbidden deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn node_graph_source_has_no_ui_transport_or_runtime_backend() {
        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("collect az-node-graph production sources");
        let forbidden = [
            ("use gpui", "GPUI import"),
            ("gpui::", "GPUI API"),
            ("use bevy", "Bevy import"),
            ("bevy::", "Bevy API"),
            ("use az_proto_", "protocol crate import"),
            ("capnp::", "raw Cap'n Proto API"),
            ("use az_project_host", "project-host service import"),
            ("use az_runtime_host", "runtime-host service import"),
            ("use az_editor", "editor crate import"),
            ("std::process::", "process spawning API"),
            ("std::fs::", "filesystem IO"),
        ];
        let mut violations = Vec::new();

        for source in sources {
            for (needle, reason) in forbidden {
                if source.source.contains(needle) {
                    violations.push(format!(
                        "{} contains `{needle}` ({reason})",
                        source.path.display()
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "az-node-graph production source must remain UI/transport/runtime-backend agnostic:\n{}",
            violations.join("\n")
        );
    }

    fn forbidden_node_graph_dependencies(
        manifest: &Value,
        workspace_manifest: &Value,
    ) -> Vec<String> {
        const FORBIDDEN_NODE_GRAPH_DEPS: &[&str] = &[
            "az-asset",
            "az-asset-processor",
            "az-assetdb",
            "az-daemon",
            "az-editor",
            "az-editor-inspector",
            "az-editor-ui",
            "az-engine",
            "az-project",
            "az-project-host",
            "az-proto-asset",
            "az-proto-core",
            "az-proto-daemon",
            "az-proto-observability",
            "az-proto-project",
            "az-proto-runtime",
            "az-proto-session",
            "az-rpc",
            "az-runtime-host",
            "az-session",
            "az-sessiond",
            "bevy",
            "gpui",
            "gridmate",
            "sample-plugin",
        ];
        let mut exact_names = Vec::new();
        exact_names.extend_from_slice(FORBIDDEN_NODE_GRAPH_DEPS);
        exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
        forbidden_production_dependencies(
            manifest,
            workspace_manifest,
            DependencyBoundary::new(
                &exact_names,
                PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
                PROJECT_OR_FORMAT_PATH_SEGMENTS,
            )
            // The registration contract is engine infrastructure, not a gem:
            // this crate owns two registry entry types and must name it. The
            // exemption is not a back door for the `bevy` ban above: the
            // contract's Bevy edge is its `app` feature, and this crate takes
            // it with `default-features = false`.
            .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES),
        )
    }

    fn float_node() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new("azoth.test.float", 1, "Float")
            .with_category_path(["Test".to_string(), "Math".to_string()])
            .with_port(data_input(1, FLOAT_SCHEMA))
            .with_port(data_output(2, FLOAT_SCHEMA).with_capacity(NodePortCapacity::Multiple))
    }

    fn string_node() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new("azoth.test.string", 1, "String")
            .with_port(data_input(1, STRING_SCHEMA))
            .with_port(data_output(2, STRING_SCHEMA).with_capacity(NodePortCapacity::Multiple))
    }

    fn runtime_logic_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "azoth.test.logic-graph",
            1,
            "Logic Graph",
            GraphSourceWorkflow::file("azoth.test.logic-graph.source", "azgraph.ron")
                .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::packed_ir(
                "azoth.test.logic-graph.compiler",
                "azoth.graph.logic-ir/v1",
            )
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.packed-ir",
                "azoth.graph.logic-ir",
                RuntimeGraphExecutionStrategy::PackedIr,
            ),
        )
        .with_category_path(["Test".to_string(), "Logic".to_string()])
        .with_node_catalog(
            GraphNodeCatalogRequirement::new("azoth.test.nodes")
                .with_minimum_version(1)
                .with_required_hash(vec![7; blake3::OUT_LEN]),
        )
        .with_palette_policy(
            GraphPalettePolicy::default()
                .with_root_category("Logic")
                .with_required_node_capability("azoth.node.call"),
        )
        .with_tag("test")
    }

    fn editor_debug_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::editor_interpreted(
            "azoth.test.debug-graph",
            1,
            "Debug Graph",
            GraphSourceWorkflow::project_document(
                "azoth.test.debug-graph.source",
                "azoth.graph.debug-document",
            ),
        )
        .with_category_path(["Test".to_string(), "Debug".to_string()])
        .with_tag("test")
    }

    fn data_input(id: u32, schema_type: &str) -> NodePortDescriptor {
        NodePortDescriptor::new(
            NodePortId::new(id),
            format!("in{id}"),
            NodePortDirection::Input,
            NodePortValue::Data {
                schema_type: schema_type.to_string(),
            },
        )
    }

    fn data_output(id: u32, schema_type: &str) -> NodePortDescriptor {
        NodePortDescriptor::new(
            NodePortId::new(id),
            format!("out{id}"),
            NodePortDirection::Output,
            NodePortValue::Data {
                schema_type: schema_type.to_string(),
            },
        )
    }

    fn test_uuid(low: u128) -> GraphNodeId {
        GraphNodeId::new(Uuid::from_u128(low))
    }

    fn test_connection_uuid(low: u128) -> GraphConnectionId {
        GraphConnectionId::new(Uuid::from_u128(low))
    }

    fn test_route_anchor_uuid(low: u128) -> GraphRouteAnchorId {
        GraphRouteAnchorId::new(Uuid::from_u128(low))
    }

    fn test_comment_uuid(low: u128) -> GraphCommentId {
        GraphCommentId::new(Uuid::from_u128(low))
    }

    struct RegisteredPrintNode;
    struct RegisteredLogicGraph;

    impl RegisteredPrintNode {
        fn run(_message: String) {}
    }

    impl VisualNode for RegisteredPrintNode {
        fn node_type_descriptor() -> NodeTypeDescriptor {
            let _: fn(String) = Self::run;
            NodeTypeDescriptor::new("azoth.test.registered-print", 1, "Registered Print")
                .with_category_path(["Test".to_string(), "Debug".to_string()])
                .with_description("Test visual node registration")
                .with_port(NodePortDescriptor::data_input::<String>(
                    NodePortId::new(1),
                    "message",
                ))
                .with_port(
                    NodePortDescriptor::execution_output(NodePortId::new(2), "then")
                        .with_capacity(NodePortCapacity::Multiple),
                )
                .with_capability(NodeCapability::new("azoth.node.call").with_marker("debug"))
                .with_runtime_binding(NodeRuntimeBinding::rust_symbol(
                    env!("CARGO_PKG_NAME"),
                    "RegisteredPrintNode::run",
                ))
                .with_source_link(crate::node_source_link!(RegisteredPrintNode::run))
                .with_tag("test")
        }
    }

    impl VisualGraphType for RegisteredLogicGraph {
        fn graph_type_descriptor() -> GraphTypeDescriptor {
            GraphTypeDescriptor::runtime_compiled(
                "azoth.test.registered-logic-graph",
                1,
                "Registered Logic Graph",
                GraphSourceWorkflow::file(
                    "azoth.test.registered-logic-graph.source",
                    "azgraph.ron",
                ),
                GraphCompilerBackendDescriptor::generated_rust_context_schedule(
                    "azoth.test.registered-logic-graph.compiler",
                    env!("CARGO_PKG_NAME"),
                    "RegisteredLogicGraph::compile",
                )
                .with_capability_marker("generated-code"),
                RuntimeGraphProductDescriptor::new(
                    "azoth.graph.generated-code",
                    "azoth.graph.generated-code",
                    RuntimeGraphExecutionStrategy::aot_compiled_rust(
                        env!("CARGO_PKG_NAME"),
                        "RegisteredLogicGraph::execute",
                        "RegisteredLogicGraph::RuntimeContext",
                    ),
                ),
            )
            .with_node_catalog(GraphNodeCatalogRequirement::new("azoth.test.nodes"))
            .with_tag("test")
        }
    }

    declare_caps!(TestCaps:);

    /// The trait-derived registrations, contributed the way a gem contributes
    /// them.
    struct TestGraphs;

    impl Contribution for TestGraphs {
        type Caps = TestCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.node-graph-tests"),
                contribution: ContributionId::new("graphs"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, TestCaps>) {
            ctx.registrar::<NodeTypeRegistration>()
                .register(NodeTypeRegistration::of::<RegisteredPrintNode>());
            ctx.registrar::<GraphTypeRegistration>()
                .register(GraphTypeRegistration::of::<RegisteredLogicGraph>());
        }
    }

    /// A second contribution, so composition order is observable.
    struct TestPalette;

    impl Contribution for TestPalette {
        type Caps = TestCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.node-graph-tests"),
                contribution: ContributionId::new("palette"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, TestCaps>) {
            ctx.registrar::<NodeTypeRegistration>().register_many([
                NodeTypeRegistration::new(float_node()),
                NodeTypeRegistration::new(string_node()),
            ]);
        }
    }

    /// One node type at two versions: distinct keys, both compose.
    struct TestVersions;

    impl Contribution for TestVersions {
        type Caps = TestCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.node-graph-tests"),
                contribution: ContributionId::new("versions"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, TestCaps>) {
            let mut second = float_node();
            second.version = 2;
            ctx.registrar::<NodeTypeRegistration>().register_many([
                NodeTypeRegistration::new(float_node()),
                NodeTypeRegistration::new(second),
            ]);
        }
    }

    fn compose_graphs() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(TestGraphs, ProductActivation::default())
            .expect("test graphs require nothing");
        composer
    }
}
