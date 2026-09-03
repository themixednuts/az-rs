@0xdb6f9c0d89a3fef1;

using Core = import "/azoth/core.capnp";
using Authoring = import "/azoth/authoring.capnp";

enum JobStatus {
  queued @0;
  leased @1;
  succeeded @2;
  failed @3;
}

enum AttemptStatus {
  leased @0;
  succeeded @1;
  failed @2;
  abandoned @3;
}

enum JobDependencyKind {
  order @0;
  fingerprint @1;
  orderOnly @2;
}

enum SourceRelation {
  sourceToSource @0;
  jobToJob @1;
  sourceLikeMatch @2;
}

enum WorkspaceEntryDiff {
  clean @0;
  added @1;
  modified @2;
  deleted @3;
  conflicted @4;
}

enum AssetRootScope {
  # Browser-facing asset roots only. Excludes the project-source root that
  # represents the whole workspace for source navigation and source control.
  browserAssets @0;
  # Every registered source root in the workspace.
  all @1;
}

enum AssetProcessorEventKind {
  sourceRecorded @0;
  sourceCreated @1;
  jobCompleted @2;
  sourceDeleted @3;
  sourceMoved @4;
  sourceReprocessed @5;
  sourceReconciled @6;
}

struct JobOwner {
  union {
    plan @0 :Void;
    build @1 :Data;
  }
}

enum AssetBuilderPatternKind {
  wildcard @0;
  regex @1;
}

enum CatalogPathRegistration {
  # This product claims its path and aliases for by-path catalog lookup.
  registered @0;
  # This remains a real AssetId and physical payload, but does not claim
  # by-path lookup. Used when multiple ids share one product path.
  assetIdOnly @1;
}

# Operational lease payload. Diagnostics use JobInspection and never expose
# source payload side channels.
struct LeasedAssetJob {
  attemptId @0 :Int64;
  workspaceId @1 :Int64;
  owner @2 :JobOwner;
  jobKey @3 :Text;
  platform @4 :Text;
  ordinal @5 :Int64;
  stagingRoot @6 :Text;
  sourceGuid @7 :Data;
  sourcePath @8 :Text;
  sourceRoot @9 :Text;
  union {
    noSourcePayload @10 :Void;
    sourcePayload @11 :Core.SideChannelHandle;
  }
  sourceSchemaType @12 :Core.OptionalText;
  preservedSourceSubId @13 :Core.OptionalU32;
}

struct InspectJobRequest {
  capability @0 :Core.Capability;
  selector :union {
    jobId @1 :Int64;
    attemptId @2 :Int64;
  }
}

struct JobRecord {
  jobId @0 :Int64;
  workspaceId @1 :Int64;
  sourceGuid @2 :Data;
  sourcePath @3 :Text;
  sourceRoot @4 :Text;
  sourceSchemaType @5 :Core.OptionalText;
  owner @6 :JobOwner;
  key @7 :Text;
  platform @8 :Text;
  status @9 :JobStatus;
  ready @10 :Bool;
  attempts @11 :Int64;
}

struct JobAttemptRecord {
  attemptId @0 :Int64;
  jobId @1 :Int64;
  ordinal @2 :Int64;
  status @3 :AttemptStatus;
  owner @4 :Core.OptionalText;
  staging @5 :Core.OptionalText;
  finishedUnixMs @6 :Core.OptionalI64;
  errorCount @7 :Int64;
  warningCount @8 :Int64;
}

struct JobActivity {
  job @0 :JobRecord;
  union {
    noAttempt @1 :Void;
    attempt @2 :JobAttemptRecord;
  }
}

struct JobProductEdgeRecord {
  productEdgeId @0 :Int64;
  productId @1 :Int64;
  assetGuid @2 :Data;
  subId @3 :Int64;
  flags @4 :Int64;
}

struct JobProductRecord {
  productId @0 :Int64;
  jobId @1 :Int64;
  path @2 :Text;
  assetType @3 :Data;
  subId @4 :Int64;
  productFormat @5 :Text;
  productFormatVersion @6 :UInt32;
  catalogPathRegistration @7 :CatalogPathRegistration;
  contentHash @8 :Text;
  byteLength @9 :Int64;
  aliases @10 :List(Text);
  edges @11 :List(JobProductEdgeRecord);
}

struct JobDependencyTarget {
  union {
    guid @0 :Data;
    path @1 :Text;
  }
}

struct JobDependencyRecord {
  jobEdgeId @0 :Int64;
  jobId @1 :Int64;
  target @2 :JobDependencyTarget;
  key @3 :Text;
  platform @4 :Text;
  kind @5 :JobDependencyKind;
}

struct JobInspection {
  job @0 :JobRecord;
  union {
    noAttempt @1 :Void;
    attempt @2 :JobAttemptRecord;
  }
  products @3 :List(JobProductRecord);
  dependencies @4 :List(JobDependencyRecord);
}

struct InspectJobResult {
  union {
    none @0 :Void;
    inspection @1 :JobInspection;
  }
}

struct AssetBuilderPatternDescriptor {
  kind @0 :AssetBuilderPatternKind;
  pattern @1 :Text;
}

struct AssetBuilderDescriptor {
  name @0 :Text;
  builderGuid @1 :Data;
  version @2 :UInt32;
  patterns @3 :List(AssetBuilderPatternDescriptor);
  sourceSchemaTypes @4 :List(Text);
  # Builder-local analysis identity. Mirrors Lumberyard/O3DE
  # AssetBuilderDesc::m_analysisFingerprint.
  analysisFingerprint @5 :Text;
}

struct SourceSchemaDescriptor {
  schemaType @0 :Text;
  owner @1 :Text;
  label @2 :Text;
  category @3 :Text;
  authoring @4 :SourceSchemaAuthoring;
  fileTemplates @5 :List(SourceFileTemplateDescriptor);
}

struct SourceFileWorkflowDescriptor {
  defaultPathPrefix @0 :Text;
  extensions @1 :List(Text);
  canCreate @2 :Bool;
  canEdit @3 :Bool;
  sourceRoot @4 :Text;
}

struct SourceFileTemplateDescriptor {
  owner @0 :Text;
  sourcePath @1 :Text;
  label @2 :Text;
  description @3 :Text;
}

struct SourceSchemaAuthoring {
  union {
    file @0 :SourceFileWorkflowDescriptor;
    projectDocument @1 :Text;
  }
}

struct AssetBuilderCatalogRequest {
  capability @0 :Core.Capability;
}

struct ProductFormatDescriptor {
  id @0 :Text;
  currentVersion @1 :UInt32;
  owner @2 :Text;
}

struct AssetBuilderCatalogResult {
  builders @0 :List(AssetBuilderDescriptor);
  sourceSchemas @1 :List(SourceSchemaDescriptor);
  productFormats @2 :List(ProductFormatDescriptor);
}

struct WorkspaceEntry {
  entryId @0 :Int64;
  workspaceId @1 :Int64;
  rootId @2 :Int64;
  sourcePath @3 :Text;
  contentHash @4 :Text;
  diff @5 :WorkspaceEntryDiff;
  diagnosticsCount @6 :Int64;
  updatedUnixMs @7 :Int64;
  jobs @8 :List(JobActivity);
  schemaType @9 :Core.OptionalText;
  assetGuid @10 :Data;
}

struct WorkspaceEntryPageRequest {
  capability @0 :Core.Capability;
  # Optional cursor from a previous result. When present, this must be a
  # positive entryId returned by nextAfterEntryId.
  afterEntryId @1 :Core.OptionalI64;
  # Bounded page size. Zero is invalid; callers should follow cursors for
  # large views instead of asking the service for an unbounded page.
  pageSize @2 :UInt32;
  rootScope @3 :AssetRootScope;
}

struct WorkspaceEntryPageResult {
  entries @0 :List(WorkspaceEntry);
  nextAfterEntryId @1 :Core.OptionalI64;
}

struct AssetProcessorEvent {
  seq @0 :UInt64;
  kind @1 :AssetProcessorEventKind;
  eventUnixMs @2 :Int64;
  entry @3 :WorkspaceEntry;
}

struct AssetProcessorEventSubscriptionRequest {
  capability @0 :Core.Capability;
}

struct AssetProcessorEventSubscriptionResult {
  subscribed @0 :Bool;
  # Captured after registering the sink. Clients publish this canonical
  # snapshot before forwarding any buffered sink updates.
  initialHealth @1 :Core.ServiceHealth;
}

interface AssetProcessorEventSink {
  update @0 (event :AssetProcessorEvent) -> ();
}

struct WorkspaceRoot {
  workspaceRootId @0 :Int64;
  workspaceId @1 :Int64;
  rootId @2 :Int64;
  ownerId @3 :Text;
  sourceRoot @4 :Text;
  displayName @5 :Text;
  portableKey @6 :Text;
  outputPrefix @7 :Text;
  isRoot @8 :Bool;
  declaredRootId @9 :Text;
  mount @10 :Text;
  recursive @11 :Bool;
  watch @12 :Bool;
  writable @13 :Bool;
}

struct WorkspaceSnapshot {
  workspaceId @0 :Int64;
  workspaceRoot @1 :Text;
  branch @2 :Text;
  createdUnixMs @3 :Int64;
  updatedUnixMs @4 :Int64;
  projectId @5 :Text;
  roots @6 :List(WorkspaceRoot);
}

struct WorkspaceSnapshotRequest {
  capability @0 :Core.Capability;
  rootScope @1 :AssetRootScope;
}

struct WorkspaceSnapshotResult {
  union {
    none @0 :Void;
    snapshot @1 :WorkspaceSnapshot;
  }
}

struct CatalogProductDependency {
  # Runtime product dependency edge carried with a current product.
  #
  # `assetGuid`/`subId` are the target product identity; `assetType` is the
  # target product's native asset type when the target is present in the same
  # workspace/platform projection. `hint` is a
  # diagnostic `@assets@/...` address only, never a resolver.
  assetGuid @0 :Data;
  subId @1 :Int64;
  union {
    noAssetType @2 :Void;
    assetType @4 :Data;
  }
  hint @3 :Core.OptionalText;
}

struct CatalogProductEntry {
  jobId @0 :Int64;
  productId @1 :Int64;
  assetGuid @2 :Data;
  sourcePath @3 :Text;
  builderGuid @4 :Data;
  jobKey @5 :Text;
  platform @6 :Text;
  productPath @7 :Text;
  assetType @8 :Data;
  subId @9 :Int64;
  contentHash @10 :Text;
  byteLength @11 :Int64;
  productFormat @12 :Text;
  productFormatVersion @13 :UInt32;
  dependencies @14 :List(CatalogProductDependency);
  catalogAliases @15 :List(Text);
  catalogPathRegistration @16 :CatalogPathRegistration;
}

struct CatalogProductsRequest {
  capability @0 :Core.Capability;
  platform @1 :Text;
}

struct CatalogProductsResult {
  entries @0 :List(CatalogProductEntry);
}

struct ReleaseContentAssetId {
  assetGuid @0 :Data;
  subId @1 :UInt32;
}

struct ReleaseContentTarget {
  union {
    assetCatalog @0 :Void;
    productAsset @1 :ReleaseContentAssetId;
  }
}

struct ReleaseContentReadRequest {
  capability @0 :Core.Capability;
  platform @1 :Text;
  target @2 :ReleaseContentTarget;
}

struct ReleaseContentProduct {
  assetGuid @0 :Data;
  subId @1 :UInt32;
  productPath @2 :Text;
  productFormat @3 :Text;
  productFormatVersion @4 :UInt32;
  byteLength @5 :UInt64;
  contentHash @6 :Data;
  payload @7 :Core.SideChannelHandle;
}

struct ReleaseContentReadResult {
  union {
    none @0 :Void;
    assetCatalog @1 :Core.SideChannelHandle;
    product @2 :ReleaseContentProduct;
  }
}

struct SourceAssetRecordRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  workspaceRootId @2 :Int64;
  sourcePath @3 :Text;
  schemaType @4 :Core.OptionalText;
  contentHash @5 :Data;
  changedUnixMs @6 :Int64;
  diagnosticsCount @7 :Int64;
  ownerId @8 :Text;
}

struct SourceAssetRecordResult {
  assetGuid @0 :Data;
  entry @1 :WorkspaceEntry;
}

struct SourceFileCreateRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  sourcePath @2 :Text;
  schemaType @3 :Text;
  changedUnixMs @4 :Int64;
  sourceRoot @7 :Text;
  union {
    defaultTemplate @5 :Void;
    payload @6 :Core.SideChannelHandle;
  }
}

struct SourceFileCreateResult {
  record @0 :SourceAssetRecordResult;
}

# Canonical workspace-file identity for structured source editing. The source
# root's canonical portable key and relative path select the authoritative
# file; schemaType selects its composed edit codec.
struct WorkspaceSourceFileRef {
  sourceRootKey @0 :Text;
  sourcePath @1 :Text;
  schemaType @2 :Text;
}

struct SourceFileEditRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  source @2 :WorkspaceSourceFileRef;
  expectedSourceFingerprint @3 :Data;
  operation @4 :Authoring.SourceFileEditOperation;
}

struct SourceFileOpenRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  source @2 :WorkspaceSourceFileRef;
}

# Trusted project-host replay of one previously returned canonical document.
# The payload remains structured and codec-validated; source bytes never cross
# the public authoring boundary.
struct SourceFileRestoreRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  source @2 :WorkspaceSourceFileRef;
  expectedSourceFingerprint @3 :Data;
  document @4 :Authoring.SourceFileEditDocument;
}

# Asset Processor -> project asset-worker codec execution. File bytes travel
# only through verified side channels; the public edit result below never
# exposes them to editor callers.
struct SourceFileCodecRequest {
  source @0 :WorkspaceSourceFileRef;
  authoritativeSource @1 :Core.SideChannelHandle;
  operation @2 :Authoring.SourceFileCodecOperation;
  # AP-owned destination for any saved source. The worker writes exactly this
  # location and returns a verified SideChannelHandle for the resulting file.
  outputDestination @3 :SourceFileCodecOutputDestination;
}

struct SourceFileCodecOutputDestination {
  locator @0 :Text;
  platform @1 :Text;
  capability @2 :Core.Capability;
}

struct SourceFileCodecResult {
  document @0 :Authoring.SourceFileEditDocument;
  union {
    unchanged @1 :Void;
    savedSource @2 :Core.SideChannelHandle;
  }
}

struct SourceFileEditSnapshot {
  source @0 :WorkspaceSourceFileRef;
  sourceFingerprint @1 :Data;
  document @2 :Authoring.SourceFileEditDocument;
}

struct SourceFileEditResult {
  snapshot @0 :SourceFileEditSnapshot;
}

struct SourceFileOpenResult {
  snapshot @0 :SourceFileEditSnapshot;
}

struct SourceFileRestoreResult {
  snapshot @0 :SourceFileEditSnapshot;
}

struct SourceFileDeleteRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  sourceRoot @2 :Text;
  sourcePath @3 :Text;
  changedUnixMs @4 :Int64;
}

struct SourceFileDeleteResult {
  record @0 :SourceAssetRecordResult;
}

struct SourceFileMoveRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  sourceRoot @2 :Text;
  fromSourcePath @3 :Text;
  toSourcePath @4 :Text;
  changedUnixMs @5 :Int64;
}

struct SourceFileMoveResult {
  record @0 :SourceAssetRecordResult;
  oldSourcePath @1 :Text;
}

struct ForceReprocessAssetRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  sourceRoot @2 :Text;
  sourcePath @3 :Text;
}

struct ForceReprocessAssetResult {
  record @0 :SourceAssetRecordResult;
  enqueuedJobs @1 :UInt32;
}

struct ReconcileAssetSourcesRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  rootScope @2 :AssetRootScope;
}

struct ReconcileAssetSourcesResult {
  sourceRootCount @0 :UInt32;
  recordedSourceAssetCount @1 :UInt32;
  deletedSourceAssetCount @2 :UInt32;
}

struct SourceDependentSource {
  sourceEdgeId @0 :Int64;
  sourcePath @1 :Text;
  builderGuid @2 :Data;
  relation @3 :SourceRelation;
}

struct SourceDependentJob {
  jobEdgeId @0 :Int64;
  jobId @1 :Int64;
  latestAttemptId @2 :Core.OptionalI64;
  sourcePath @3 :Text;
  owner @4 :JobOwner;
  jobKey @5 :Text;
  platform @6 :Text;
  dependencyJobKey @7 :Text;
  dependencyPlatform @8 :Text;
  kind @9 :JobDependencyKind;
  productPaths @10 :List(Text);
}

struct SourceDependentsRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  sourceRoot @2 :Text;
  sourcePath @3 :Text;
}

struct SourceDependentsResult {
  sourcePath @0 :Text;
  sourceDependents @1 :List(SourceDependentSource);
  jobDependents @2 :List(SourceDependentJob);
}

struct LeaseAssetJobRequest {
  capability @0 :Core.Capability;
  leaseOwner @1 :Text;
  leaseDurationMs @2 :UInt64;
  stagingRoot @3 :Core.OptionalText;
}

struct LeaseAssetJobResult {
  leased @0 :LeasedAssetJob;
  # Process-local dispatcher fence. It is not a durable routing or identity key.
  grantKey @1 :Data;
}

struct RenewAssetJobLeaseRequest {
  capability @0 :Core.Capability;
  assetJobAttemptId @1 :Int64;
  leaseOwner @2 :Text;
  # Extends by the duration admitted on lease; workers cannot change policy.
  grantKey @3 :Data;
}

struct ProductManifest {
  schemaVersion @0 :UInt32;
  generatedUnixMs @1 :UInt64;
  products @2 :List(Product);

  struct Product {
    productPath @0 :Text;
    assetType @1 :Data;
    subId @2 :UInt32;
    stagedPath @3 :Text;
    contentHash @4 :Data;
    byteLength @5 :UInt64;
    productFormat @6 :Text;
    productFormatVersion @7 :UInt32;
    dependencies @8 :List(Dependency);
    catalogAliases @9 :List(Text);
    catalogPathRegistration @10 :CatalogPathRegistration;
  }

  struct Dependency {
    assetGuid @0 :Data;
    subId @1 :UInt32;
    flags @2 :UInt32;
  }
}

struct CompleteAssetJobAttemptRequest {
  capability @0 :Core.Capability;
  assetJobAttemptId @1 :Int64;
  leaseOwner @2 :Text;
  status @3 :AttemptStatus;
  finishedUnixMs @4 :Int64;
  errorCount @5 :Int64;
  warningCount @6 :Int64;
  union {
    noProductManifest @7 :Void;
    productManifest @8 :Core.SideChannelHandle;
  }
  grantKey @9 :Data;
}

struct AssetProcessingStatusRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  platform @2 :Text;
}

struct AssetProcessingStatusResult {
  queued @0 :UInt32;
  leased @1 :UInt32;
  failed @2 :UInt32;
  inFlightSweeps @3 :UInt32;
}

struct PublishAssetCatalogRequest {
  capability @0 :Core.Capability;
  sessionId @1 :Text;
  platform @2 :Text;
}

struct PublishAssetCatalogResult {
  catalogPath @0 :Text;
  entryCount @1 :UInt32;
  reused @2 :Bool;
}

# Worker → host registration of the project-linked builder/schema catalog.
# The engine-owned asset-processor host has zero project gem linkage, so it
# cannot discover builders via inventory; the project asset-worker publishes
# this catalog after connecting and after a protocol-version handshake.
struct PublishBuilderCatalogRequest {
  capability @0 :Core.Capability;
  protocol @1 :Core.ProtocolVersion;
  catalog @2 :AssetBuilderCatalogResult;
}

struct PublishBuilderCatalogResult {
  accepted @0 :Bool;
}

interface AssetProcessor {
  lease @0 (request :LeaseAssetJobRequest) -> (result :LeaseAssetJobResult);
  renewLease @1 (request :RenewAssetJobLeaseRequest) -> (renewed :Bool);
  completeAttempt @2 (request :CompleteAssetJobAttemptRequest) -> (completed :Bool);
  inspectJob @3 (request :InspectJobRequest) -> (result :InspectJobResult);
  workspaceEntryPage @4 (request :WorkspaceEntryPageRequest) -> (result :WorkspaceEntryPageResult);
  recordSourceAsset @5 (request :SourceAssetRecordRequest) -> (result :SourceAssetRecordResult);
  workspaceSnapshot @6 (request :WorkspaceSnapshotRequest) -> (result :WorkspaceSnapshotResult);
  builderCatalog @7 (request :AssetBuilderCatalogRequest) -> (result :AssetBuilderCatalogResult);
  catalogProducts @8 (request :CatalogProductsRequest) -> (result :CatalogProductsResult);
  releaseContent @9 (request :ReleaseContentReadRequest) -> (result :ReleaseContentReadResult);
  createSourceFile @10 (request :SourceFileCreateRequest) -> (result :SourceFileCreateResult);
  subscribeEvents @11 (request :AssetProcessorEventSubscriptionRequest, sink :AssetProcessorEventSink) -> (result :AssetProcessorEventSubscriptionResult);
  deleteSourceFile @12 (request :SourceFileDeleteRequest) -> (result :SourceFileDeleteResult);
  moveSourceFile @13 (request :SourceFileMoveRequest) -> (result :SourceFileMoveResult);
  sourceDependents @14 (request :SourceDependentsRequest) -> (result :SourceDependentsResult);
  forceReprocessAsset @15 (request :ForceReprocessAssetRequest) -> (result :ForceReprocessAssetResult);
  reconcileAssetSources @16 (request :ReconcileAssetSourcesRequest) -> (result :ReconcileAssetSourcesResult);
  health @17 () -> (health :Core.ServiceHealth);
  processingStatus @18 (request :AssetProcessingStatusRequest) -> (result :AssetProcessingStatusResult);
  publishAssetCatalog @19 (request :PublishAssetCatalogRequest) -> (result :PublishAssetCatalogResult);
  publishBuilderCatalog @20 (request :PublishBuilderCatalogRequest, codec :SourceFileCodec) -> (result :PublishBuilderCatalogResult);
  openSourceFile @21 (request :SourceFileOpenRequest) -> (result :SourceFileOpenResult);
  editSourceFile @22 (request :SourceFileEditRequest) -> (result :SourceFileEditResult);
  restoreSourceFile @23 (request :SourceFileRestoreRequest) -> (result :SourceFileRestoreResult);
  # Parks until the source graph has a coherent idle processing aggregate.
  # It is intentionally distinct from processingStatus, which is a snapshot.
  waitForIdle @24 (request :AssetProcessingStatusRequest) -> (result :AssetProcessingStatusResult);
}

interface SourceFileCodec {
  execute @0 (request :SourceFileCodecRequest) -> (result :SourceFileCodecResult);
}
