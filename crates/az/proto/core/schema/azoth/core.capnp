@0xb9d8d3934f225ef0;

struct ProtocolVersion {
  major @0 :UInt16;
  minor @1 :UInt16;
  patch @2 :UInt16;
}

struct SessionId {
  # UUID bytes in network byte order.
  uuid @0 :Data;
}

struct OptionalText {
  union {
    none @0 :Void;
    value @1 :Text;
  }
}

struct OptionalI64 {
  union {
    none @0 :Void;
    value @1 :Int64;
  }
}

struct OptionalBool {
  union {
    none @0 :Void;
    value @1 :Bool;
  }
}

struct OptionalU32 {
  union {
    none @0 :Void;
    value @1 :UInt32;
  }
}

struct OptionalU64 {
  union {
    none @0 :Void;
    value @1 :UInt64;
  }
}

struct OptionalF64 {
  union {
    none @0 :Void;
    value @1 :Float64;
  }
}

struct OptionalData {
  union {
    none @0 :Void;
    value @1 :Data;
  }
}

struct OptionalUuid {
  union {
    none @0 :Void;
    # UUID bytes in network byte order.
    value @1 :Data;
  }
}

struct ServiceId {
  namespace @0 :Text;
  name @1 :Text;
}

enum ServiceRole {
  unknown @0;
  editor @1;
  daemon @2;
  sessionSupervisor @3;
  projectHost @4;
  assetProcessor @5;
  runtimeHost @6;
  worker @7;
}

struct Capability {
  session @0 :OptionalUuid;
  service @1 :ServiceId;
  role @2 :ServiceRole;
  permissions @3 :List(Text);
  audience @4 :Text;
  expiresUnixMs @5 :UInt64;
  tokenHash @6 :Data;
}

struct CapabilityGrantSet {
  grants @0 :List(Capability);
}

enum EndpointKind {
  windowsNamedPipe @0;
  unixDomainSocket @1;
  tcp @2;
  inProcess @3;
}

struct Endpoint {
  kind @0 :EndpointKind;
  address @1 :Text;
}

enum SideChannelKind {
  sharedMemory @0;
  casBlob @1;
  stagingFile @2;
  gpuSurface @3;
  mmapFile @4;
}

struct SideChannelHandle {
  kind @0 :SideChannelKind;
  capability @1 :Capability;
  locator @2 :Text;
  byteLength @3 :UInt64;
  contentHash @4 :Data;
  platform @5 :Text;
}

struct ServiceDescriptor {
  id @0 :ServiceId;
  role @1 :ServiceRole;
  # UUIDv7 launch label. Observational only; never an authority or routing key.
  run @2 :Data;
  protocol @3 :ProtocolVersion;
  endpoint @4 :Endpoint;
  capabilities @5 :List(Capability);
  observabilityEndpoint @6 :Endpoint;
  # Authenticated process-lifetime control endpoint. This is distinct from the
  # role RPC endpoint so shutdown remains one generic service contract.
  lifecycleEndpoint @7 :Endpoint;
}

enum ServiceHealthState {
  unknown @0;
  starting @1;
  ready @2;
  busy @3;
  degraded @4;
  stopping @5;
  failed @6;
}

struct ServiceHealth {
  service @0 :ServiceId;
  role @1 :ServiceRole;
  # Echoed launch label for log attribution only.
  run @2 :Data;
  protocolVersion @3 :ProtocolVersion;
  state @4 :ServiceHealthState;
  ready @5 :Bool;
  degraded @6 :Bool;
  pid @7 :UInt32;
  checkedUnixMs @8 :UInt64;
  uptimeMs @9 :UInt64;
  activeOperation @10 :Text;
  message @11 :Text;
  lastEventSeq @12 :UInt64;
}

# A service entrypoint owns this capability. A trusted session supervisor uses
# it to request graceful process shutdown before identity-bound force
# termination is considered.
interface ServiceLifecycleControl {
  shutdown @0 (capability :Capability) -> ();
}
