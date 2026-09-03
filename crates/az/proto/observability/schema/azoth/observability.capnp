@0xf35c9e7a1a4c2b61;

using Core = import "/azoth/core.capnp";

enum LogLevel {
  trace @0;
  debug @1;
  info @2;
  warn @3;
  error @4;
}

struct ObservedField {
  name @0 :Text;
  value @1 :Text;
}

struct ObservedSpan {
  name @0 :Text;
  target @1 :Text;
  fields @2 :List(ObservedField);
}

struct TraceContext {
  # OpenTelemetry-compatible binary ids. Empty ids mean no active span.
  # Non-empty traceId is 16 bytes; spanId and parentSpanId are 8 bytes.
  traceId @0 :Data;
  spanId @1 :Data;
  parentSpanId @2 :Data;
  traceFlags @3 :UInt32;
  traceState @4 :Text;
}

struct ObservedLogRecord {
  sequence @0 :UInt64;
  timestampUnixNs @1 :UInt64;
  service @2 :Core.ServiceId;
  role @3 :Core.ServiceRole;
  run @4 :Data;
  sessionSlug @5 :Text;
  target @6 :Text;
  level @7 :LogLevel;
  message @8 :Text;
  fields @9 :List(ObservedField);
  trace @10 :TraceContext;
  file @11 :Text;
  line @12 :UInt32;
  spans @13 :List(ObservedSpan);
}

struct AppendLogBatchRequest {
  capability @0 :Core.Capability;
  records @1 :List(ObservedLogRecord);
}

struct AppendLogBatchResult {
  accepted @0 :Bool;
  nextSequence @1 :UInt64;
  rejectedReason @2 :Text;
}

struct LogQueryRequest {
  capability @0 :Core.Capability;
  sessionSlug @1 :Text;
  service @2 :Core.ServiceId;
  run @3 :Core.OptionalUuid;
  minLevel @4 :LogLevel;
  afterSequence @5 :UInt64;
  pageSize @6 :UInt32;
  includeFields @7 :Bool;
}

struct LogQueryResult {
  records @0 :List(ObservedLogRecord);
  nextSequence @1 :UInt64;
  truncated @2 :Bool;
  source @3 :Text;
}

struct DiagnosticBundleFile {
  path @0 :Text;
  category @1 :Text;
  byteLength @2 :UInt64;
  contentHash @3 :Data;
}

struct DiagnosticBundleManifest {
  schemaVersion @0 :UInt32;
  sessionSlug @1 :Text;
  createdUnixMs @2 :UInt64;
  reason @3 :Text;
  files @4 :List(DiagnosticBundleFile);
}

struct DiagnosticBundleRequest {
  capability @0 :Core.Capability;
  sessionSlug @1 :Text;
  reason @2 :Text;
  includeLogs @3 :Bool;
  includeCrashDumps @4 :Bool;
  maxBytes @5 :UInt64;
}

struct DiagnosticBundleResult {
  accepted @0 :Bool;
  manifest @1 :Core.SideChannelHandle;
  diagnostic @2 :Text;
}

# ---------------------------------------------------------------------------
# Runtime log-verbosity control (engine-intent logging, cross-process dialing).
# A verbosity *threshold*: like LogLevel but with `off`, used to silence/raise a
# channel. Every Azoth process serves `ObservabilityControl` on its own control
# endpoint so the editor can dial a service's channel verbosity live.
# ---------------------------------------------------------------------------
enum LogVerbosity {
  off @0;
  error @1;
  warn @2;
  info @3;
  debug @4;
  trace @5;
}

struct SetLogChannelLevelRequest {
  capability @0 :Core.Capability;
  # A channel target like "az.assets".
  target @1 :Text;
  verbosity @2 :LogVerbosity;
  # When true, remove the override (inherit base) instead of setting `verbosity`.
  clear @3 :Bool;
}

struct ChannelLevel {
  target @0 :Text;
  verbosity @1 :LogVerbosity;
}

struct SetLogChannelLevelResult {
  accepted @0 :Bool;
  effectiveDirectives @1 :Text;
  diagnostic @2 :Text;
}

struct ListChannelLevelsResult {
  effectiveDirectives @0 :Text;
  overrides @1 :List(ChannelLevel);
}

interface ObservabilityControl {
  setLogChannelLevel @0 (request :SetLogChannelLevelRequest) -> (result :SetLogChannelLevelResult);
  listChannelLevels @1 (capability :Core.Capability) -> (result :ListChannelLevelsResult);
  health @2 () -> (health :Core.ServiceHealth);
}
