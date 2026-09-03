@0xcee076d5c7962bc2;

using Core = import "/azoth/core.capnp";
using Project = import "/azoth/project.capnp";
using Asset = import "/azoth/asset.capnp";
using Runtime = import "/azoth/runtime.capnp";

struct CreateSessionRequest {
  name @0 :Text;
}

enum SessionState {
  preparing @0;
  active @1;
  failedPreserved @2;
  removed @3;
}

enum ServiceProcessState {
  planned @0;
  starting @1;
  running @2;
  exited @3;
  failed @4;
}

enum ServiceLogStream {
  stdout @0;
  stderr @1;
  structured @2;
}

struct ServiceProgramArtifact {
  byteLength @0 :UInt64;
  modifiedUnixNs @1 :UInt64;
  hasFileIdentity @2 :Bool;
  fileSystemId @3 :UInt64;
  fileId @4 :UInt64;
}

struct ServiceProcessRecord {
  serviceName @0 :Text;
  role @1 :Core.ServiceRole;
  # UUIDv7 label for this planned launch; never a control key.
  run @2 :Data;
  endpoint @3 :Core.Endpoint;
  program @4 :Text;
  cwd @5 :Text;
  args @6 :List(Text);
  stdoutLog @7 :Text;
  stderrLog @8 :Text;
  state @9 :ServiceProcessState;
  pid @10 :UInt32;
  exitCode @11 :Core.OptionalI64;
  failure @12 :Text;
  plannedUnixMs @13 :UInt64;
  updatedUnixMs @14 :UInt64;
  startedUnixMs @15 :UInt64;
  exitedUnixMs @16 :UInt64;
  ownerId @17 :Text;
  structuredLog @18 :Text;
  ownerRoot @19 :Text;
  programArtifact @20 :ServiceProgramArtifact;
  # Platform start token for `pid`. Zero means the record predates the token.
  processStartTime @21 :UInt64;
  # The one retained predecessor log, if any.
  previousRun @22 :Core.OptionalUuid;
}

struct SessionManifest {
  id @0 :Core.SessionId;
  slug @1 :Text;
  projectRoot @2 :Text;
  workspaceRoot @3 :Text;
  runDir @4 :Text;
  state @5 :SessionState;
  services @6 :List(Core.ServiceDescriptor);
  processes @7 :List(ServiceProcessRecord);
  projectId @8 :Text;
}

struct SessionWorkspaceStatus {
  manifest @0 :SessionManifest;
  failureReason @1 :Text;
}

struct SessionSupervisorIdentity {
  processId @0 :UInt32;
  processStartTime @1 :UInt64;
  descriptor @2 :Core.ServiceDescriptor;
}

struct SaveGraphDocumentRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
  document @2 :Project.DocumentId;
}

struct SaveGraphDocumentResult {
  saved @0 :Project.SavedDocument;
  assetRecord @1 :Asset.SourceAssetRecordResult;
}

struct StopServicesRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
  reason @2 :Text;
}

struct StopServicesResult {
  status @0 :SessionWorkspaceStatus;
  stopped @1 :List(Text);
  skipped @2 :List(Text);
}

struct StartServicesRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
  reason @2 :Text;
  # Empty means start every planned/cleanly-exited service.
  serviceNames @3 :List(Text);
}

struct StartServicesResult {
  status @0 :SessionWorkspaceStatus;
  started @1 :List(Text);
  skipped @2 :List(Text);
}

struct SessionSupervisorEvent {
  # Observation-only sequence. A reconnect starts with a fresh snapshot;
  # clients must never use this as a control or revision token.
  sequence @0 :UInt64;
  status @1 :SessionWorkspaceStatus;
}

struct SessionSupervisorEventSubscriptionRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
}

struct SessionSupervisorEventSubscriptionResult {
  subscribed @0 :Bool;
  initialSequence @1 :UInt64;
  initialStatus @2 :SessionWorkspaceStatus;
}

interface SessionSupervisorEventSink {
  update @0 (event :SessionSupervisorEvent) -> ();
}

struct ServiceLogRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
  serviceName @2 :Text;
  run @3 :Core.OptionalUuid;
  stream @4 :ServiceLogStream;
  all @5 :Bool;
  tailLines @6 :UInt32;
  offset @7 :Core.OptionalU64;
}

struct ServiceLogResult {
  sessionSlug @0 :Text;
  serviceName @1 :Text;
  run @2 :Data;
  stream @3 :ServiceLogStream;
  path @4 :Text;
  lines @5 :List(Text);
  nextOffset @6 :UInt64;
}

struct ExecCommandRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
  program @2 :Text;
  args @3 :List(Text);
  maxOutputBytes @4 :UInt32;
}

struct ExecCommandResult {
  success @0 :Bool;
  exited @1 :Bool;
  exitCode @2 :Int32;
  stdout @3 :Text;
  stderr @4 :Text;
  stdoutTruncated @5 :Bool;
  stderrTruncated @6 :Bool;
}

struct RuntimeAssetPackageRootsRequest {
  capability @0 :Core.Capability;
  slug @1 :Text;
}

struct RuntimeAssetPackageRootsResult {
  roots @0 :List(Runtime.RuntimeAssetPackageRoot);
}

interface SessionSupervisor {
  create @0 (request :CreateSessionRequest, capability :Core.Capability)
      -> (manifest :SessionManifest);
  list @1 (capability :Core.Capability)
      -> (sessions :List(SessionManifest));
  status @2 (slug :Text, capability :Core.Capability)
      -> (status :SessionWorkspaceStatus);
  preserveFailed @3 (slug :Text, reason :Text, capability :Core.Capability)
      -> (manifest :SessionManifest);
  registerService @4 (slug :Text, descriptor :Core.ServiceDescriptor, capability :Core.Capability)
      -> (manifest :SessionManifest);
  resolveService @5 (slug :Text, id :Core.ServiceId, role :Core.ServiceRole, capability :Core.Capability)
      -> (descriptor :Core.ServiceDescriptor);
  stopServices @6 (request :StopServicesRequest) -> (result :StopServicesResult);
  recover @7 (slug :Text, force :Bool, capability :Core.Capability)
      -> (manifest :SessionManifest);
  serviceLog @8 (request :ServiceLogRequest) -> (result :ServiceLogResult);
  execCommand @9 (request :ExecCommandRequest) -> (result :ExecCommandResult);
  startServices @10 (request :StartServicesRequest) -> (result :StartServicesResult);
  runtimeAssetPackageRoots @11 (request :RuntimeAssetPackageRootsRequest)
      -> (result :RuntimeAssetPackageRootsResult);
  saveGraphDocument @12 (request :SaveGraphDocumentRequest) -> (result :SaveGraphDocumentResult);
  health @13 () -> (health :Core.ServiceHealth);
  supervisionIdentity @14 () -> (identity :SessionSupervisorIdentity);
  retire @15 (slug :Text, capability :Core.Capability) -> (manifest :SessionManifest);
  subscribeEvents @16 (request :SessionSupervisorEventSubscriptionRequest, sink :SessionSupervisorEventSink)
      -> (result :SessionSupervisorEventSubscriptionResult);
  # Clients invoke this only after receiving stopServices' durable terminal
  # result. A streaming result makes this connection-terminal command truly
  # one-way: send queues the call without creating a cancellable reply promise.
  shutdownSupervisor @17 (slug :Text, reason :Text, capability :Core.Capability) -> stream;
}
