@0xf0fe538a5c806112;

using Core = import "/azoth/core.capnp";

# Process-neutral reflected value carrier. Format-specific protocols refer to
# this type rather than declaring competing envelope definitions.
enum ReflectedValueEncoding {
  bevyRemoteJson @0;
  typedRon @1;
  capnpData @2;
}

struct ReflectedValueEnvelope {
  typePath @0 :Text;
  encoding @1 :ReflectedValueEncoding;
  payload @2 :Data;
}

struct SourceFileEditObject {
  objectId @0 :Text;
  schema @1 :Text;
  value @2 :ReflectedValueEnvelope;
}

# Canonical document reloaded through a source schema's edit codec. Codec
# state is opaque to protocol infrastructure but must survive a worker round
# trip so format adapters can preserve stable identity and layout metadata.
struct SourceFileEditDocument {
  rootObjectId @0 :Core.OptionalText;
  rootSchema @1 :Text;
  value @2 :ReflectedValueEnvelope;
  objects @3 :List(SourceFileEditObject);
  codecState @4 :Data;
}

struct SourceFileEditOperation {
  union {
    appendDefault @0 :Void;
    duplicateObject @1 :Text;
    removeObject @2 :Text;
  }
}

# Trusted processor-to-codec operation. Public editor requests carry only
# SourceFileEditOperation; history replay may restore a validated structured
# document without exposing or accepting arbitrary source bytes.
struct SourceFileCodecOperation {
  union {
    load @0 :Void;
    edit @1 :SourceFileEditOperation;
    restoreDocument @2 :SourceFileEditDocument;
  }
}
