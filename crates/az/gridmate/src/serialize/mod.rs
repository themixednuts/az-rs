// GridMate Serialize module (mirrors GridMate/Serialize/)
//
// This module provides GridMate-style (de)serialization primitives:
// - Fixed-width and utility types in `data_marshal` and `utility_marshal`.
// - VLQ32/64 encodings in `vlq` for compact container lengths and varints.
// - Container support in `container_marshal` (e.g. `Vec<T>`, tuples, `ArrayVec<T, N>`,
//   `IndexSet<T>`, `IndexMap<K, V>`, plus standard Rust maps/sets for non-protocol
//   uses). Containers encode their element-count using VLQ32, followed by elements
//   in iteration order.
// - Replicated-state building blocks live in `replicated_container`,
//   `replicated_field`, and `compression_marshal`. `ReplicatedContainer`
//   mirrors the native `m_lastModified`/`m_currentChanges`/journal shape and
//   implements `ReplicatedFieldHandlerBase` directly.
//   The default `Marshaler<bool>` is strict — `0`/`1` only — so there is no
//   separate "strict bool" policy.
//
// Compatibility notes (modeled after GridMate/Serialize):
// - Lengths are encoded as unsigned VLQ32 values.
// - Native unordered-map/set helpers still marshal an ordered byte stream: the current
//   native iteration order. Use `IndexSet<T>` / `IndexMap<K, V>` for protocol fields so
//   unmarshal preserves that order. Rust `HashSet<T>` / `HashMap<K, V>` remain available
//   for semantic containers, but their randomized iteration order cannot reproduce a
//   capture byte-for-byte.
// - All `Marshaler` impls consume values on marshal (by value). Unmarshal returns
//   `Result<Self, MarshalerError>` so callers get explicit error context.

mod asset_marshal;
pub mod buffer;
pub mod compression_marshal;
pub mod container_marshal;
pub mod crc32;
pub mod data_marshal;
pub mod error;
pub mod flat_bitmask;
pub mod live_mask;
pub mod marshaler;
pub mod mask_chain;
pub mod math_marshal;
mod quantize;
pub mod replicated_container;
pub mod replicated_field;
mod secret_value_marshal;
pub mod utility_marshal;
pub mod vlq;

#[cfg(debug_assertions)]
pub use buffer::DebugRecorder;
pub use buffer::{CARRIER_ENDIAN, Endian, ReadBuffer, WriteBuffer};
pub use compression_marshal::{
    Float16Marshaler, IntegerQuantizationMarshalerU8, IntegerQuantizationMarshalerU16,
    IntegerQuantizationMarshalerU32, NonUniformScaleCompMarshaler, PackedNormalizedVec3Marshaller,
    PackedPositionMarshaller, PackedSize, QuatCompMarshaler, QuatCompNorm, QuatCompNormMarshaler,
    QuatCompNormQuantized, QuatCompNormQuantizedAngles, QuatCompNormQuantizedMarshaler,
    QuatSmallestThreeQuantized, QuatSmallestThreeQuantizedMarshaler, TransformCompressor,
    Vec2CompMarshaler, Vec3CompMarshaler, Vec3CompNormMarshaler,
};
pub use container_marshal::WIRE_VEC_CAP;
pub use data_marshal::{ConversionMarshaler, MarshalerConversion};
pub use error::MarshalerError;
pub use flat_bitmask::FlatBitmask;
pub use indexmap::{IndexMap, IndexSet};
pub use live_mask::{read_live_mask_batches, write_live_mask_batches};
#[cfg(debug_assertions)]
pub use marshaler::DebugField;
pub use marshaler::{Codec, DefaultMarshaler, Marshaler};
pub use mask_chain::MaskChain;
pub use replicated_container::{
    Change, ChangeOp, ChangeSet, REPLICATED_CONTAINER_FIXED_JOURNAL_SIZE, ReplicatedContainer,
    ReplicatedIndexMap, ReplicatedMap, ReplicatedVec,
};
pub use replicated_field::{
    DeltaCompressedCounterHandler, DeltaCompressedReplicatedFieldHandler, DeltaIntegerMarshaler,
    DeltaMarshaler, DynamicDeltaReplicatedFieldHandler, FloatTimerDeltaReplicatedField,
    HalfF32Marshaler, HalfVec3Marshaler, IntegerOmitLowerByteMarshaler, PositionAnchorMarshaler,
    QuantizedRelativePosition, ReplicatedFieldHandler,
};
pub use utility_marshal::{BitSet, HalfF32, NativeEndian, RawU64Revision};
pub use vlq::{VlqU16, VlqU16Marshaler, VlqU32, VlqU32Marshaler, VlqU64, VlqU64Marshaler};
