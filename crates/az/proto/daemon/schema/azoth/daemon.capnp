@0xa871ca062d49b47f;

using Core = import "/azoth/core.capnp";
using Session = import "/azoth/session.capnp";

struct ProjectRecord {
  projectId @0 :Text;
  name @1 :Text;
  root @2 :Text;
  manifestPath @3 :Text;
  engineVersion @4 :Text;
}

struct ProjectResult {
  union {
    none @0 :Void;
    project @1 :ProjectRecord;
  }
}

struct RegisterProjectRequest {
  capability @0 :Core.Capability;
  project @1 :ProjectRecord;
}

struct RegisterProjectRootRequest {
  capability @0 :Core.Capability;
  root @1 :Text;
}

struct ResolveProjectRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
}

struct ListProjectsRequest {
  capability @0 :Core.Capability;
}

struct ListProjectsResult {
  projects @0 :List(ProjectRecord);
  # Stable bootstrap field: listProjects @1 predates ADR-0031 renumbering, so
  # clients check this before invoking any method whose ordinal moved.
  protocolVersion @1 :Core.ProtocolVersion;
}

struct RegisterSessionSupervisorRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionSlug @2 :Text;
  descriptor @3 :Core.ServiceDescriptor;
}

struct UnregisterSessionSupervisorRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionSlug @2 :Text;
  descriptor @3 :Core.ServiceDescriptor;
}

struct UnregisterSessionSupervisorResult {
  removed @0 :Bool;
}

struct ResolveSessionSupervisorRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionSlug @2 :Text;
}

struct ListSessionSupervisorsRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
}

struct SessionSupervisorDescriptor {
  sessionSlug @0 :Text;
  descriptor @1 :Core.ServiceDescriptor;
}

struct ListSessionSupervisorsResult {
  supervisors @0 :List(SessionSupervisorDescriptor);
}

struct SessionSupervisorResult {
  union {
    none @0 :Void;
    descriptor @1 :Core.ServiceDescriptor;
  }
}

struct EnsureProjectSessionRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionName @2 :Text;
}

struct ProjectSessionResult {
  manifest @0 :Session.SessionManifest;
  created @1 :Bool;
}

struct PrepareProjectSessionServicesRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionSlug @2 :Text;
  endpointKind @3 :Core.EndpointKind;
  skipBuild @4 :Bool;
  serviceNames @5 :List(Text);
  otlpEndpoint @6 :Text;
  recover @7 :Bool;
}

struct ProjectSessionServicesResult {
  manifest @0 :Session.SessionManifest;
  serviceNames @1 :List(Text);
  buildCommandCount @2 :UInt32;
  preparedProcessCount @3 :UInt32;
  built @4 :Bool;
}

struct EnsureProjectSessionServicesRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionName @2 :Text;
  endpointKind @3 :Core.EndpointKind;
  skipBuild @4 :Bool;
  startServiceNames @5 :List(Text);
  timeoutMs @6 :UInt64;
  daemonEndpoint @7 :Core.Endpoint;
}

struct ProjectSessionServicesStartResult {
  manifest @0 :Session.SessionManifest;
  serviceNames @1 :List(Text);
  runningServiceNames @2 :List(Text);
  buildCommandCount @3 :UInt32;
  preparedProcessCount @4 :UInt32;
  built @5 :Bool;
  sessionCreated @6 :Bool;
  supervisor @7 :Core.ServiceDescriptor;
  reusedSupervisor @8 :Bool;
  startRequested @9 :Bool;
  sessiondPid @10 :UInt32;
  logPath @11 :Text;
}

struct PlanProjectBuildRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  profile @2 :Text;
  targetTriple @3 :Text;
  packageSelectors @4 :List(Text);
}

struct ProjectBuildCommand {
  targetName @0 :Text;
  program @1 :Text;
  cwd @2 :Text;
  args @3 :List(Text);
  ownerId @4 :Text;
  ownerRoot @5 :Text;
  cargoTargetDir @6 :Core.OptionalText;
}

struct ProjectBuildPackageProfile {
  name @0 :Text;
  assetPlatform @1 :Text;
  cargoProfile @2 :Text;
  container @3 :Text;
  compression @4 :Text;
  oodleCompressor @5 :Core.OptionalText;
  oodleEffort @6 :Core.OptionalText;
}

struct OptionalProjectBuildPackageProfile {
  union {
    none @0 :Void;
    value @1 :ProjectBuildPackageProfile;
  }
}

struct ProjectBuildPlan {
  commands @0 :List(ProjectBuildCommand);
  packageProfile @1 :OptionalProjectBuildPackageProfile;
}

struct ExecuteProjectBuildRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  profile @2 :Text;
  targetTriple @3 :Text;
  packageSelectors @4 :List(Text);
}

struct OptionalProjectBuildCommand {
  union {
    none @0 :Void;
    value @1 :ProjectBuildCommand;
  }
}

struct ProjectBuildExecutionResult {
  success @0 :Bool;
  plan @1 :ProjectBuildPlan;
  completedCommandCount @2 :UInt32;
  failingCommand @3 :OptionalProjectBuildCommand;
  diagnosticHeadline @4 :Text;
  diagnosticTail @5 :Text;
}

struct ProjectBuildProgressEvent {
  seq @0 :UInt64;
  commandIndex @1 :UInt32;
  commandCount @2 :UInt32;
  targetName @3 :Text;
  doneBp @4 :UInt32;
  commandDone @5 :UInt64;
  commandTotal @6 :UInt64;
  message @7 :Text;
}

interface ProjectBuildProgressSink {
  update @0 (event :ProjectBuildProgressEvent) -> ();
}

struct ExecuteProjectBuildWithProgressRequest {
  request @0 :ExecuteProjectBuildRequest;
  progressSink @1 :ProjectBuildProgressSink;
}

struct PlanProjectServicesRequest {
  capability @0 :Core.Capability;
  projectId @1 :Text;
  sessionSlug @2 :Text;
  endpointKind @3 :Core.EndpointKind;
  workspaceRoot @4 :Text;
  serviceNames @5 :List(Text);
}

struct ProjectServiceCommand {
  serviceName @0 :Text;
  role @1 :Core.ServiceRole;
  endpoint @2 :Core.Endpoint;
  program @3 :Text;
  cwd @4 :Text;
  args @5 :List(Text);
  ownerId @6 :Text;
  ownerRoot @7 :Text;
  buildOutputRoot @8 :Text;
}

struct ProjectServicePlan {
  commands @0 :List(ProjectServiceCommand);
  buildCommands @1 :List(ProjectBuildCommand);
}

# Project-open progress (Stage B). Integers only on the wire: `doneBp` is the
# aggregate completion in basis points (0..=10000); `phaseDone`/`phaseTotal`
# are the raw fraction for the active phase, where `phaseTotal == 0` means the
# phase total is not yet known (a first-ever build streams live messages).
enum OpenProjectPhase {
  resolvePlan @0;
  build @1;
  startServices @2;
  attach @3;
  loadCatalogs @4;
}

struct ProjectOpenProgressEvent {
  seq @0 :UInt64;
  phase @1 :OpenProjectPhase;
  doneBp @2 :UInt32;
  phaseDone @3 :UInt64;
  phaseTotal @4 :UInt64;
  message @5 :Text;
}

# Editor-supplied callback capability. The daemon calls `update` during the
# build/start phases; the final result still returns on the originating RPC.
interface ProjectOpenProgressSink {
  update @0 (event :ProjectOpenProgressEvent) -> ();
}

struct EnsureProjectSessionServicesWithProgressRequest {
  request @0 :EnsureProjectSessionServicesRequest;
  progressSink @1 :ProjectOpenProgressSink;
}

struct ShutdownDaemonRequest {
  capability @0 :Core.Capability;
  reason @1 :Text;
}

struct ShutdownDaemonResult {
  accepted @0 :Bool;
  reason @1 :Text;
}

struct ProcessIdentity {
  processId @0 :UInt32;
  processStartTime @1 :UInt64;
}

struct TouchEditorLeaseRequest {
  capability @0 :Core.Capability;
  leaseId @1 :Text;
  ownerProcess @2 :ProcessIdentity;
  purpose @3 :Text;
}

struct TouchEditorLeaseResult {
  accepted @0 :Bool;
  leaseId @1 :Text;
  activeLeaseCount @2 :UInt32;
}

interface AzDaemon {
  registerProject @0 (request :RegisterProjectRequest) -> (project :ProjectRecord);
  listProjects @1 (request :ListProjectsRequest) -> (result :ListProjectsResult);
  resolveProject @2 (request :ResolveProjectRequest) -> (result :ProjectResult);
  registerSessionSupervisor @3 (request :RegisterSessionSupervisorRequest) -> (descriptor :Core.ServiceDescriptor);
  resolveSessionSupervisor @4 (request :ResolveSessionSupervisorRequest) -> (result :SessionSupervisorResult);
  planProjectBuild @5 (request :PlanProjectBuildRequest) -> (plan :ProjectBuildPlan);
  planProjectServices @6 (request :PlanProjectServicesRequest) -> (plan :ProjectServicePlan);
  registerProjectRoot @7 (request :RegisterProjectRootRequest) -> (project :ProjectRecord);
  listSessionSupervisors @8 (request :ListSessionSupervisorsRequest) -> (result :ListSessionSupervisorsResult);
  unregisterSessionSupervisor @9 (request :UnregisterSessionSupervisorRequest) -> (result :UnregisterSessionSupervisorResult);
  shutdown @10 (request :ShutdownDaemonRequest) -> (result :ShutdownDaemonResult);
  ensureProjectSession @11 (request :EnsureProjectSessionRequest) -> (result :ProjectSessionResult);
  prepareProjectSessionServices @12 (request :PrepareProjectSessionServicesRequest) -> (result :ProjectSessionServicesResult);
  touchEditorLease @13 (request :TouchEditorLeaseRequest) -> (result :TouchEditorLeaseResult);
  ensureProjectSessionServicesWithProgress @14 (request :EnsureProjectSessionServicesWithProgressRequest) -> (result :ProjectSessionServicesStartResult);
  executeProjectBuild @15 (request :ExecuteProjectBuildWithProgressRequest) -> (result :ProjectBuildExecutionResult);
  health @16 () -> (health :Core.ServiceHealth);
}
