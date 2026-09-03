@0xe203af83e43044c1;

using Core = import "/azoth/core.capnp";
using Authoring = import "/azoth/authoring.capnp";
using Asset = import "/azoth/asset.capnp";
using Runtime = import "/azoth/runtime.capnp";

struct DocumentId {
  value @0 :Text;
}

struct ObjectId {
  uuid @0 :Data;
}

struct DocumentRevision {
  value @0 :UInt64;
}

struct AuthoredHistoryCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  union {
    expectedRevision @2 :DocumentRevision;
    noExpectedRevision @3 :Void;
  }
  clientCommandId @4 :Text;
}

# Schema-declared type-level button action dispatched against one authored
# object. The action name must match a button declared by the schema's
# TypeEditHints; project-host runs the registered handler inside the same
# revision/undo machinery as a normal authored edit.
struct SchemaActionCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  object @2 :ObjectId;
  schemaType @3 :Text;
  action @4 :Text;
  union {
    expectedRevision @5 :DocumentRevision;
    noExpectedRevision @6 :Void;
  }
  clientCommandId @7 :Text;
}

struct AddPrefabComponentCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  entity @2 :ObjectId;
  componentSchemaType @3 :Text;
  union {
    expectedRevision @4 :DocumentRevision;
    noExpectedRevision @5 :Void;
  }
  clientCommandId @6 :Text;
}

struct AddPrefabEntityCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  parentEntity @2 :ObjectId;
  union {
    expectedRevision @3 :DocumentRevision;
    noExpectedRevision @4 :Void;
  }
  clientCommandId @5 :Text;
}

struct RemovePrefabEntityCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  entity @2 :ObjectId;
  union {
    expectedRevision @3 :DocumentRevision;
    noExpectedRevision @4 :Void;
  }
  clientCommandId @5 :Text;
}

struct RemovePrefabComponentCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  entity @2 :ObjectId;
  component @3 :ObjectId;
  union {
    expectedRevision @4 :DocumentRevision;
    noExpectedRevision @5 :Void;
  }
  clientCommandId @6 :Text;
}

struct AuthoredObjectCommand {
  capability @0 :Core.Capability;
  document @1 :DocumentId;
  schemaType @2 :Text;
  expectedRevision @3 :DocumentRevision;
  clientCommandId @4 :Text;
  union {
    addDefault @5 :Void;
    duplicate @6 :ObjectId;
    remove @7 :ObjectId;
  }
}

struct RuntimeLaunchSnapshotRequest {
  capability @0 :Core.Capability;
  role @1 :Runtime.RuntimeRole;
  projectId @2 :Text;
  sessionSlug @3 :Text;
  projectRoot @4 :Text;
  workspacePath @5 :Text;
  workspaceId @6 :Int64;
  includeUnsavedJournal @7 :Bool;
  launchProfile @8 :Text;
  assetSourceRoots @9 :List(Runtime.RuntimeAssetSourceRoot);
  sessionId @10 :Core.SessionId;
  runtimeLaunchCapability @11 :Core.Capability;
  assetPackageRoots @12 :List(Runtime.RuntimeAssetPackageRoot);
}

struct AuthoredEditResult {
  revision @0 :DocumentRevision;
  journalEntryId @1 :Text;
  document @2 :DocumentId;
}

struct SavedDocument {
  document @0 :DocumentId;
  revision @1 :DocumentRevision;
  sourcePath @2 :Text;
  schemaType @3 :Text;
  contentHash @4 :Data;
  byteLength @5 :UInt64;
}

struct AuthoredDocumentSnapshot {
  document @0 :DocumentId;
  revision @1 :DocumentRevision;
  objects @2 :List(AuthoredObjectSnapshot);
}

struct AuthoredDocumentStatus {
  document @0 :DocumentId;
  revision @1 :DocumentRevision;
  unsavedChanges @2 :Bool;
  objectCount @3 :UInt32;
  journalEntryCount @4 :UInt32;
  sourcePath @5 :Text;
  union {
    noSavedRevision @6 :Void;
    savedRevision @7 :DocumentRevision;
  }
}

struct AuthoredDocumentRecoverySnapshot {
  document @0 :DocumentId;
  revision @1 :DocumentRevision;
  schemaType @2 :Text;
  sourcePath @3 :Text;
  sourceHash @4 :Data;
  payloadRon @5 :Text;
  union {
    noSavedRevision @6 :Void;
    savedRevision @7 :DocumentRevision;
  }
}

struct AuthoredDocumentList {
  documents @0 :List(AuthoredDocumentListEntry);
}

struct AuthoredDocumentListEntry {
  document @0 :DocumentId;
  sourcePath @1 :Text;
  schemaType @2 :Text;
  revision @3 :DocumentRevision;
  unsavedChanges @4 :Bool;
  objectCount @5 :UInt32;
  journalEntryCount @6 :UInt32;
  loaded @7 :Bool;
  valid @8 :Bool;
  diagnostic @9 :Text;
  union {
    noSavedRevision @10 :Void;
    savedRevision @11 :DocumentRevision;
  }
}

struct AuthoredObjectSnapshot {
  object @0 :ObjectId;
  schemaType @1 :Text;
  value @2 :AuthoredValue;
  # Per-instance conditional-rule overrides evaluated by project-host through
  # the schema's registered rule evaluator. The editor consumes these folded
  # booleans; it never executes rule predicates.
  ruleOverrides @3 :List(FieldRuleOverride);
  schemaVersion @4 :UInt32;
}

struct FieldRuleOverride {
  fieldId @0 :UInt32;
  hidden @1 :Core.OptionalBool;
  readOnly @2 :Core.OptionalBool;
}

struct AuthoredValue {
  union {
    null @0 :Void;
    boolValue @1 :Bool;
    i64Value @2 :Int64;
    u64Value @3 :UInt64;
    f64Value @4 :Float64;
    text @5 :Text;
    assetPath @6 :Text;
    objectRef @7 :ObjectId;
    list @8 :List(AuthoredValue);
    structFields @9 :List(FieldValue);
    mapEntries @10 :List(MapEntry);
    variant @11 :VariantValue;
  }

  struct FieldValue {
    field @0 :UInt32;
    value @1 :AuthoredValue;
  }

  struct MapEntry {
    key @0 :Text;
    value @1 :AuthoredValue;
  }

  struct VariantValue {
    variant @0 :UInt32;
    union {
      noValue @1 :Void;
      value @2 :AuthoredValue;
    }
  }
}

struct SchemaCatalogSnapshot {
  schemaVersion @0 :UInt32;
  generatedUnixMs @1 :UInt64;
  schemas @2 :List(TypeSchema);
}

# Authoritative project+gem component identities. Presence in this catalog,
# not serializable struct shape, makes a schema attachable to an entity.
struct ComponentTypeDescriptor {
  schemaType @0 :Text;
  nativeTypeId @1 :Text;
  provides @2 :List(Text);
  requires @3 :List(Text);
  incompatible @4 :List(Text);
  editorExport @5 :Bool;
  runtimeExport @6 :Bool;
}

struct TypeSchema {
  name @0 :Text;
  rustType @1 :Text;
  version @2 :UInt32;
  docs @3 :Text;
  kind @4 :SchemaKind;
  edit @5 :TypeEditHints;
  defaultValue @6 :SchemaValue;
}

# Portable schema-authored value. Object references remain stable authored
# strings here rather than the runtime UUID-only ObjectId transport.
struct SchemaValue {
  union {
    null @0 :Void;
    boolValue @1 :Bool;
    i64Value @2 :Int64;
    u64Value @3 :UInt64;
    f64Value @4 :Float64;
    text @5 :Text;
    assetPath @6 :Text;
    objectRef @7 :Text;
    list @8 :List(SchemaValue);
    structFields @9 :List(FieldValue);
    mapEntries @10 :List(MapEntry);
    variant @11 :VariantValue;
  }

  struct FieldValue {
    field @0 :UInt32;
    value @1 :SchemaValue;
  }

  struct MapEntry {
    key @0 :Text;
    value @1 :SchemaValue;
  }

  struct VariantValue {
    variant @0 :UInt32;
    union {
      noValue @1 :Void;
      value @2 :SchemaValue;
    }
  }
}

struct SchemaKind {
  union {
    structFields @0 :List(FieldSchema);
    unitStruct @1 :Void;
    enumVariants @2 :List(VariantSchema);
    primitive @3 :PrimitiveSchema;
    list @4 :ListSchema;
    optional @5 :OptionalSchema;
    map @6 :MapSchema;
  }
}

struct ListSchema {
  elementType @0 :Text;
}

struct OptionalSchema {
  valueType @0 :Text;
}

struct MapSchema {
  keyType @0 :Text;
  valueType @1 :Text;
}

struct PrimitiveSchema {
  union {
    boolValue @0 :Void;
    unsignedInteger @1 :UInt8;
    signedInteger @2 :UInt8;
    floatValue @3 :UInt8;
    text @4 :Void;
  }
}

struct FieldSchema {
  id @0 :UInt32;
  name @1 :Text;
  rustType @2 :Text;
  docs @3 :Text;
  edit @4 :FieldEditHints;
  schemaType @5 :Text;
  defaultValue @6 :SchemaValue;
  constraints @7 :ValueConstraints;
}

struct ValueConstraints {
  min @0 :Core.OptionalF64;
  max @1 :Core.OptionalF64;
  minLength @2 :Core.OptionalU32;
  maxLength @3 :Core.OptionalU32;
  allowedStrings @4 :List(Text);
  allowedVariants @5 :List(UInt32);
}

struct VariantSchema {
  id @0 :UInt32;
  name @1 :Text;
  docs @2 :Text;
  edit @3 :VariantEditHints;
  payloadSchemaType @4 :Text;
  payloadRustType @5 :Text;
}

struct TypeEditHints {
  category @0 :Text;
  icon @1 :Text;
  description @2 :Text;
  document @3 :SourceDocumentHint;
  buttons @4 :List(ButtonHint);
  label @5 :Text;
}

struct ButtonHint {
  label @0 :Text;
  action @1 :Text;
}

struct SourceDocumentHint {
  sourceRoot @0 :Text;
  pathPrefix @1 :Text;
  extension @2 :Text;
}

struct FieldEditHints {
  label @0 :Text;
  description @1 :Text;
  readOnly @2 :Bool;
  hidden @3 :Bool;
  widget @4 :WidgetHint;
  # Conditional-rule method names resolved through link-time registrations in
  # project-host; empty text means no rule.
  visibleIf @5 :Text;
  readOnlyIf @6 :Text;
  changeNotify @7 :Text;
}

struct VariantEditHints {
  label @0 :Text;
  description @1 :Text;
  hidden @2 :Bool;
}

struct WidgetHint {
  union {
    default @0 :Void;
    slider @1 :SliderWidget;
    number @2 :NumberWidget;
    checkbox @3 :Void;
    toggle @4 :Void;
    dropdown @5 :Void;
    vec2 @6 :VectorWidget;
    vec3 @7 :VectorWidget;
    vec4 @8 :VectorWidget;
    quat @9 :VectorWidget;
    color @10 :Void;
    asset @11 :Text;
    readOnly @12 :Void;
    hidden @13 :Void;
    textarea @14 :TextareaWidget;
  }

  struct SliderWidget {
    min @0 :Float64;
    max @1 :Float64;
    step @2 :Core.OptionalF64;
    suffix @3 :Text;
  }

  struct NumberWidget {
    step @0 :Core.OptionalF64;
    suffix @1 :Text;
  }

  struct TextareaWidget {
    rows @0 :Core.OptionalU32;
  }

  struct VectorWidget {
    suffix @0 :Text;
  }
}

struct GameDataCatalogSnapshot {
  catalogVersion @0 :UInt32;
  generatedUnixMs @1 :UInt64;
  tables @2 :List(GameDataTableDescriptor);
  families @3 :List(GameDataTableFamilyDescriptor);
  managers @4 :List(GameDataManagerCatalogEntry);
  diagnostics @5 :List(GameDataCatalogDiagnostic);
}

struct GameDataTableDescriptor {
  name @0 :Text;
  rowType @1 :Text;
  sourceRoot @2 :Text;
  sourcePath @3 :Text;
  owner @4 :Text;
  schemaHash @5 :Core.OptionalU64;
  documentId @6 :Text;
  schemaType @7 :Text;
  category @8 :Text;
  rowCount @9 :Core.OptionalU64;
  families @10 :List(Text);
  # Canonical Asset Processor identity for generic source-authoring. This is
  # deliberately distinct from rowType/schemaType, which describe rows.
  sourceRef @11 :Asset.WorkspaceSourceFileRef;
}

struct GameDataTableFamilyDescriptor {
  name @0 :Text;
  rowType @1 :Text;
  owner @2 :Text;
  duplicateKeyPolicy @3 :Text;
  tables @4 :List(Text);
}

struct GameDataProviderTarget {
  kind @0 :Text;
  name @1 :Text;
  rowType @2 :Text;
}

struct GameDataManagerNodeRef {
  key @0 :Text;
  label @1 :Text;
  kind @2 :Text;
}

struct GameDataKeyPolicy {
  kind @0 :Text;
  transforms @1 :List(Text);
  rejectZeroCrc @2 :Bool;
  storeKeyText @3 :Bool;
}

struct GameDataManagerInput {
  kind @0 :Text;
  name @1 :Text;
  rowType @2 :Text;
  sourceRoot @3 :Text;
  sourcePath @4 :Text;
  detail @5 :Text;
  providerKind @6 :Text;
}

struct GameDataRowFilter {
  field @0 :Text;
  predicate @1 :Text;
  compareField @2 :Text;
}

struct GameDataProjectionTransform {
  field @0 :Text;
  sourceColumn @1 :Text;
  kind @2 :Text;
}

struct GameDataSecondaryIndex {
  name @0 :Text;
  field @1 :Text;
  keyKind @2 :Text;
  storage @3 :Text;
  duplicateKeyPolicy @4 :Text;
}

struct GameDataCatalogDiagnostic {
  code @0 :Text;
  message @1 :Text;
  targetKey @2 :Text;
  targetLabel @3 :Text;
}

struct GameDataManagerCatalogEntry {
  key @0 :Text;
  name @1 :Text;
  owner @2 :Text;
  rowType @3 :Text;
  kind @4 :Text;
  outputType @5 :Text;
  readOnly @6 :Bool;
  providerTarget @7 :GameDataProviderTarget;
  keyPolicy @8 :GameDataKeyPolicy;
  duplicateKeyPolicy @9 :Text;
  inputs @10 :List(GameDataManagerInput);
  rowFilters @11 :List(GameDataRowFilter);
  projectionTransforms @12 :List(GameDataProjectionTransform);
  secondaryIndexes @13 :List(GameDataSecondaryIndex);
  sourceTargets @14 :List(GameDataProviderTarget);
  dependencies @15 :List(GameDataManagerNodeRef);
  dependents @16 :List(GameDataManagerNodeRef);
  diagnostics @17 :List(GameDataCatalogDiagnostic);
  projectionHash @18 :Data;
}

struct NodeTypeCatalogSnapshot {
  catalogVersion @0 :UInt32;
  generatedUnixMs @1 :UInt64;
  nodeTypes @2 :List(NodeTypeDescriptor);
}

struct NodeTypeDescriptor {
  id @0 :Text;
  version @1 :UInt32;
  displayName @2 :Text;
  categoryPath @3 :List(Text);
  description @4 :Text;
  ports @5 :List(NodePortDescriptor);
  capabilities @6 :List(NodeCapability);
  sourceLinks @7 :List(NodeSourceLink);
  tags @8 :List(Text);
  union {
    noRuntimeBinding @9 :Void;
    runtimeBinding @10 :NodeRuntimeBinding;
  }
}

struct NodePortDescriptor {
  id @0 :UInt32;
  name @1 :Text;
  direction @2 :NodePortDirection;
  value @3 :NodePortValue;
  capacity @4 :NodePortCapacity;
  description @5 :Text;
  union {
    noDefaultValue @6 :Void;
    defaultValue @7 :Authoring.ReflectedValueEnvelope;
  }
  layout @8 :NodePortLayout;
}

enum NodePortDirection {
  input @0;
  output @1;
}

enum NodePortCapacity {
  single @0;
  multiple @1;
}

struct NodePortLayout {
  side @0 :NodePortSide;
  union {
    noOrder @1 :Void;
    order @2 :UInt32;
  }
  attachment @3 :NodePortAttachment;
  fixedFractionPerMille @4 :UInt16;
}

enum NodePortAttachment {
  evenlySpaced @0;
  fixedFraction @1;
}

enum NodePortSide {
  north @0;
  east @1;
  south @2;
  west @3;
}

struct NodePortValue {
  union {
    execution @0 :Void;
    data @1 :DataValue;
    dynamicData @2 :DynamicDataValue;
  }

  struct DataValue {
    schemaType @0 :Text;
  }

  struct DynamicDataValue {
    group @0 :Text;
    acceptedSchemaTypes @1 :List(Text);
  }
}

struct NodeCapability {
  id @0 :Text;
  markers @1 :List(Text);
}

struct NodeRuntimeBinding {
  union {
    rustSymbol @0 :RustSymbolBinding;
    assetBuilder @1 :AssetBuilderBinding;
    runtimeComponent @2 :RuntimeComponentBinding;
    external @3 :ExternalBinding;
  }

  struct RustSymbolBinding {
    package @0 :Text;
    symbol @1 :Text;
    callAbi @2 :RustNodeCallAbi;
  }

  struct AssetBuilderBinding {
    builderId @0 :Text;
  }

  struct RuntimeComponentBinding {
    componentType @0 :Text;
  }

  struct ExternalBinding {
    kind @0 :Text;
    locator @1 :Text;
  }
}

struct RustNodeCallAbi {
  union {
    contextSchedule @0 :Void;
    typedDataflow @1 :RustTypedDataflowNodeCall;
  }
}

struct RustTypedDataflowNodeCall {
  parameters @0 :List(RustDataflowParameter);
  output @1 :RustDataflowOutput;
  result @2 :RustCallResult;
}

struct RustDataflowParameter {
  source @0 :RustDataflowParameterSource;
  inputPortId @1 :UInt32;
  rustType @2 :Text;
  passing @3 :RustValuePassing;
}

enum RustDataflowParameterSource {
  runtimeContext @0;
  inputPort @1;
}

enum RustValuePassing {
  byValue @0;
  bySharedRef @1;
  byMutableRef @2;
}

struct RustDataflowOutput {
  union {
    none @0 :Void;
    single @1 :RustDataflowSingleOutput;
    structFields @2 :RustDataflowStructFieldsOutput;
  }
}

struct RustDataflowSingleOutput {
  portId @0 :UInt32;
  rustType @1 :Text;
}

struct RustDataflowStructFieldsOutput {
  rustType @0 :Text;
  fields @1 :List(RustDataflowOutputField);
}

struct RustDataflowOutputField {
  portId @0 :UInt32;
  field @1 :Text;
  rustType @2 :Text;
}

enum RustCallResult {
  plain @0;
  result @1;
}

struct NodeSourceLink {
  package @0 :Text;
  modulePath @1 :Text;
  symbolPath @2 :Text;
  file @3 :Text;
  line @4 :UInt32;
  column @5 :UInt32;
  docsUrl @6 :Text;
}

enum NodeSourceLinkPathKind {
  unresolved @0;
  absolute @1;
  packageRelative @2;
  workspaceRelative @3;
  docsOnly @4;
}

struct NodeSourceLinkRequest {
  capability @0 :Core.Capability;
  sourceLink @1 :NodeSourceLink;
}

struct NodeSourceLinkTarget {
  package @0 :Text;
  modulePath @1 :Text;
  symbolPath @2 :Text;
  file @3 :Text;
  line @4 :UInt32;
  column @5 :UInt32;
  docsUrl @6 :Text;
  resolvedPath @7 :Text;
  packageId @8 :Text;
  packageRoot @9 :Text;
  pathKind @10 :NodeSourceLinkPathKind;
  exists @11 :Bool;
}

struct GraphTypeCatalogSnapshot {
  catalogVersion @0 :UInt32;
  generatedUnixMs @1 :UInt64;
  graphTypes @2 :List(GraphTypeDescriptor);
}

struct GraphTypeDescriptor {
  id @0 :Text;
  version @1 :UInt32;
  displayName @2 :Text;
  description @3 :Text;
  categoryPath @4 :List(Text);
  sourceWorkflow @5 :GraphSourceWorkflow;
  template @6 :GraphDocumentTemplate;
  allowedNodeCatalogs @7 :List(GraphNodeCatalogRequirement);
  compilerBackend @8 :GraphCompilerBackendDescriptor;
  runtimeProduct @9 :RuntimeGraphProductDescriptor;
  executionMode @10 :GraphExecutionMode;
  palettePolicy @11 :GraphPalettePolicy;
  tags @12 :List(Text);
}

struct GraphSourceWorkflow {
  workflowId @0 :Text;
  kind @1 :GraphSourceWorkflowKind;
  sourceSchema @2 :Text;
  defaultPathPrefix @3 :Text;
  defaultExtension @4 :Text;
  sourceRoot @5 :Text;
}

enum GraphSourceWorkflowKind {
  projectDocument @0;
  file @1;
}

struct GraphDocumentTemplate {
  graph @0 :VisualGraphDocument;
}

struct GraphNodeCatalogRequirement {
  catalogId @0 :Text;
  minimumVersion @1 :UInt32;
  requiredHash @2 :Data;
}

struct GraphCompilerBackendDescriptor {
  id @0 :Text;
  kind @1 :GraphCompilerBackendKind;
  capabilityMarkers @2 :List(Text);
}

struct GraphCompilerBackendKind {
  union {
    generatedRust @0 :GeneratedRustBackend;
    packedIr @1 :PackedIrBackend;
    shaderPipeline @2 :ShaderPipelineBackend;
    external @3 :ExternalBackend;
  }

  struct GeneratedRustBackend {
    package @0 :Text;
    entrySymbol @1 :Text;
    abi @2 :GeneratedRustGraphAbi;
  }

  struct PackedIrBackend {
    irSchema @0 :Text;
  }

  struct ShaderPipelineBackend {
    pipelineKind @0 :Text;
  }

  struct ExternalBackend {
    kind @0 :Text;
    locator @1 :Text;
  }
}

enum GeneratedRustGraphAbi {
  contextSchedule @0;
  typedDataflow @1;
}

struct RuntimeGraphProductDescriptor {
  assetType @0 :Text;
  productKind @1 :Text;
  streamable @2 :Bool;
  diffableChunks @3 :Bool;
  executionStrategy @4 :RuntimeGraphExecutionStrategy;
}

struct RuntimeGraphExecutionStrategy {
  union {
    packedIr @0 :Void;
    aotCompiledCode @1 :AotCompiledCode;
    hotReloadedCompiledModule @2 :HotReloadedCompiledModule;
    shaderPipeline @3 :ShaderPipelineRuntime;
    external @4 :ExternalRuntime;
  }

  struct AotCompiledCode {
    language @0 :Text;
    package @1 :Text;
    entrySymbol @2 :Text;
    contextType @3 :Text;
  }

  struct HotReloadedCompiledModule {
    abi @0 :Text;
    entrySymbol @1 :Text;
  }

  struct ShaderPipelineRuntime {
    pipelineKind @0 :Text;
  }

  struct ExternalRuntime {
    kind @0 :Text;
    locator @1 :Text;
  }
}

enum GraphExecutionMode {
  runtimeCompiled @0;
  editorInterpreted @1;
  runtimeCompiledAndEditorInterpreted @2;
}

struct GraphPalettePolicy {
  rootCategories @0 :List(Text);
  requiredNodeCapabilities @1 :List(Text);
  hiddenNodeTags @2 :List(Text);
}

struct GraphCommandBatchSnapshot {
  snapshotVersion @0 :UInt32;
  document @1 :DocumentId;
  clientBatchId @2 :Text;
  commands @3 :List(GraphCommand);
  union {
    noExpectedRevision @4 :Void;
    expectedRevision @5 :DocumentRevision;
  }
}

struct GraphCommandStatusSnapshot {
  snapshotVersion @0 :UInt32;
  document @1 :DocumentId;
  clientBatchId @2 :Text;
  appliedCommandCount @3 :UInt32;
  diagnostics @4 :List(GraphCommandDiagnostic);
  union {
    acceptedRevision @5 :DocumentRevision;
    rejected @6 :GraphCommandRejection;
  }
}

struct GraphDocumentSnapshot {
  snapshotVersion @0 :UInt32;
  document @1 :DocumentId;
  revision @2 :DocumentRevision;
  graph @3 :VisualGraphDocument;
}

struct VisualGraphDocument {
  documentVersion @0 :UInt32;
  graphType @1 :Text;
  nodes @2 :List(GraphNode);
  connections @3 :List(GraphConnection);
  comments @4 :List(GraphComment);
  union {
    noRequiredCatalogHash @5 :Void;
    requiredCatalogHash @6 :Data;
  }
}

struct GraphCommandRejection {
  reason @0 :Text;
  union {
    noCommandIndex @1 :Void;
    commandIndex @2 :UInt32;
  }
}

struct GraphCommandDiagnostic {
  message @0 :Text;
  severity @1 :GraphDiagnosticSeverity;
  union {
    noCommandIndex @2 :Void;
    commandIndex @3 :UInt32;
  }
}

enum GraphDiagnosticSeverity {
  info @0;
  warning @1;
  error @2;
}

struct GraphCommand {
  union {
    addNode @0 :GraphNode;
    removeNode @1 :GraphNodeId;
    setInputValue @2 :GraphInputValueCommand;
    moveNode @3 :GraphMoveNodeCommand;
    connect @4 :GraphConnection;
    disconnect @5 :GraphConnectionId;
    upsertComment @6 :GraphComment;
    removeComment @7 :GraphCommentId;
    setConnectionRoute @8 :GraphConnectionRouteCommand;
  }
}

struct GraphNodeId {
  uuid @0 :Data;
}

struct GraphConnectionId {
  uuid @0 :Data;
}

struct GraphCommentId {
  uuid @0 :Data;
}

struct GraphRouteAnchorId {
  uuid @0 :Data;
}

struct GraphNode {
  id @0 :GraphNodeId;
  nodeType @1 :Text;
  nodeTypeVersion @2 :UInt32;
  inputValues @3 :List(GraphInputValue);
  layout @4 :GraphNodeLayout;
}

struct GraphInputValue {
  port @0 :UInt32;
  value @1 :Authoring.ReflectedValueEnvelope;
}

struct GraphNodeLayout {
  x @0 :Float32;
  y @1 :Float32;
}

struct GraphInputValueCommand {
  node @0 :GraphNodeId;
  port @1 :UInt32;
  union {
    clear @2 :Void;
    value @3 :Authoring.ReflectedValueEnvelope;
  }
}

struct GraphMoveNodeCommand {
  node @0 :GraphNodeId;
  layout @1 :GraphNodeLayout;
}

struct GraphConnection {
  id @0 :GraphConnectionId;
  from @1 :GraphPortRef;
  to @2 :GraphPortRef;
  route @3 :GraphConnectionRoute;
}

struct GraphPortRef {
  node @0 :GraphNodeId;
  port @1 :UInt32;
}

struct GraphConnectionRouteCommand {
  connection @0 :GraphConnectionId;
  route @1 :GraphConnectionRoute;
}

struct GraphConnectionRoute {
  style @0 :GraphRouteStyle;
  anchors @1 :List(GraphRouteAnchor);
}

enum GraphRouteStyle {
  orthogonal @0;
  polyline @1;
  spline @2;
}

struct GraphRouteAnchor {
  id @0 :GraphRouteAnchorId;
  position @1 :GraphPoint;
  kind @2 :GraphRouteAnchorKind;
  outgoingSegment @3 :GraphRouteSegmentConstraint;
}

enum GraphRouteAnchorKind {
  userWaypoint @0;
  solverWaypoint @1;
  junction @2;
}

enum GraphRouteSegmentConstraint {
  flexible @0;
  fixed @1;
}

struct GraphPoint {
  x @0 :Float32;
  y @1 :Float32;
}

struct GraphComment {
  id @0 :GraphCommentId;
  text @1 :Text;
  bounds @2 :GraphCommentBounds;
}

struct GraphCommentBounds {
  x @0 :Float32;
  y @1 :Float32;
  width @2 :Float32;
  height @3 :Float32;
}

enum ProjectInventoryGemKind {
  project @0;
  engine @1;
  unknown @2;
}

enum ProjectInventoryLockState {
  fresh @0;
  missing @1;
  stale @2;
  unavailable @3;
}

struct ProjectInventoryLockStatus {
  state @0 :ProjectInventoryLockState;
  path @1 :Text;
  diagnostic @2 :Text;
}

struct ProjectInventoryRegistryCounts {
  buildRules @0 :UInt64;
  nodeTypes @1 :UInt64;
  graphTypes @2 :UInt64;
}

struct ProjectInventoryGem {
  id @0 :Text;
  name @1 :Text;
  version @2 :Text;
  kind @3 :ProjectInventoryGemKind;
  expected @4 :Bool;
  active @5 :Bool;
  capabilities @6 :List(Text);
  expectedPackage @7 :Text;
}

struct ProjectInventoryReport {
  serviceRole @0 :Text;
  lockStatus @1 :ProjectInventoryLockStatus;
  gems @2 :List(ProjectInventoryGem);
  registry @3 :ProjectInventoryRegistryCounts;
  diagnostics @4 :List(Text);
  degraded @5 :Bool;
}

# ADR 0022 vNext process-neutral registry and typed Prefab vocabulary.
struct TypeRegistrySnapshot {
  schemaCatalogHash @0 :Data;
  types @1 :List(ReflectedTypeDescriptor);
}

struct ReflectedTypeDescriptor {
  typePath @0 :Text;
  shortPath @1 :Text;
  kind @2 :ReflectedTypeKind;
  fields @3 :List(ReflectedFieldDescriptor);
  variants @4 :List(ReflectedVariantDescriptor);
  editorAttributes @5 :EditorAttributes;
  typeDataFlags @6 :List(Text);
  applicability @7 :ApplicabilityDescriptor;
  reflectedDefault @8 :Authoring.ReflectedValueEnvelope;
}

struct ApplicabilityDescriptor {
  provides @0 :List(Text);
  requires @1 :List(Text);
  incompatible @2 :List(Text);
  defaultAvailable @3 :Bool;
}

struct ReflectedTypeKind {
  union {
    structKind @0 :Void;
    tupleStruct @1 :Void;
    tuple @2 :Void;
    list @3 :Void;
    array @4 :UInt32;
    map @5 :Void;
    set @6 :Void;
    enumKind @7 :Void;
    optional @8 :Void;
    boolKind @9 :Void;
    signedInteger @10 :UInt8;
    unsignedInteger @11 :UInt8;
    floatKind @12 :UInt8;
    stringKind @13 :Void;
    opaque @14 :Void;
  }
}

struct ReflectedFieldDescriptor {
  name @0 :Text;
  typePath @1 :Text;
  editorAttributes @2 :EditorAttributes;
}

struct ReflectedVariantDescriptor {
  name @0 :Text;
  fields @1 :List(ReflectedFieldDescriptor);
  editorAttributes @2 :EditorAttributes;
}

struct EditorAttributes {
  label @0 :Core.OptionalText;
  description @1 :Core.OptionalText;
  category @2 :Core.OptionalText;
  icon @3 :Core.OptionalText;
  widget @4 :Core.OptionalText;
  range @5 :NumericRange;
  readOnly @6 :Bool;
  hidden @7 :Bool;
  actionIds @8 :List(Text);
  constraints @9 :FieldConstraints;
}

struct FieldConstraints {
  minimumLength @0 :Core.OptionalU32;
  maximumLength @1 :Core.OptionalU32;
  allowedStrings @2 :List(Text);
  allowedVariants @3 :List(Text);
}

struct NumericRange {
  minimum @0 :Core.OptionalText;
  maximum @1 :Core.OptionalText;
  step @2 :Core.OptionalText;
  suffix @3 :Core.OptionalText;
}

struct ReflectedPath {
  componentTypePath @0 :Text;
  segments @1 :List(Segment);

  struct Segment {
    union {
      field @0 :Text;
      variant @1 :Text;
      tupleIndex @2 :UInt32;
      listIndex @3 :UInt32;
    }
  }
}

struct PrefabValueTarget {
  instanceAliasChain @0 :List(Text);
  entityAlias @1 :Text;
  path @2 :ReflectedPath;
}

struct PrefabSourceSnapshot {
  documentVersion @0 :UInt32;
  typeVersions @1 :List(TypeVersion);
  entities @2 :List(Entity);
  hierarchy @3 :List(HierarchyEdge);
  components @4 :List(Component);
  instances @5 :List(Instance);
  overrides @6 :List(Override);
  revision @7 :UInt64;

  struct TypeVersion { typePath @0 :Text; version @1 :UInt32; }
  struct Entity { alias @0 :Text; }
  struct HierarchyEdge { childAlias @0 :Text; parentAlias @1 :Core.OptionalText; }
  struct Component { entityAlias @0 :Text; typePath @1 :Text; sparseValue @2 :Authoring.ReflectedValueEnvelope; }
  struct Instance { alias @0 :Text; sourceAsset @1 :Text; parentEntityAlias @2 :Core.OptionalText; }
  struct Override {
    target @0 :PrefabValueTarget;
    value @1 :Authoring.ReflectedValueEnvelope;
    operation @2 :PrefabOverrideOperation;
  }
}

struct PrefabOverrideOperation {
  union {
    set @0 :Set;
    clear @1 :TargetOnly;
    insert @2 :Insert;
    remove @3 :Remove;
    move @4 :Move;
  }

  struct Set { target @0 :PrefabValueTarget; value @1 :Authoring.ReflectedValueEnvelope; }
  struct TargetOnly { target @0 :PrefabValueTarget; }
  struct Insert { target @0 :PrefabValueTarget; index @1 :UInt32; value @2 :Authoring.ReflectedValueEnvelope; }
  struct Remove { target @0 :PrefabValueTarget; index @1 :UInt32; }
  struct Move { target @0 :PrefabValueTarget; from @1 :UInt32; to @2 :UInt32; }
}

struct PrefabEditCommand {
  union {
    setValue @0 :SetValue;
    listInsert @1 :ListInsert;
    listRemove @2 :ListIndexCommand;
    listMove @3 :ListMove;
    mapInsert @4 :MapInsert;
    mapRemove @5 :MapRemove;
    setVariant @6 :SetVariant;
    addComponent @7 :AddComponent;
    removeComponent @8 :RemoveComponent;
    addEntity @9 :AddEntity;
    removeEntity @10 :RemoveEntity;
    reparentEntity @11 :ReparentEntity;
    addInstance @12 :AddInstance;
    removeInstance @13 :RemoveInstance;
    reparentInstance @14 :ReparentInstance;
    setOverride @15 :SetOverride;
    removeOverride @16 :RemoveOverride;
    clearOverride @17 :RemoveOverride;
    insertOverride @18 :ListInsert;
    removeOverrideItem @19 :ListIndexCommand;
    moveOverride @20 :ListMove;
  }

  struct SetValue { target @0 :PrefabValueTarget; value @1 :Authoring.ReflectedValueEnvelope; }
  struct ListInsert { target @0 :PrefabValueTarget; index @1 :UInt32; value @2 :Authoring.ReflectedValueEnvelope; }
  struct ListIndexCommand { target @0 :PrefabValueTarget; index @1 :UInt32; }
  struct ListMove { target @0 :PrefabValueTarget; from @1 :UInt32; to @2 :UInt32; }
  struct MapInsert { target @0 :PrefabValueTarget; key @1 :Authoring.ReflectedValueEnvelope; value @2 :Authoring.ReflectedValueEnvelope; }
  struct MapRemove { target @0 :PrefabValueTarget; key @1 :Authoring.ReflectedValueEnvelope; }
  struct SetVariant { target @0 :PrefabValueTarget; variantName @1 :Text; value @2 :Authoring.ReflectedValueEnvelope; }
  struct AddComponent { entityAlias @0 :Text; componentTypePath @1 :Text; initialValue @2 :Authoring.ReflectedValueEnvelope; }
  struct RemoveComponent { entityAlias @0 :Text; componentTypePath @1 :Text; }
  struct AddEntity { alias @0 :Text; parentAlias @1 :Core.OptionalText; }
  struct RemoveEntity { alias @0 :Text; }
  struct ReparentEntity { alias @0 :Text; parentAlias @1 :Core.OptionalText; }
  struct AddInstance { alias @0 :Text; sourceAsset @1 :Text; parentEntityAlias @2 :Core.OptionalText; }
  struct RemoveInstance { alias @0 :Text; }
  struct ReparentInstance { alias @0 :Text; parentEntityAlias @1 :Core.OptionalText; }
  struct SetOverride { target @0 :PrefabValueTarget; value @1 :Authoring.ReflectedValueEnvelope; }
  struct RemoveOverride { target @0 :PrefabValueTarget; }
}

enum VNextDiagnosticSeverity {
  info @0;
  warning @1;
  error @2;
}

struct PrefabDiagnostic {
  severity @0 :VNextDiagnosticSeverity;
  code @1 :Text;
  message @2 :Text;
  target @3 :PrefabValueTarget;
}

struct PrefabRpcResult {
  snapshot @0 :PrefabSourceSnapshot;
  diagnostics @1 :List(PrefabDiagnostic);
}

struct TypedActionResult {
  snapshot @0 :PrefabSourceSnapshot;
  changedPaths @1 :List(ReflectedPath);
  diagnostics @2 :List(PrefabDiagnostic);
}

enum SourceSessionCommand {
  open @0;
  save @1;
  saveRecovery @2;
  undo @3;
  redo @4;
  close @5;
  status @6;
}

struct SourceSessionStatus {
  open @0 :Bool;
  revision @1 :UInt64;
  dirty @2 :Bool;
  undoDepth @3 :UInt32;
  redoDepth @4 :UInt32;
}

struct SourceSessionResult {
  status @0 :SourceSessionStatus;
  snapshot @1 :PrefabSourceSnapshot;
  diagnostics @2 :List(PrefabDiagnostic);
}

# Generic source-authoring session contract. Project-host owns the optimistic
# session/history; Asset Processor remains the only authority that reads,
# edits, restores, and saves workspace source files.
struct SourceAuthoringSessionCommand {
  union {
    open @0 :Void;
    apply @1 :SourceAuthoringApply;
    undo @2 :Void;
    redo @3 :Void;
    close @4 :Void;
    status @5 :Void;
  }
}

struct SourceAuthoringApply {
  operation @0 :Authoring.SourceFileEditOperation;
}

struct SourceAuthoringSessionRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  source @2 :Asset.WorkspaceSourceFileRef;
  expectedRevision @3 :UInt64;
  command @4 :SourceAuthoringSessionCommand;
}

struct SourceAuthoringSessionStatus {
  open @0 :Bool;
  revision @1 :UInt64;
  undoDepth @2 :UInt32;
  redoDepth @3 :UInt32;
}

enum SourceAuthoringFailureCode {
  unavailable @0;
  notOpen @1;
  revisionConflict @2;
  historyEmpty @3;
  transaction @4;
  sourceMismatch @5;
}

struct SourceAuthoringFailure {
  code @0 :SourceAuthoringFailureCode;
  detail @1 :Text;
  expectedRevision @2 :UInt64;
  currentRevision @3 :UInt64;
}

struct SourceAuthoringSessionResult {
  status @0 :SourceAuthoringSessionStatus;
  union {
    snapshot @1 :Asset.SourceFileEditSnapshot;
    failure @2 :SourceAuthoringFailure;
    closed @3 :Void;
  }
}

interface ProjectHost {
  # ADR-0031: the pre-1.0 lockstep contract uses dense live method ordinals;
  # reserved tombstones are intentionally forbidden.
  runtimeLaunchSnapshot @0 (request :RuntimeLaunchSnapshotRequest) -> (snapshot :Core.SideChannelHandle);
  nodeTypeCatalog @1 (capability :Core.Capability) -> (snapshot :Core.SideChannelHandle);
  applyGraphCommands @2 (
    capability :Core.Capability,
    batch :Core.SideChannelHandle
  ) -> (status :Core.SideChannelHandle);
  createGraphDocument @3 (
    capability :Core.Capability,
    document :DocumentId,
    graphType :Text
  ) -> (snapshot :Core.SideChannelHandle);
  graphDocumentSnapshot @4 (
    capability :Core.Capability,
    document :DocumentId
  ) -> (snapshot :Core.SideChannelHandle);
  saveGraphDocument @5 (capability :Core.Capability, document :DocumentId) -> (
    revision :DocumentRevision,
    saved :SavedDocument
  );
  graphTypeCatalog @6 (capability :Core.Capability) -> (snapshot :Core.SideChannelHandle);
  resolveNodeSourceLink @7 (request :NodeSourceLinkRequest) -> (target :NodeSourceLinkTarget);
  projectInventory @8 (capability :Core.Capability) -> (report :ProjectInventoryReport);
  gamedataCatalog @9 (capability :Core.Capability) -> (snapshot :Core.SideChannelHandle);
  health @10 () -> (health :Core.ServiceHealth);
  typeRegistrySnapshot @11 (capability :Core.Capability) -> (snapshot :TypeRegistrySnapshot);
  prefabSourceSnapshot @12 (capability :Core.Capability, sourcePath :Text) -> (result :PrefabRpcResult);
  applyPrefabEditCommand @13 (
    capability :Core.Capability,
    sourcePath :Text,
    expectedRevision :UInt64,
    command :PrefabEditCommand
  ) -> (result :PrefabRpcResult);
  invokeTypedAction @14 (
    capability :Core.Capability,
    sourcePath :Text,
    expectedRevision :UInt64,
    target :PrefabValueTarget,
    actionId :Text
  ) -> (result :TypedActionResult);
  prefabDiagnostics @15 (capability :Core.Capability, sourcePath :Text) -> (diagnostics :List(PrefabDiagnostic));
  sourceSessionLifecycle @16 (
    capability :Core.Capability,
    sourcePath :Text,
    command :SourceSessionCommand,
    expectedRevision :UInt64
  ) -> (result :SourceSessionResult);
  sourceAuthoringSession @17 (request :SourceAuthoringSessionRequest) -> (result :SourceAuthoringSessionResult);
}
