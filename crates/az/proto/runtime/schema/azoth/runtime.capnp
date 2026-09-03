@0xd8ea3a6af1d5294e;

using Core = import "/azoth/core.capnp";

enum RuntimeRole {
  editorWorld @0;
  playPreview @1;
  serverPreview @2;
  validation @3;
  thumbnail @4;
  bake @5;
}

enum RuntimeState {
  stopped @0;
  starting @1;
  running @2;
  failed @3;
}

enum RuntimeAssetPackageContainer {
  loose @0;
  azpack @1;
  pak @2;
}

enum ViewportPixelFormat {
  unknown @0;
  bgra8Unorm @1;
  rgba8Unorm @2;
  rgba16Float @3;
  depth32Float @4;
}

struct RuntimeLaunchSnapshot {
  schemaVersion @0 :UInt32;
  projectId @1 :Text;
  sessionSlug @2 :Text;
  role @3 :RuntimeRole;
  projectRoot @4 :Text;
  workspacePath @5 :Text;
  authoredRevision @6 :UInt64;
  schemaCatalogHash @7 :Data;
  workspaceId @8 :Int64;
  includeUnsavedJournal @9 :Bool;
  launchProfile @10 :Text;
  resolvedGems @11 :List(RuntimeResolvedGem);
  assetSourceRoots @12 :List(RuntimeAssetSourceRoot);
  sessionId @13 :Core.SessionId;
  assetPackageRoots @14 :List(RuntimeAssetPackageRoot);
}

struct RuntimeAssetSourceRoot {
  workspaceRootId @0 :Int64;
  workspaceId @1 :Int64;
  rootId @2 :Int64;
  ownerId @3 :Text;
  sourceRoot @4 :Text;
  displayName @5 :Text;
  portableKey @6 :Text;
  outputPrefix @7 :Text;
  isRoot @8 :Bool;
}

struct RuntimeAssetPackageRoot {
  profile @0 :Text;
  assetPlatform @1 :Text;
  container @2 :RuntimeAssetPackageContainer;
  mountRoot @3 :Text;
  payloadPath @4 :Text;
  catalogPath @5 :Text;
  releaseId @6 :Text;
}

struct RuntimeResolvedGem {
  id @0 :Text;
  name @1 :Text;
  version @2 :Text;
  root @3 :Text;
}

struct LaunchRuntimeRequest {
  capability @0 :Core.Capability;
  runtimeId @1 :Text;
  role @2 :RuntimeRole;
  snapshot @3 :Core.SideChannelHandle;
}

struct RuntimeStatusRequest {
  capability @0 :Core.Capability;
  runtimeId @1 :Text;
}

struct StopRuntimeRequest {
  capability @0 :Core.Capability;
  runtimeId @1 :Text;
  preserve @2 :Bool;
}

struct RuntimeStatus {
  runtimeId @0 :Text;
  role @1 :RuntimeRole;
  state @2 :RuntimeState;
  projectId @3 :Text;
  sessionSlug @4 :Text;
  authoredRevision @5 :UInt64;
  diagnostic @6 :Text;
}

struct RuntimeStatusResult {
  union {
    none @0 :Void;
    status @1 :RuntimeStatus;
  }
}

struct RuntimeViewportRequest {
  capability @0 :Core.Capability;
  runtimeId @1 :Text;
}

struct RuntimeViewportFrame {
  runtimeId @0 :Text;
  width @1 :UInt32;
  height @2 :UInt32;
  rowPitch @3 :UInt32;
  format @4 :ViewportPixelFormat;
  color @5 :Core.SideChannelHandle;
}

struct RuntimeViewportResult {
  union {
    none @0 :Void;
    frame @1 :RuntimeViewportFrame;
  }
}

struct RuntimeProjectionDescriptor {
  name @0 :Text;
  priority @1 :Int32;
  roles @2 :List(RuntimeRole);
  launchProfiles @3 :List(Text);
}

struct RuntimeProjectionCatalogRequest {
  capability @0 :Core.Capability;
}

struct RuntimeProjectionCatalogResult {
  projections @0 :List(RuntimeProjectionDescriptor);
}

interface RuntimeHost {
  launch @0 (request :LaunchRuntimeRequest) -> (status :RuntimeStatus);
  status @1 (request :RuntimeStatusRequest) -> (result :RuntimeStatusResult);
  stop @2 (request :StopRuntimeRequest) -> (status :RuntimeStatus);
  viewportFrame @3 (request :RuntimeViewportRequest) -> (result :RuntimeViewportResult);
  projectionCatalog @4 (request :RuntimeProjectionCatalogRequest) -> (result :RuntimeProjectionCatalogResult);
  health @5 () -> (health :Core.ServiceHealth);
}
