//! Runtime graph product format.
//!
//! This crate owns the `AZGIR` product reader used by engine/runtime code. It
//! intentionally has no dependency on `az-node-graph`, `az-graph-builder`,
//! project-host, Bevy, RON, or editor UI crates. Build-time graph compilers
//! write runtime products; runtime loaders validate/bind those products once or
//! resolve generated-code entry points without loading the authoring graph.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

use std::{any::Any, fmt, ops::Range, sync::Arc};

use az_gem_contract::{Registries, RegistryEntry, Unconditional};
use thiserror::Error;

#[cfg(feature = "bevy")]
pub mod bevy_asset;

pub const PACKED_GRAPH_IR_MAGIC: &[u8; 8] = b"AZGIR\0\0\0";
pub const PACKED_GRAPH_IR_VERSION: u32 = 1;
pub const PACKED_GRAPH_IR_PRODUCT_EXTENSION: &str = "azgir";
pub const AOT_GRAPH_MANIFEST_MAGIC: &[u8; 8] = b"AZGAOT\0\0";
pub const AOT_GRAPH_MANIFEST_VERSION: u32 = 1;
pub const AOT_GRAPH_MANIFEST_PRODUCT_EXTENSION: &str = "azgaot";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AotGraphManifest<'a> {
    pub header: AotGraphManifestHeader,
    pub graph_type: &'a str,
    pub product_kind: &'a str,
    pub language: &'a str,
    pub package: &'a str,
    pub entry_symbol: &'a str,
    pub context_type: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AotGraphManifestHeader {
    pub version: u32,
    pub flags: u32,
    pub graph_type_version: u32,
    pub node_catalog_version: u32,
    pub node_catalog_hash: [u8; 32],
    pub source_uuid: uuid::Uuid,
}

#[derive(Debug, Error)]
pub enum AotGraphManifestError {
    #[error("AOT graph manifest ended while reading {field}")]
    UnexpectedEof { field: &'static str },
    #[error("bad AOT graph manifest magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported AOT graph manifest version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("AOT graph manifest {field} is not UTF-8: {source}")]
    InvalidUtf8 {
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("AOT graph manifest has {remaining} trailing bytes")]
    TrailingBytes { remaining: usize },
}

/// # Errors
///
/// [`AotGraphManifestError`] when `bytes` is not a well-formed manifest: bad
/// magic, an unsupported version, a truncated or malformed record, or trailing
/// bytes after the last record.
pub fn decode_aot_graph_manifest(
    bytes: &[u8],
) -> Result<AotGraphManifest<'_>, AotGraphManifestError> {
    let mut reader = AotGraphManifestReader::new(bytes);
    let magic = reader.read_array::<8>("magic")?;
    if &magic != AOT_GRAPH_MANIFEST_MAGIC {
        return Err(AotGraphManifestError::BadMagic { found: magic });
    }

    let header = AotGraphManifestHeader {
        version: reader.read_u32("version")?,
        flags: reader.read_u32("flags")?,
        graph_type_version: reader.read_u32("graph type version")?,
        node_catalog_version: reader.read_u32("node catalog version")?,
        node_catalog_hash: reader.read_array::<32>("node catalog hash")?,
        source_uuid: uuid::Uuid::from_bytes(reader.read_array::<16>("source uuid")?),
    };
    if header.version != AOT_GRAPH_MANIFEST_VERSION {
        return Err(AotGraphManifestError::UnsupportedVersion {
            version: header.version,
        });
    }

    let manifest = AotGraphManifest {
        header,
        graph_type: reader.read_text("graph type")?,
        product_kind: reader.read_text("product kind")?,
        language: reader.read_text("language")?,
        package: reader.read_text("package")?,
        entry_symbol: reader.read_text("entry symbol")?,
        context_type: reader.read_text("context type")?,
    };

    if reader.remaining() != 0 {
        return Err(AotGraphManifestError::TrailingBytes {
            remaining: reader.remaining(),
        });
    }

    Ok(manifest)
}

pub type AotGraphEntryFn =
    for<'a> fn(AotGraphExecuteContext<'a>) -> Result<(), AotGraphExecutionError>;

#[derive(Debug)]
pub struct AotGraphExecuteContext<'a> {
    pub graph_instance: uuid::Uuid,
    pub runtime_context: &'a mut dyn Any,
}

impl AotGraphExecuteContext<'_> {
    /// # Errors
    ///
    /// [`AotGraphExecutionError::ContextType`] when the caller's runtime
    /// context is not a `T`.
    pub fn runtime_context_mut<T: 'static>(&mut self) -> Result<&mut T, AotGraphExecutionError> {
        self.runtime_context.downcast_mut::<T>().ok_or_else(|| {
            AotGraphExecutionError::ContextType {
                expected: std::any::type_name::<T>(),
            }
        })
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AotGraphExecutionError {
    #[error("{reason}")]
    Failed { reason: &'static str },
    #[error("runtime context type mismatch: expected {expected}")]
    ContextType { expected: &'static str },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AotGraphLookupError {
    #[error("AOT graph runtime entry `{package}` / `{entry_symbol}` is not registered")]
    MissingEntry {
        package: String,
        entry_symbol: String,
    },
}

/// Compose key of one AOT graph: the pair a runtime resolves on.
///
/// The AOT manifest a product carries names `package` and `entry_symbol`, and
/// a host resolves its entry by exactly that pair, so two registrations under
/// one pair would each answer the other's lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AotGraphKey {
    pub package: &'static str,
    pub entry_symbol: &'static str,
}

impl fmt::Display for AotGraphKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}::{}", self.package, self.entry_symbol)
    }
}

/// One compiled-to-Rust graph contributed to a host's AOT graph registry.
///
/// Written by the graph compiler's AOT emitters as a `const` beside the entry
/// function it names, and registered by the generated glue module that
/// `include!`s them — a gem author never writes one by hand.
#[derive(Debug, Clone, Copy)]
pub struct AotGraphRuntimeRegistration {
    package: &'static str,
    entry_symbol: &'static str,
    graph_type: &'static str,
    context_type: &'static str,
    entry: AotGraphEntryFn,
}

impl RegistryEntry for AotGraphRuntimeRegistration {
    type Key = AotGraphKey;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "aot-graph"
    }

    fn key(&self) -> AotGraphKey {
        AotGraphKey {
            package: self.package,
            entry_symbol: self.entry_symbol,
        }
    }
}

impl AotGraphRuntimeRegistration {
    #[must_use]
    pub const fn new(
        package: &'static str,
        entry_symbol: &'static str,
        graph_type: &'static str,
        context_type: &'static str,
        entry: AotGraphEntryFn,
    ) -> Self {
        Self {
            package,
            entry_symbol,
            graph_type,
            context_type,
            entry,
        }
    }

    #[must_use]
    pub const fn package(&self) -> &'static str {
        self.package
    }

    #[must_use]
    pub const fn entry_symbol(&self) -> &'static str {
        self.entry_symbol
    }

    #[must_use]
    pub const fn graph_type(&self) -> &'static str {
        self.graph_type
    }

    #[must_use]
    pub const fn context_type(&self) -> &'static str {
        self.context_type
    }

    #[must_use]
    pub fn matches(&self, package: &str, entry_symbol: &str) -> bool {
        self.package == package && self.entry_symbol == entry_symbol
    }

    /// # Errors
    ///
    /// Whatever the registered entry point reports, including
    /// [`AotGraphExecutionError::ContextType`] when `runtime_context` is not
    /// the type the entry expects.
    pub fn execute(
        &self,
        graph_instance: uuid::Uuid,
        runtime_context: &mut dyn Any,
    ) -> Result<(), AotGraphExecutionError> {
        (self.entry)(AotGraphExecuteContext {
            graph_instance,
            runtime_context,
        })
    }
}

/// AOT graphs this host composed, in composition order.
///
/// The registry has no precedence of its own: a lookup is keyed, not ordered,
/// so composition order is the only order and nothing re-sorts it.
#[must_use]
pub fn aot_graphs(registries: &Registries) -> Vec<&AotGraphRuntimeRegistration> {
    registries
        .get::<AotGraphRuntimeRegistration>()
        .map(|registry| registry.entries().collect())
        .unwrap_or_default()
}

/// # Errors
///
/// [`AotGraphLookupError::MissingEntry`] when this host composed no AOT graph
/// under `package` and `entry_symbol`.
pub fn resolve_aot_graph<'a>(
    registries: &'a Registries,
    package: &str,
    entry_symbol: &str,
) -> Result<&'a AotGraphRuntimeRegistration, AotGraphLookupError> {
    aot_graphs(registries)
        .into_iter()
        .find(|registration| registration.matches(package, entry_symbol))
        .ok_or_else(|| AotGraphLookupError::MissingEntry {
            package: package.to_string(),
            entry_symbol: entry_symbol.to_string(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphIr<'a> {
    pub header: PackedGraphIrHeader,
    pub graph_type: &'a str,
    pub product_kind: &'a str,
    pub backend: &'a str,
    pub ir_schema: &'a str,
    pub capability_markers: Vec<&'a str>,
    pub strings: Vec<&'a str>,
    pub nodes: Vec<PackedGraphNode>,
    pub ports: Vec<PackedGraphPort>,
    pub input_values: Vec<PackedGraphInputValue<'a>>,
    pub connections: Vec<PackedGraphConnection>,
    pub execution_order: Vec<u32>,
}

impl<'a> PackedGraphIr<'a> {
    #[must_use]
    pub fn string(&self, index: u32) -> Option<&'a str> {
        self.strings.get(index as usize).copied()
    }

    #[must_use]
    pub const fn flags(&self) -> PackedGraphRuntimeFlags {
        PackedGraphRuntimeFlags::from_bits(self.header.flags)
    }

    #[must_use]
    pub fn node_ports(&self, node: &PackedGraphNode) -> &[PackedGraphPort] {
        &self.ports[node.port_range.clone()]
    }

    #[must_use]
    pub fn node_input_values(&self, node: &PackedGraphNode) -> &[PackedGraphInputValue<'a>] {
        &self.input_values[node.input_value_range.clone()]
    }

    /// # Panics
    ///
    /// If `node` did not come from this IR; every node's `node_type` string
    /// index is range-checked by [`decode_packed_graph_ir`].
    #[must_use]
    pub fn node_type(&self, node: &PackedGraphNode) -> &'a str {
        self.string(node.node_type)
            .expect("packed graph decoder validates node type string indexes")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphProduct {
    bytes: Arc<[u8]>,
    pub header: PackedGraphIrHeader,
    graph_type: Range<usize>,
    product_kind: Range<usize>,
    backend: Range<usize>,
    ir_schema: Range<usize>,
    capability_markers: Vec<Range<usize>>,
    strings: Vec<Range<usize>>,
    pub nodes: Vec<PackedGraphNode>,
    pub ports: Vec<PackedGraphPort>,
    pub input_values: Vec<PackedGraphInputValueRange>,
    pub connections: Vec<PackedGraphConnection>,
    pub execution_order: Vec<u32>,
}

impl PackedGraphProduct {
    /// # Errors
    ///
    /// [`PackedGraphIrError`] when `bytes` is not a well-formed packed graph:
    /// bad magic, an unsupported version, a truncated or malformed section, an
    /// out-of-range index, or trailing bytes.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<Self, PackedGraphIrError> {
        let bytes = bytes.into();
        let (
            header,
            graph_type,
            product_kind,
            backend,
            ir_schema,
            capability_markers,
            strings,
            nodes,
            ports,
            input_values,
            connections,
            execution_order,
        ) = {
            let decoded = decode_packed_graph_ir(&bytes)?;
            let graph_type = range_for_str(&bytes, decoded.graph_type);
            let product_kind = range_for_str(&bytes, decoded.product_kind);
            let backend = range_for_str(&bytes, decoded.backend);
            let ir_schema = range_for_str(&bytes, decoded.ir_schema);
            let capability_markers = decoded
                .capability_markers
                .iter()
                .map(|value| range_for_str(&bytes, value))
                .collect();
            let strings = decoded
                .strings
                .iter()
                .map(|value| range_for_str(&bytes, value))
                .collect();
            let input_values = decoded
                .input_values
                .iter()
                .map(|value| PackedGraphInputValueRange {
                    port_index: value.port_index,
                    bytes: range_for_slice(&bytes, value.bytes),
                })
                .collect();
            (
                decoded.header,
                graph_type,
                product_kind,
                backend,
                ir_schema,
                capability_markers,
                strings,
                decoded.nodes,
                decoded.ports,
                input_values,
                decoded.connections,
                decoded.execution_order,
            )
        };

        Ok(Self {
            bytes,
            header,
            graph_type,
            product_kind,
            backend,
            ir_schema,
            capability_markers,
            strings,
            nodes,
            ports,
            input_values,
            connections,
            execution_order,
        })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn graph_type(&self) -> &str {
        self.str_from_range(&self.graph_type)
    }

    #[must_use]
    pub fn product_kind(&self) -> &str {
        self.str_from_range(&self.product_kind)
    }

    #[must_use]
    pub fn backend(&self) -> &str {
        self.str_from_range(&self.backend)
    }

    #[must_use]
    pub fn ir_schema(&self) -> &str {
        self.str_from_range(&self.ir_schema)
    }

    #[must_use]
    pub fn capability_markers(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.capability_markers
            .iter()
            .map(|range| self.str_from_range(range))
    }

    #[must_use]
    pub fn string(&self, index: u32) -> Option<&str> {
        self.strings
            .get(index as usize)
            .map(|range| self.str_from_range(range))
    }

    /// # Errors
    ///
    /// [`PackedGraphBindLookupError::StringIndexOutOfRange`] when `index` is
    /// past the end of this product's string table.
    pub fn try_string(
        &self,
        index: u32,
        field: &'static str,
    ) -> Result<&str, PackedGraphBindLookupError> {
        self.string(index)
            .ok_or(PackedGraphBindLookupError::StringIndexOutOfRange {
                field,
                index,
                strings: self.strings.len(),
            })
    }

    /// # Panics
    ///
    /// If `node` did not come from this product; every node's `node_type`
    /// string index is range-checked by [`Self::from_bytes`].
    #[must_use]
    pub fn node_type(&self, node: &PackedGraphNode) -> &str {
        self.string(node.node_type)
            .expect("packed graph product validates node type string indexes")
    }

    #[must_use]
    pub fn node_ports(&self, node: &PackedGraphNode) -> &[PackedGraphPort] {
        &self.ports[node.port_range.clone()]
    }

    #[must_use]
    pub fn node_input_values(&self, node: &PackedGraphNode) -> &[PackedGraphInputValueRange] {
        &self.input_values[node.input_value_range.clone()]
    }

    #[must_use]
    pub fn input_value_bytes(&self, value: &PackedGraphInputValueRange) -> &[u8] {
        &self.bytes[value.bytes.clone()]
    }

    #[must_use]
    pub const fn flags(&self) -> PackedGraphRuntimeFlags {
        PackedGraphRuntimeFlags::from_bits(self.header.flags)
    }

    /// # Errors
    ///
    /// [`PackedGraphBindLookupError::StringIndexOutOfRange`] when `binding`
    /// names a string outside this product's table.
    pub fn runtime_binding_ref(
        &self,
        binding: &PackedRuntimeBinding,
    ) -> Result<PackedRuntimeBindingRef<'_>, PackedGraphBindLookupError> {
        match binding {
            PackedRuntimeBinding::RustSymbol { package, symbol } => {
                Ok(PackedRuntimeBindingRef::RustSymbol {
                    package: self.try_string(*package, "runtime binding package")?,
                    symbol: self.try_string(*symbol, "runtime binding symbol")?,
                })
            }
            PackedRuntimeBinding::AssetBuilder { builder_id } => {
                Ok(PackedRuntimeBindingRef::AssetBuilder {
                    builder_id: self.try_string(*builder_id, "runtime binding builder id")?,
                })
            }
            PackedRuntimeBinding::RuntimeComponent { component_type } => {
                Ok(PackedRuntimeBindingRef::RuntimeComponent {
                    component_type: self
                        .try_string(*component_type, "runtime binding component type")?,
                })
            }
            PackedRuntimeBinding::External { kind, locator } => {
                Ok(PackedRuntimeBindingRef::External {
                    kind: self.try_string(*kind, "runtime binding external kind")?,
                    locator: self.try_string(*locator, "runtime binding external locator")?,
                })
            }
        }
    }

    fn str_from_range(&self, range: &Range<usize>) -> &str {
        std::str::from_utf8(&self.bytes[range.clone()])
            .expect("packed graph product validates string ranges as UTF-8")
    }

    /// # Errors
    ///
    /// [`PackedGraphBindError::Node`] wrapping whatever `binder` reports for a
    /// node it cannot bind, or [`PackedGraphBindError::Lookup`] when a node
    /// references a string outside this product's table.
    pub fn bind<R>(
        &self,
        binder: &R,
    ) -> Result<PackedGraphExecutionPlan<R::Node>, PackedGraphBindError<R::Error>>
    where
        R: PackedGraphNodeBinder,
    {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for (node_index, node) in self.nodes.iter().enumerate() {
            validate_node_runtime_ranges(self, node_index, node)?;
            let node_type = self
                .try_string(node.node_type, "node type")
                .map_err(|source| PackedGraphBindError::BindingLookup {
                    node_index,
                    node_id: node.id,
                    source,
                })?;
            let runtime_binding_ref =
                self.runtime_binding_ref(&node.runtime_binding)
                    .map_err(|source| PackedGraphBindError::BindingLookup {
                        node_index,
                        node_id: node.id,
                        source,
                    })?;
            let prepared_node = binder
                .bind_node(PackedGraphNodeBindRequest {
                    product: self,
                    node_index,
                    node,
                    node_type,
                    runtime_binding: &node.runtime_binding,
                    runtime_binding_ref,
                    ports: self.node_ports(node),
                    input_values: PackedGraphNodeInputValues {
                        product: self,
                        values: self.node_input_values(node),
                    },
                })
                .map_err(|binder_error| PackedGraphBindError::BindNode {
                    node_index,
                    node_id: node.id,
                    node_type: node_type.to_string(),
                    binder_error,
                })?;
            nodes.push(PackedGraphBoundNode {
                node_index,
                node_id: node.id,
                prepared_node,
                port_range: node.port_range.clone(),
                input_value_range: node.input_value_range.clone(),
            });
        }

        let mut execution_order = Vec::with_capacity(self.execution_order.len());
        for node_index in &self.execution_order {
            let node_index = usize::try_from(*node_index).map_err(|_| {
                PackedGraphBindError::NodeIndexOutOfRange {
                    field: "execution order",
                    node_index: usize::MAX,
                    nodes: self.nodes.len(),
                }
            })?;
            if node_index >= self.nodes.len() {
                return Err(PackedGraphBindError::NodeIndexOutOfRange {
                    field: "execution order",
                    node_index,
                    nodes: self.nodes.len(),
                });
            }
            execution_order.push(node_index);
        }

        Ok(PackedGraphExecutionPlan {
            nodes: nodes.into_boxed_slice(),
            execution_order: execution_order.into_boxed_slice(),
        })
    }
}

pub trait PackedGraphNodeBinder {
    type Node;
    type Error;

    /// # Errors
    ///
    /// Implementation-defined: whatever the binder reports for a node it does
    /// not recognize or cannot construct.
    fn bind_node(&self, request: PackedGraphNodeBindRequest<'_>)
    -> Result<Self::Node, Self::Error>;
}

#[derive(Debug, Clone, Copy)]
pub struct PackedGraphNodeBindRequest<'a> {
    pub product: &'a PackedGraphProduct,
    pub node_index: usize,
    pub node: &'a PackedGraphNode,
    pub node_type: &'a str,
    pub runtime_binding: &'a PackedRuntimeBinding,
    pub runtime_binding_ref: PackedRuntimeBindingRef<'a>,
    pub ports: &'a [PackedGraphPort],
    pub input_values: PackedGraphNodeInputValues<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackedRuntimeBindingRef<'a> {
    RustSymbol { package: &'a str, symbol: &'a str },
    AssetBuilder { builder_id: &'a str },
    RuntimeComponent { component_type: &'a str },
    External { kind: &'a str, locator: &'a str },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PackedGraphBindLookupError {
    #[error("string index for {field} is {index}, but string count is {strings}")]
    StringIndexOutOfRange {
        field: &'static str,
        index: u32,
        strings: usize,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PackedGraphNodeInputValues<'a> {
    product: &'a PackedGraphProduct,
    values: &'a [PackedGraphInputValueRange],
}

impl<'a> PackedGraphNodeInputValues<'a> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub const fn ranges(&self) -> &'a [PackedGraphInputValueRange] {
        self.values
    }

    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = PackedGraphInputValueRef<'a>> + '_ {
        self.values.iter().map(|value| PackedGraphInputValueRef {
            port_index: value.port_index,
            bytes: self.product.input_value_bytes(value),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedGraphInputValueRef<'a> {
    pub port_index: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphExecutionPlan<N> {
    nodes: Box<[PackedGraphBoundNode<N>]>,
    execution_order: Box<[usize]>,
}

impl<N> PackedGraphExecutionPlan<N> {
    #[must_use]
    pub fn nodes(&self) -> &[PackedGraphBoundNode<N>] {
        &self.nodes
    }

    #[must_use]
    pub fn execution_order(&self) -> &[usize] {
        &self.execution_order
    }

    #[must_use]
    pub fn execution_nodes(&self) -> impl ExactSizeIterator<Item = &PackedGraphBoundNode<N>> + '_ {
        self.execution_order
            .iter()
            .map(|node_index| &self.nodes[*node_index])
    }

    /// # Errors
    ///
    /// The first error `executor` reports; execution stops at that node.
    pub fn execute<E>(&self, executor: &mut E, context: &mut E::Context) -> Result<(), E::Error>
    where
        E: PackedGraphExecutor<Node = N>,
    {
        for node in self.execution_nodes() {
            executor.execute_node(node, context)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphBoundNode<N> {
    pub node_index: usize,
    pub node_id: uuid::Uuid,
    pub prepared_node: N,
    pub port_range: Range<usize>,
    pub input_value_range: Range<usize>,
}

pub trait PackedGraphExecutor {
    type Node;
    type Context;
    type Error;

    /// # Errors
    ///
    /// Implementation-defined: whatever the executor reports for a node it
    /// cannot run. The plan stops at the first error.
    fn execute_node(
        &mut self,
        node: &PackedGraphBoundNode<Self::Node>,
        context: &mut Self::Context,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Error)]
pub enum PackedGraphBindError<E> {
    #[error("runtime node bind failed for node #{node_index} `{node_type}` ({node_id})")]
    BindNode {
        node_index: usize,
        node_id: uuid::Uuid,
        node_type: String,
        binder_error: E,
    },
    #[error("runtime binding lookup failed for node #{node_index} ({node_id}): {source}")]
    BindingLookup {
        node_index: usize,
        node_id: uuid::Uuid,
        source: PackedGraphBindLookupError,
    },
    #[error("{field} references node #{node_index}, but product has {nodes} nodes")]
    NodeIndexOutOfRange {
        field: &'static str,
        node_index: usize,
        nodes: usize,
    },
    #[error(
        "node #{node_index} has invalid {field} range {start}..{end}, but product has {available} {field} entries"
    )]
    RangeOutOfBounds {
        node_index: usize,
        field: &'static str,
        start: usize,
        end: usize,
        available: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphIrHeader {
    pub version: u32,
    pub flags: u32,
    pub graph_type_version: u32,
    pub node_catalog_version: u32,
    pub node_catalog_hash: [u8; 32],
    pub source_uuid: uuid::Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedGraphRuntimeFlags {
    pub streamable: bool,
    pub diffable_chunks: bool,
}

impl PackedGraphRuntimeFlags {
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            streamable: bits & 0x1 != 0,
            diffable_chunks: bits & 0x2 != 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphNode {
    pub id: uuid::Uuid,
    pub node_type: u32,
    pub node_type_version: u32,
    pub runtime_binding: PackedRuntimeBinding,
    pub port_range: Range<usize>,
    pub input_value_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackedRuntimeBinding {
    RustSymbol { package: u32, symbol: u32 },
    AssetBuilder { builder_id: u32 },
    RuntimeComponent { component_type: u32 },
    External { kind: u32, locator: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphPort {
    pub id: u32,
    pub name: u32,
    pub direction: PackedGraphPortDirection,
    pub capacity: PackedGraphPortCapacity,
    pub value: PackedGraphPortValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackedGraphPortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackedGraphPortCapacity {
    Single,
    Multiple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackedGraphPortValue {
    Execution,
    Data {
        schema_type: u32,
    },
    DynamicData {
        group: u32,
        accepted_schema_types: Vec<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphInputValue<'a> {
    pub port_index: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphInputValueRange {
    pub port_index: u32,
    pub bytes: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedGraphConnection {
    pub id: uuid::Uuid,
    pub from_node_index: u32,
    pub from_port_index: u32,
    pub to_node_index: u32,
    pub to_port_index: u32,
}

#[derive(Debug, Error)]
pub enum PackedGraphIrError {
    #[error("packed graph IR ended while reading {field}")]
    UnexpectedEof { field: &'static str },
    #[error("bad packed graph IR magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported packed graph IR version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("packed graph IR string #{string_index} is not UTF-8: {source}")]
    InvalidUtf8 {
        string_index: usize,
        source: std::str::Utf8Error,
    },
    #[error("packed graph IR string index for {field} is {index}, but string count is {strings}")]
    StringIndexOutOfRange {
        field: &'static str,
        index: u32,
        strings: usize,
    },
    #[error("packed graph IR node index for {field} is {index}, but node count is {nodes}")]
    NodeIndexOutOfRange {
        field: &'static str,
        index: u32,
        nodes: usize,
    },
    #[error(
        "packed graph IR port index for {field} is {index}, but node #{node_index} has {ports} ports"
    )]
    PortIndexOutOfRange {
        field: &'static str,
        node_index: u32,
        index: u32,
        ports: usize,
    },
    #[error("packed graph IR {field} tag {tag} is invalid")]
    InvalidTag { field: &'static str, tag: u8 },
    #[error("packed graph IR {field} count mismatch: header says {expected}, decoded {actual}")]
    CountMismatch {
        field: &'static str,
        expected: u32,
        actual: usize,
    },
    #[error("packed graph IR has {remaining} trailing bytes")]
    TrailingBytes { remaining: usize },
}

/// # Errors
///
/// [`PackedGraphIrError`] when `bytes` is not a well-formed packed graph IR:
/// bad magic, an unsupported version, a truncated or malformed section, a
/// non-UTF-8 string, an out-of-range index, a count mismatch, or trailing bytes.
pub fn decode_packed_graph_ir(bytes: &[u8]) -> Result<PackedGraphIr<'_>, PackedGraphIrError> {
    let mut reader = PackedGraphReader::new(bytes);
    let header = read_packed_graph_header(&mut reader)?;

    let graph_type_index = reader.read_u32("graph type string index")?;
    let product_kind_index = reader.read_u32("product kind string index")?;
    let backend_index = reader.read_u32("backend string index")?;
    let ir_schema_index = reader.read_u32("IR schema string index")?;
    let string_count = reader.read_u32("string count")?;
    let marker_count = reader.read_u32("backend marker count")?;
    let node_count = reader.read_u32("node count")?;
    let expected_port_count = reader.read_u32("port count")?;
    let expected_input_value_count = reader.read_u32("input value count")?;
    let connection_count = reader.read_u32("connection count")?;
    let execution_order_count = reader.read_u32("execution order count")?;

    if execution_order_count != node_count {
        return Err(PackedGraphIrError::CountMismatch {
            field: "execution order",
            expected: node_count,
            actual: execution_order_count as usize,
        });
    }

    let strings = read_packed_graph_strings(&mut reader, string_count)?;

    let graph_type = require_string(&strings, graph_type_index, "graph type")?;
    let product_kind = require_string(&strings, product_kind_index, "product kind")?;
    let backend = require_string(&strings, backend_index, "backend")?;
    let ir_schema = require_string(&strings, ir_schema_index, "IR schema")?;

    let mut capability_markers = Vec::with_capacity(marker_count as usize);
    for _ in 0..marker_count {
        let index = reader.read_u32("backend marker string index")?;
        capability_markers.push(require_string(&strings, index, "backend marker")?);
    }

    let mut nodes = Vec::with_capacity(node_count as usize);
    let mut ports = Vec::with_capacity(expected_port_count as usize);
    let mut input_values = Vec::with_capacity(expected_input_value_count as usize);
    for _ in 0..node_count {
        let node = read_packed_node(
            &mut reader,
            &strings,
            &mut ports,
            &mut input_values,
            nodes.len(),
        )?;
        nodes.push(node);
    }
    assert_count("ports", expected_port_count, ports.len())?;
    assert_count(
        "input values",
        expected_input_value_count,
        input_values.len(),
    )?;

    let mut connections = Vec::with_capacity(connection_count as usize);
    for _ in 0..connection_count {
        connections.push(read_packed_connection(&mut reader, &nodes)?);
    }

    let mut execution_order = Vec::with_capacity(execution_order_count as usize);
    for _ in 0..execution_order_count {
        let index = reader.read_u32("execution node index")?;
        require_node(&nodes, index, "execution order")?;
        execution_order.push(index);
    }

    if reader.remaining() != 0 {
        return Err(PackedGraphIrError::TrailingBytes {
            remaining: reader.remaining(),
        });
    }

    Ok(PackedGraphIr {
        header,
        graph_type,
        product_kind,
        backend,
        ir_schema,
        capability_markers,
        strings,
        nodes,
        ports,
        input_values,
        connections,
        execution_order,
    })
}

fn read_packed_graph_header(
    reader: &mut PackedGraphReader<'_>,
) -> Result<PackedGraphIrHeader, PackedGraphIrError> {
    let magic = reader.read_array::<8>("magic")?;
    if &magic != PACKED_GRAPH_IR_MAGIC {
        return Err(PackedGraphIrError::BadMagic { found: magic });
    }

    let header = PackedGraphIrHeader {
        version: reader.read_u32("version")?,
        flags: reader.read_u32("flags")?,
        graph_type_version: reader.read_u32("graph type version")?,
        node_catalog_version: reader.read_u32("node catalog version")?,
        node_catalog_hash: reader.read_array::<32>("node catalog hash")?,
        source_uuid: uuid::Uuid::from_bytes(reader.read_array::<16>("source uuid")?),
    };
    if header.version == PACKED_GRAPH_IR_VERSION {
        Ok(header)
    } else {
        Err(PackedGraphIrError::UnsupportedVersion {
            version: header.version,
        })
    }
}

fn read_packed_graph_strings<'a>(
    reader: &mut PackedGraphReader<'a>,
    string_count: u32,
) -> Result<Vec<&'a str>, PackedGraphIrError> {
    let mut strings = Vec::with_capacity(string_count as usize);
    for string_index in 0..string_count as usize {
        let length = reader.read_u32("string byte count")? as usize;
        let bytes = reader.read_bytes("string bytes", length)?;
        let value =
            std::str::from_utf8(bytes).map_err(|source| PackedGraphIrError::InvalidUtf8 {
                string_index,
                source,
            })?;
        strings.push(value);
    }
    Ok(strings)
}

struct PackedGraphReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PackedGraphReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn read_bytes(
        &mut self,
        field: &'static str,
        length: usize,
    ) -> Result<&'a [u8], PackedGraphIrError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(PackedGraphIrError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(PackedGraphIrError::UnexpectedEof { field })?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], PackedGraphIrError> {
        self.read_bytes(field, N).map(|bytes| {
            bytes
                .try_into()
                .expect("read_bytes returned exactly the requested array length")
        })
    }

    fn read_u8(&mut self, field: &'static str) -> Result<u8, PackedGraphIrError> {
        Ok(self.read_array::<1>(field)?[0])
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, PackedGraphIrError> {
        Ok(u32::from_le_bytes(self.read_array::<4>(field)?))
    }
}

fn read_packed_node<'a>(
    reader: &mut PackedGraphReader<'a>,
    strings: &[&'a str],
    ports: &mut Vec<PackedGraphPort>,
    input_values: &mut Vec<PackedGraphInputValue<'a>>,
    node_index: usize,
) -> Result<PackedGraphNode, PackedGraphIrError> {
    let id = uuid::Uuid::from_bytes(reader.read_array::<16>("node id")?);
    let node_type = reader.read_u32("node type string index")?;
    require_string(strings, node_type, "node type")?;
    let node_type_version = reader.read_u32("node type version")?;
    let runtime_binding = read_packed_runtime_binding(reader, strings)?;

    let port_start = ports.len();
    let port_count = reader.read_u32("node port count")? as usize;
    for _ in 0..port_count {
        ports.push(read_packed_port(reader, strings)?);
    }
    let port_end = ports.len();

    let input_value_start = input_values.len();
    let input_value_count = reader.read_u32("node input value count")? as usize;
    for _ in 0..input_value_count {
        let port_index = reader.read_u32("input value port index")?;
        if port_index as usize >= port_count {
            return Err(PackedGraphIrError::PortIndexOutOfRange {
                field: "input value",
                // The caller counts nodes off a `u32` node count, so this
                // always fits; saturating keeps a degraded index in the report
                // rather than wrapping it.
                node_index: u32::try_from(node_index).unwrap_or(u32::MAX),
                index: port_index,
                ports: port_count,
            });
        }
        let length = reader.read_u32("input value byte count")? as usize;
        let bytes = reader.read_bytes("input value bytes", length)?;
        input_values.push(PackedGraphInputValue { port_index, bytes });
    }
    let input_value_end = input_values.len();

    Ok(PackedGraphNode {
        id,
        node_type,
        node_type_version,
        runtime_binding,
        port_range: port_start..port_end,
        input_value_range: input_value_start..input_value_end,
    })
}

fn read_packed_runtime_binding(
    reader: &mut PackedGraphReader<'_>,
    strings: &[&str],
) -> Result<PackedRuntimeBinding, PackedGraphIrError> {
    let tag = reader.read_u8("runtime binding tag")?;
    match tag {
        1 => {
            let package = reader.read_u32("runtime binding package")?;
            let symbol = reader.read_u32("runtime binding symbol")?;
            require_string(strings, package, "runtime binding package")?;
            require_string(strings, symbol, "runtime binding symbol")?;
            Ok(PackedRuntimeBinding::RustSymbol { package, symbol })
        }
        2 => {
            let builder_id = reader.read_u32("runtime binding builder id")?;
            require_string(strings, builder_id, "runtime binding builder id")?;
            Ok(PackedRuntimeBinding::AssetBuilder { builder_id })
        }
        3 => {
            let component_type = reader.read_u32("runtime binding component type")?;
            require_string(strings, component_type, "runtime binding component type")?;
            Ok(PackedRuntimeBinding::RuntimeComponent { component_type })
        }
        4 => {
            let kind = reader.read_u32("runtime binding external kind")?;
            let locator = reader.read_u32("runtime binding external locator")?;
            require_string(strings, kind, "runtime binding external kind")?;
            require_string(strings, locator, "runtime binding external locator")?;
            Ok(PackedRuntimeBinding::External { kind, locator })
        }
        tag => Err(PackedGraphIrError::InvalidTag {
            field: "runtime binding",
            tag,
        }),
    }
}

fn read_packed_port(
    reader: &mut PackedGraphReader<'_>,
    strings: &[&str],
) -> Result<PackedGraphPort, PackedGraphIrError> {
    let id = reader.read_u32("port id")?;
    let name = reader.read_u32("port name string index")?;
    require_string(strings, name, "port name")?;
    let direction = match reader.read_u8("port direction")? {
        0 => PackedGraphPortDirection::Input,
        1 => PackedGraphPortDirection::Output,
        tag => {
            return Err(PackedGraphIrError::InvalidTag {
                field: "port direction",
                tag,
            });
        }
    };
    let capacity = match reader.read_u8("port capacity")? {
        0 => PackedGraphPortCapacity::Single,
        1 => PackedGraphPortCapacity::Multiple,
        tag => {
            return Err(PackedGraphIrError::InvalidTag {
                field: "port capacity",
                tag,
            });
        }
    };
    let value = match reader.read_u8("port value tag")? {
        0 => PackedGraphPortValue::Execution,
        1 => {
            let schema_type = reader.read_u32("port schema type")?;
            require_string(strings, schema_type, "port schema type")?;
            PackedGraphPortValue::Data { schema_type }
        }
        2 => {
            let group = reader.read_u32("dynamic port group")?;
            require_string(strings, group, "dynamic port group")?;
            let accepted_count = reader.read_u32("dynamic port accepted schema count")?;
            let mut accepted_schema_types = Vec::with_capacity(accepted_count as usize);
            for _ in 0..accepted_count {
                let schema_type = reader.read_u32("dynamic port accepted schema type")?;
                require_string(strings, schema_type, "dynamic port accepted schema type")?;
                accepted_schema_types.push(schema_type);
            }
            PackedGraphPortValue::DynamicData {
                group,
                accepted_schema_types,
            }
        }
        tag => {
            return Err(PackedGraphIrError::InvalidTag {
                field: "port value",
                tag,
            });
        }
    };

    Ok(PackedGraphPort {
        id,
        name,
        direction,
        capacity,
        value,
    })
}

fn read_packed_connection(
    reader: &mut PackedGraphReader<'_>,
    nodes: &[PackedGraphNode],
) -> Result<PackedGraphConnection, PackedGraphIrError> {
    let id = uuid::Uuid::from_bytes(reader.read_array::<16>("connection id")?);
    let from_node_index = reader.read_u32("connection source node index")?;
    let from_port_index = reader.read_u32("connection source port index")?;
    let to_node_index = reader.read_u32("connection target node index")?;
    let to_port_index = reader.read_u32("connection target port index")?;
    require_port(nodes, from_node_index, from_port_index, "connection source")?;
    require_port(nodes, to_node_index, to_port_index, "connection target")?;

    Ok(PackedGraphConnection {
        id,
        from_node_index,
        from_port_index,
        to_node_index,
        to_port_index,
    })
}

fn require_string<'a>(
    strings: &[&'a str],
    index: u32,
    field: &'static str,
) -> Result<&'a str, PackedGraphIrError> {
    strings
        .get(index as usize)
        .copied()
        .ok_or(PackedGraphIrError::StringIndexOutOfRange {
            field,
            index,
            strings: strings.len(),
        })
}

fn require_node<'a>(
    nodes: &'a [PackedGraphNode],
    index: u32,
    field: &'static str,
) -> Result<&'a PackedGraphNode, PackedGraphIrError> {
    nodes
        .get(index as usize)
        .ok_or(PackedGraphIrError::NodeIndexOutOfRange {
            field,
            index,
            nodes: nodes.len(),
        })
}

fn require_port(
    nodes: &[PackedGraphNode],
    node_index: u32,
    port_index: u32,
    field: &'static str,
) -> Result<(), PackedGraphIrError> {
    let node = require_node(nodes, node_index, field)?;
    let ports = node.port_range.len();
    if port_index as usize >= ports {
        return Err(PackedGraphIrError::PortIndexOutOfRange {
            field,
            node_index,
            index: port_index,
            ports,
        });
    }
    Ok(())
}

const fn assert_count(
    field: &'static str,
    expected: u32,
    actual: usize,
) -> Result<(), PackedGraphIrError> {
    if expected as usize == actual {
        return Ok(());
    }
    Err(PackedGraphIrError::CountMismatch {
        field,
        expected,
        actual,
    })
}

fn validate_node_runtime_ranges<E>(
    product: &PackedGraphProduct,
    node_index: usize,
    node: &PackedGraphNode,
) -> Result<(), PackedGraphBindError<E>> {
    validate_range(node_index, "ports", &node.port_range, product.ports.len())?;
    validate_range(
        node_index,
        "input values",
        &node.input_value_range,
        product.input_values.len(),
    )
}

const fn validate_range<E>(
    node_index: usize,
    field: &'static str,
    range: &Range<usize>,
    available: usize,
) -> Result<(), PackedGraphBindError<E>> {
    if range.start <= range.end && range.end <= available {
        return Ok(());
    }
    Err(PackedGraphBindError::RangeOutOfBounds {
        node_index,
        field,
        start: range.start,
        end: range.end,
        available,
    })
}

struct AotGraphManifestReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> AotGraphManifestReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn read_bytes(
        &mut self,
        field: &'static str,
        length: usize,
    ) -> Result<&'a [u8], AotGraphManifestError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(AotGraphManifestError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(AotGraphManifestError::UnexpectedEof { field })?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], AotGraphManifestError> {
        self.read_bytes(field, N).map(|bytes| {
            bytes
                .try_into()
                .expect("read_bytes returned exactly the requested array length")
        })
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, AotGraphManifestError> {
        Ok(u32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_text(&mut self, field: &'static str) -> Result<&'a str, AotGraphManifestError> {
        let length = self.read_u32("text byte count")? as usize;
        let bytes = self.read_bytes(field, length)?;
        std::str::from_utf8(bytes)
            .map_err(|source| AotGraphManifestError::InvalidUtf8 { field, source })
    }
}

fn range_for_str(bytes: &[u8], value: &str) -> Range<usize> {
    range_for_slice(bytes, value.as_bytes())
}

fn range_for_slice(bytes: &[u8], value: &[u8]) -> Range<usize> {
    let base = bytes.as_ptr() as usize;
    let start = value.as_ptr() as usize - base;
    let end = start + value.len();
    debug_assert!(end <= bytes.len());
    start..end
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
    use uuid::uuid;

    #[derive(Debug, Default)]
    struct TestAotRuntimeContext {
        executed: Vec<uuid::Uuid>,
    }

    const TEST_AOT_RUNTIME_CONTEXT_TYPE: &str = "az.tests.TestAotRuntimeContext";

    fn test_aot_graph_entry(
        mut context: AotGraphExecuteContext<'_>,
    ) -> Result<(), AotGraphExecutionError> {
        let graph_instance = context.graph_instance;
        context
            .runtime_context_mut::<TestAotRuntimeContext>()?
            .executed
            .push(graph_instance);
        Ok(())
    }

    declare_caps!(GraphCaps:);

    /// Stands in for the generated glue module a build script writes: the
    /// registration is a `const` the emitter produced, and the module hands
    /// the whole set to one registrar call.
    const TEST_AOT_GRAPH: AotGraphRuntimeRegistration = AotGraphRuntimeRegistration::new(
        "az.tests.pkg",
        "az_tests::compiled_graph",
        "az.tests.graph",
        TEST_AOT_RUNTIME_CONTEXT_TYPE,
        test_aot_graph_entry,
    );

    struct Graphs;

    impl Contribution for Graphs {
        type Caps = GraphCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.graph-runtime-tests"),
                contribution: ContributionId::new("graphs"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, GraphCaps>) {
            ctx.registrar::<AotGraphRuntimeRegistration>()
                .register(TEST_AOT_GRAPH);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::Server);
        composer
            .add(Graphs, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    const FORBIDDEN_GRAPH_RUNTIME_DEPS: &[&str] = &[
        "az-asset",
        "az-asset-builder",
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-inspector",
        "az-editor-ui",
        "az-engine",
        "az-framework",
        "az-graph-builder",
        "az-node-graph",
        "az-observability",
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
        "az-schema",
        "az-session",
        "az-sessiond",
        "bevy",
        "gpui",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "sample-plugin",
        "ron",
        "serde",
        "serde_json",
    ];

    #[test]
    fn graph_runtime_crate_stays_runtime_product_only() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-graph-runtime Cargo.toml");
        let manifest =
            toml::from_str::<Value>(&manifest).expect("parse az-graph-runtime Cargo.toml");
        let workspace_manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
                .expect("read workspace Cargo.toml");
        let workspace_manifest =
            toml::from_str::<Value>(&workspace_manifest).expect("parse workspace Cargo.toml");
        let violations = forbidden_graph_runtime_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-graph-runtime must stay a runtime product/bind crate; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn graph_runtime_source_has_no_authoring_transport_or_service_boundary() {
        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("collect az-graph-runtime production sources");
        let forbidden = [
            ("use az_node_graph", "visual graph authoring model import"),
            ("az_node_graph::", "visual graph authoring model API"),
            ("use az_graph_builder", "asset builder import"),
            ("az_graph_builder::", "asset builder API"),
            ("use az_proto_", "protocol crate import"),
            ("capnp::", "raw Cap'n Proto API"),
            ("use az_project", "project workflow crate import"),
            ("use az_project_host", "project-host service import"),
            ("use az_runtime_host", "runtime-host service import"),
            ("use az_editor", "editor crate import"),
            ("gpui::", "editor UI API"),
            ("ron::", "authored graph source codec"),
            ("serde::", "authored descriptor serialization"),
            ("std::fs::", "filesystem IO"),
            ("use std::fs", "filesystem IO import"),
            ("std::process::", "process spawning API"),
            ("use std::process", "process spawning import"),
        ];
        let mut violations = Vec::new();

        for source in sources {
            if source.path.ends_with("bevy_asset.rs") {
                continue;
            }

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
            "az-graph-runtime core source must stay authoring/service/UI agnostic:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn bevy_integration_is_feature_isolated_to_asset_loader() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-graph-runtime Cargo.toml");
        let manifest =
            toml::from_str::<Value>(&manifest).expect("parse az-graph-runtime Cargo.toml");
        assert!(
            manifest["dependencies"].get("bevy").is_none(),
            "az-graph-runtime must not depend on the full Bevy meta crate"
        );
        for dependency in ["bevy_app", "bevy_asset", "bevy_reflect"] {
            let bevy_dep = manifest["dependencies"][dependency]
                .as_table()
                .unwrap_or_else(|| panic!("{dependency} dependency must stay table-form"));
            assert_eq!(
                bevy_dep.get("optional").and_then(Value::as_bool),
                Some(true),
                "{dependency} must stay optional"
            );
        }
        assert!(
            manifest["features"]["default"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "az-graph-runtime default feature set must stay runtime-core only"
        );
        let bevy_feature = manifest["features"]["bevy"]
            .as_array()
            .expect("bevy feature must list narrow dependencies");
        for expected in ["dep:bevy_app", "dep:bevy_asset", "dep:bevy_reflect"] {
            assert!(
                bevy_feature
                    .iter()
                    .any(|value| value.as_str() == Some(expected)),
                "bevy feature must include `{expected}`"
            );
        }

        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("collect az-graph-runtime production sources");
        let bevy_violations = sources
            .iter()
            .filter(|source| !source.path.ends_with("bevy_asset.rs"))
            .filter(|source| source.source.contains("bevy::") || source.source.contains("use bevy"))
            .map(|source| source.path.display().to_string())
            .collect::<Vec<_>>();

        assert!(
            bevy_violations.is_empty(),
            "Bevy imports must stay isolated to bevy_asset.rs: {}",
            bevy_violations.join(", ")
        );
    }

    fn forbidden_graph_runtime_dependencies(
        manifest: &Value,
        workspace_manifest: &Value,
    ) -> Vec<String> {
        let mut exact_names = Vec::new();
        exact_names.extend_from_slice(FORBIDDEN_GRAPH_RUNTIME_DEPS);
        exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
        forbidden_production_dependencies(
            manifest,
            workspace_manifest,
            DependencyBoundary::new(
                &exact_names,
                PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
                PROJECT_OR_FORMAT_PATH_SEGMENTS,
            )
            .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES),
        )
    }

    #[test]
    fn a_composed_graph_resolves_and_executes_by_package_and_symbol() {
        let composer = composed();
        let registration = resolve_aot_graph(
            composer.registries(),
            "az.tests.pkg",
            "az_tests::compiled_graph",
        )
        .unwrap();
        assert_eq!(registration.package(), "az.tests.pkg");
        assert_eq!(registration.entry_symbol(), "az_tests::compiled_graph");
        assert_eq!(registration.graph_type(), "az.tests.graph");
        assert_eq!(registration.context_type(), TEST_AOT_RUNTIME_CONTEXT_TYPE);

        let graph_instance = uuid!("018f0c5a-0000-7000-8000-0000000000a0");
        let mut runtime_context = TestAotRuntimeContext::default();

        registration
            .execute(graph_instance, &mut runtime_context)
            .unwrap();

        assert_eq!(runtime_context.executed, vec![graph_instance]);
    }

    #[test]
    fn every_composed_graph_is_attributed_to_its_contribution() {
        let report = composed().finalize().expect("composition is valid");
        let graphs = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "aot-graph")
            .collect::<Vec<_>>();

        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0].key, "az.tests.pkg::az_tests::compiled_graph");
        assert_eq!(graphs[0].instance.gem.as_str(), "azoth.graph-runtime-tests");
    }

    /// The projection module a build script writes hands the registrar an
    /// array literal of what was compiled, which for a project that compiled
    /// no graphs is empty. That is a legitimate composition, and this pins
    /// that the emitter's empty output still type-checks.
    #[test]
    fn a_project_with_no_compiled_graphs_composes_an_empty_registry() {
        struct Empty;

        impl Contribution for Empty {
            type Caps = GraphCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                ContributionDescriptor {
                    gem: GemId::new("azoth.graph-runtime-none"),
                    contribution: ContributionId::new("graphs"),
                    roles: &[],
                }
            }

            fn register(&self, ctx: &mut GemContext<'_, GraphCaps>) {
                ctx.registrar::<AotGraphRuntimeRegistration>()
                    .register_many([]);
            }
        }

        let mut composer = Composer::new(GemTargetRole::Server);
        composer
            .add(Empty, ProductActivation::default())
            .expect("an empty floor composes");
        composer.finalize().expect("composition is valid");

        assert!(aot_graphs(composer.registries()).is_empty());
    }

    #[test]
    fn a_symbol_this_host_did_not_compose_is_a_named_lookup_failure() {
        let composer = composed();
        let error = resolve_aot_graph(composer.registries(), "az.missing.pkg", "az_missing::graph")
            .unwrap_err();
        assert_eq!(
            error,
            AotGraphLookupError::MissingEntry {
                package: "az.missing.pkg".to_string(),
                entry_symbol: "az_missing::graph".to_string(),
            }
        );
    }

    #[test]
    fn two_graphs_under_one_package_and_symbol_fail_composition() {
        struct Rival;

        impl Contribution for Rival {
            type Caps = GraphCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                ContributionDescriptor {
                    gem: GemId::new("azoth.graph-runtime-rival"),
                    contribution: ContributionId::new("graphs"),
                    roles: &[],
                }
            }

            fn register(&self, ctx: &mut GemContext<'_, GraphCaps>) {
                // A different graph type behind the same resolution pair: the
                // pair is what a runtime looks up, so this is the collision
                // that matters.
                ctx.registrar::<AotGraphRuntimeRegistration>().register(
                    AotGraphRuntimeRegistration::new(
                        "az.tests.pkg",
                        "az_tests::compiled_graph",
                        "az.tests.other-graph",
                        TEST_AOT_RUNTIME_CONTEXT_TYPE,
                        test_aot_graph_entry,
                    ),
                );
            }
        }

        let mut composer = composed();
        composer
            .add(Rival, ProductActivation::default())
            .expect("an empty floor composes");

        let error = composer.finalize().expect_err("the pair collides");
        let ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = error
        else {
            panic!("expected a duplicate-key failure, got {error}");
        };
        assert_eq!(registry, "aot-graph");
        assert_eq!(key, "az.tests.pkg::az_tests::compiled_graph");
        assert_eq!(first.gem.as_str(), "azoth.graph-runtime-tests");
        assert_eq!(second.gem.as_str(), "azoth.graph-runtime-rival");
    }

    #[test]
    fn aot_runtime_entry_reports_context_type_mismatches_before_running_graph() {
        let composer = composed();
        let registration = resolve_aot_graph(
            composer.registries(),
            "az.tests.pkg",
            "az_tests::compiled_graph",
        )
        .unwrap();
        let mut wrong_context = Vec::<String>::new();

        let error = registration
            .execute(uuid::Uuid::nil(), &mut wrong_context)
            .unwrap_err();

        assert_eq!(
            error,
            AotGraphExecutionError::ContextType {
                expected: std::any::type_name::<TestAotRuntimeContext>(),
            }
        );
        assert!(wrong_context.is_empty());
    }

    #[test]
    fn aot_graph_manifest_decodes_native_runtime_entry_contract() {
        let bytes = sample_aot_manifest();
        let manifest = decode_aot_graph_manifest(&bytes).unwrap();

        assert_eq!(manifest.header.version, AOT_GRAPH_MANIFEST_VERSION);
        assert_eq!(manifest.header.flags, 0x3);
        assert_eq!(manifest.header.graph_type_version, 1);
        assert_eq!(manifest.header.node_catalog_version, 1);
        assert_eq!(manifest.header.node_catalog_hash, [0x5a; 32]);
        assert_eq!(
            manifest.header.source_uuid,
            uuid!("018f0c5a-0000-7000-8000-0000000000a1")
        );
        assert_eq!(manifest.graph_type, "az.tests.graph");
        assert_eq!(manifest.product_kind, "azoth.graph.generated-rust");
        assert_eq!(manifest.language, "rust");
        assert_eq!(manifest.package, "az.tests.pkg");
        assert_eq!(manifest.entry_symbol, "az_tests::compiled_graph");
        assert_eq!(manifest.context_type, TEST_AOT_RUNTIME_CONTEXT_TYPE);
    }

    #[test]
    fn aot_graph_manifest_rejects_version_in_magic() {
        let bytes = b"AZGAOT01";
        let error = decode_aot_graph_manifest(bytes).unwrap_err();
        assert!(matches!(error, AotGraphManifestError::BadMagic { .. }));
    }

    #[test]
    fn product_owns_bytes_and_exposes_runtime_view() {
        let bytes = sample_empty_product();
        let product = PackedGraphProduct::from_bytes(bytes.into_boxed_slice()).unwrap();

        assert_eq!(product.header.version, PACKED_GRAPH_IR_VERSION);
        assert_eq!(product.graph_type(), "az.tests.graph");
        assert_eq!(product.product_kind(), "azoth.graph.logic-ir");
        assert_eq!(product.backend(), "az.tests.compiler");
        assert_eq!(product.ir_schema(), "azoth.graph.logic-ir/v1");
        assert_eq!(product.capability_markers().len(), 0);
        assert!(product.nodes.is_empty());
        assert!(product.execution_order.is_empty());
    }

    #[test]
    fn product_binds_runtime_symbols_once_into_execution_plan() {
        let product =
            PackedGraphProduct::from_bytes(sample_single_node_product().into_boxed_slice())
                .unwrap();
        let plan = product.bind(&SymbolBinder).unwrap();

        assert_eq!(plan.nodes().len(), 1);
        assert_eq!(plan.execution_order(), &[0]);
        let bound = plan.execution_nodes().next().unwrap();
        assert_eq!(bound.node_index, 0);
        assert_eq!(bound.node_id, product.nodes[0].id);
        assert_eq!(
            bound.prepared_node,
            PreparedSymbolNode {
                package: "az.tests.pkg".to_string(),
                symbol: "az_tests::print".to_string(),
                input_count: 1,
                first_input: Some(vec![0xaa, 0xbb, 0xcc]),
            }
        );
    }

    #[test]
    fn execution_plan_runs_prepared_nodes_in_precomputed_order() {
        let product =
            PackedGraphProduct::from_bytes(sample_single_node_product().into_boxed_slice())
                .unwrap();
        let plan = product.bind(&SymbolBinder).unwrap();
        let mut executor = SymbolExecutor;
        let mut executed = Vec::new();

        plan.execute(&mut executor, &mut executed).unwrap();

        assert_eq!(executed, vec!["az.tests.pkg::az_tests::print".to_string()]);
    }

    #[test]
    fn product_bind_reports_node_binder_errors_with_node_identity() {
        let product =
            PackedGraphProduct::from_bytes(sample_single_node_product().into_boxed_slice())
                .unwrap();
        let error = product.bind(&RejectBinder).unwrap_err();

        match error {
            PackedGraphBindError::BindNode {
                node_index,
                node_type,
                binder_error,
                ..
            } => {
                assert_eq!(node_index, 0);
                assert_eq!(node_type, "az.tests.Print");
                assert_eq!(binder_error, "missing binding");
            }
            other => panic!("unexpected bind error: {other:?}"),
        }
    }

    #[test]
    fn product_bind_reports_binding_lookup_errors_before_resolver_dispatch() {
        let mut product =
            PackedGraphProduct::from_bytes(sample_single_node_product().into_boxed_slice())
                .unwrap();
        product.nodes[0].runtime_binding = PackedRuntimeBinding::RustSymbol {
            package: 99,
            symbol: 6,
        };

        let error = product.bind(&SymbolBinder).unwrap_err();

        match error {
            PackedGraphBindError::BindingLookup {
                node_index,
                source:
                    PackedGraphBindLookupError::StringIndexOutOfRange {
                        field,
                        index,
                        strings,
                    },
                ..
            } => {
                assert_eq!(node_index, 0);
                assert_eq!(field, "runtime binding package");
                assert_eq!(index, 99);
                assert_eq!(strings, 9);
            }
            other => panic!("unexpected bind error: {other:?}"),
        }
    }

    #[test]
    fn decoder_rejects_version_in_magic() {
        let bytes = b"AZGIR001";
        let error = decode_packed_graph_ir(bytes).unwrap_err();
        assert!(matches!(error, PackedGraphIrError::BadMagic { .. }));
    }

    fn sample_empty_product() -> Vec<u8> {
        let strings = [
            "az.tests.graph",
            "azoth.graph.logic-ir",
            "az.tests.compiler",
            "azoth.graph.logic-ir/v1",
        ];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PACKED_GRAPH_IR_MAGIC);
        write_u32(&mut bytes, PACKED_GRAPH_IR_VERSION);
        write_u32(&mut bytes, 0x3);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 1);
        bytes.extend_from_slice(&[0x5a; 32]);
        bytes.extend_from_slice(uuid::Uuid::nil().as_bytes());
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 2);
        write_u32(&mut bytes, 3);
        write_u32(
            &mut bytes,
            u32::try_from(strings.len()).expect("fixture string count"),
        );
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 0);
        for value in strings {
            write_u32(
                &mut bytes,
                u32::try_from(value.len()).expect("fixture string length"),
            );
            bytes.extend_from_slice(value.as_bytes());
        }
        bytes
    }

    fn sample_aot_manifest() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(AOT_GRAPH_MANIFEST_MAGIC);
        write_u32(&mut bytes, AOT_GRAPH_MANIFEST_VERSION);
        write_u32(&mut bytes, 0x3);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 1);
        bytes.extend_from_slice(&[0x5a; 32]);
        bytes.extend_from_slice(uuid!("018f0c5a-0000-7000-8000-0000000000a1").as_bytes());
        write_text(&mut bytes, "az.tests.graph");
        write_text(&mut bytes, "azoth.graph.generated-rust");
        write_text(&mut bytes, "rust");
        write_text(&mut bytes, "az.tests.pkg");
        write_text(&mut bytes, "az_tests::compiled_graph");
        write_text(&mut bytes, TEST_AOT_RUNTIME_CONTEXT_TYPE);
        bytes
    }

    fn sample_single_node_product() -> Vec<u8> {
        let strings = [
            "az.tests.graph",
            "azoth.graph.logic-ir",
            "az.tests.compiler",
            "azoth.graph.logic-ir/v1",
            "az.tests.Print",
            "az.tests.pkg",
            "az_tests::print",
            "value",
            "az.schema.bytes",
        ];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PACKED_GRAPH_IR_MAGIC);
        write_u32(&mut bytes, PACKED_GRAPH_IR_VERSION);
        write_u32(&mut bytes, 0x3);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 1);
        bytes.extend_from_slice(&[0x5a; 32]);
        bytes.extend_from_slice(uuid!("018f0c5a-0000-7000-8000-0000000000aa").as_bytes());
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 2);
        write_u32(&mut bytes, 3);
        write_u32(
            &mut bytes,
            u32::try_from(strings.len()).expect("fixture string count"),
        );
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 1);
        for value in strings {
            write_u32(
                &mut bytes,
                u32::try_from(value.len()).expect("fixture string length"),
            );
            bytes.extend_from_slice(value.as_bytes());
        }

        bytes.extend_from_slice(uuid!("018f0c5a-0000-7000-8000-000000000001").as_bytes());
        write_u32(&mut bytes, 4);
        write_u32(&mut bytes, 1);
        write_u8(&mut bytes, 1);
        write_u32(&mut bytes, 5);
        write_u32(&mut bytes, 6);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 7);
        write_u8(&mut bytes, 0);
        write_u8(&mut bytes, 0);
        write_u8(&mut bytes, 1);
        write_u32(&mut bytes, 8);
        write_u32(&mut bytes, 1);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, 3);
        bytes.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        write_u32(&mut bytes, 0);
        bytes
    }

    fn write_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u8(bytes: &mut Vec<u8>, value: u8) {
        bytes.push(value);
    }

    fn write_text(bytes: &mut Vec<u8>, value: &str) {
        write_u32(
            bytes,
            u32::try_from(value.len()).expect("fixture string length"),
        );
        bytes.extend_from_slice(value.as_bytes());
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct PreparedSymbolNode {
        package: String,
        symbol: String,
        input_count: usize,
        first_input: Option<Vec<u8>>,
    }

    struct SymbolBinder;

    impl PackedGraphNodeBinder for SymbolBinder {
        type Node = PreparedSymbolNode;
        type Error = &'static str;

        fn bind_node(
            &self,
            request: PackedGraphNodeBindRequest<'_>,
        ) -> Result<Self::Node, Self::Error> {
            let PackedRuntimeBindingRef::RustSymbol { package, symbol } =
                request.runtime_binding_ref
            else {
                return Err("unsupported binding kind");
            };
            assert_eq!(request.node_type, "az.tests.Print");
            assert_eq!(request.ports.len(), 1);
            let mut input_values = request.input_values.iter();
            let first_input = input_values.next().map(|value| {
                assert_eq!(value.port_index, 0);
                value.bytes.to_vec()
            });
            assert!(input_values.next().is_none());
            Ok(PreparedSymbolNode {
                package: package.to_string(),
                symbol: symbol.to_string(),
                input_count: request.input_values.len(),
                first_input,
            })
        }
    }

    struct SymbolExecutor;

    impl PackedGraphExecutor for SymbolExecutor {
        type Node = PreparedSymbolNode;
        type Context = Vec<String>;
        type Error = &'static str;

        fn execute_node(
            &mut self,
            node: &PackedGraphBoundNode<Self::Node>,
            context: &mut Self::Context,
        ) -> Result<(), Self::Error> {
            context.push(format!(
                "{}::{}",
                node.prepared_node.package, node.prepared_node.symbol
            ));
            Ok(())
        }
    }

    struct RejectBinder;

    impl PackedGraphNodeBinder for RejectBinder {
        type Node = ();
        type Error = &'static str;

        fn bind_node(
            &self,
            _request: PackedGraphNodeBindRequest<'_>,
        ) -> Result<Self::Node, Self::Error> {
            Err("missing binding")
        }
    }
}
