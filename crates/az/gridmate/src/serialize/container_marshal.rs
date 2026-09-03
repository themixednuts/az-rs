// Container marshalers (specialized impls)
//
// Conventions:
// - Default container marshalers use VLQ32 lengths.
// - Explicit `ContainerMarshaler` and `MapContainerMarshaler` policies use raw
//   u16 lengths.
// - Elements are serialized in the container's iteration order.
// - Unordered map/set marshalers emit entries in their current iteration order.
//   Use `IndexMap` / `IndexSet` when insertion order must survive a round trip.

use super::{
    buffer::{ReadBuffer, WriteBuffer},
    error::MarshalerError,
    marshaler::{Codec, DefaultMarshaler, Marshaler},
    vlq::VlqU32Marshaler,
};
use arrayvec::{ArrayString, ArrayVec};
use indexmap::{IndexMap, IndexSet};
use smallvec::{Array, SmallVec};
use std::marker::PhantomData;
use std::str::FromStr;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt::Debug,
};

/// Maximum element or byte count accepted by dynamically sized container
/// readers.
pub const WIRE_VEC_CAP: usize = 0x0200_0000;

/// Narrow a default container length to its VLQ32 value.
#[inline]
pub(super) fn wire_len(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// Write an explicit container policy's count prefix.
///
/// `Marshaler::marshal` is infallible, so release builds saturate an invalid
/// oversized value after the debug assertion instead of truncating it.
#[inline]
pub(super) fn marshal_container_len(wb: &mut WriteBuffer, len: usize) {
    debug_assert!(
        len < usize::from(u16::MAX),
        "GridMate container count must be less than u16::MAX"
    );
    u16::try_from(len).unwrap_or(u16::MAX).marshal(wb);
}

/// Read an explicit container policy's count prefix.
#[inline]
pub(super) fn unmarshal_container_len(rb: &mut ReadBuffer) -> Result<usize, MarshalerError> {
    Ok(usize::from(u16::unmarshal(rb)?))
}

/// Source-shaped explicit container policy.
///
/// The C++ `GridMate::ContainerMarshaler<Container, DataMarshaler>` writes a
/// raw `u16` element count followed by each element through the selected inner
/// marshaler. Use this policy when a field selects a non-default element codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContainerMarshaler<T, M = DefaultMarshaler<T>>(PhantomData<fn() -> (T, M)>);

impl<T, M> ContainerMarshaler<T, M>
where
    M: Codec<T>,
{
    fn marshal_len(wb: &mut WriteBuffer, len: usize) {
        marshal_container_len(wb, len);
    }

    fn unmarshal_len(rb: &mut ReadBuffer) -> Result<usize, MarshalerError> {
        unmarshal_container_len(rb)
    }
}

impl<T, M> Codec<Vec<T>> for ContainerMarshaler<T, M>
where
    M: Codec<T>,
{
    fn marshal(value: &Vec<T>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for item in value {
            M::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Vec<T>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = Vec::with_capacity(len);
        for index in 0..len {
            value.push(rb.indexed_span(index, |rb| M::unmarshal(rb))?);
        }
        Ok(value)
    }
}

impl<T, M, const N: usize> Codec<ArrayVec<T, N>> for ContainerMarshaler<T, M>
where
    M: Codec<T>,
{
    fn marshal(value: &ArrayVec<T, N>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for item in value {
            M::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<ArrayVec<T, N>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        if len > N {
            return Err(MarshalerError::ContainerOverflow { len, capacity: N });
        }
        let mut value = ArrayVec::new();
        for index in 0..len {
            value.push(rb.indexed_span(index, |rb| M::unmarshal(rb))?);
        }
        Ok(value)
    }
}

impl Codec<String> for ContainerMarshaler<u8> {
    fn marshal(value: &String, wb: &mut WriteBuffer) {
        let bytes = value.as_bytes();
        Self::marshal_len(wb, bytes.len());
        wb.write_bytes(bytes);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<String, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let bytes = rb.read_bytes(len)?;
        Ok(std::str::from_utf8(bytes)?.to_string())
    }
}

impl<const N: usize> Codec<ArrayString<N>> for ContainerMarshaler<u8> {
    fn marshal(value: &ArrayString<N>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        wb.write_bytes(value.as_bytes());
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<ArrayString<N>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        if len > N {
            return Err(MarshalerError::ArrayStringOverflow { len, capacity: N });
        }
        let bytes = rb.read_bytes(len)?;
        let s = std::str::from_utf8(bytes)?;
        ArrayString::from_str(s)
            .map_err(|_| MarshalerError::ArrayStringOverflow { len, capacity: N })
    }
}

impl<T, M> Codec<IndexSet<T>> for ContainerMarshaler<T, M>
where
    T: Eq + std::hash::Hash,
    M: Codec<T>,
{
    fn marshal(value: &IndexSet<T>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for item in value {
            M::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<IndexSet<T>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = IndexSet::with_capacity(len);
        for index in 0..len {
            value.insert(rb.indexed_span(index, |rb| M::unmarshal(rb))?);
        }
        Ok(value)
    }
}

impl<T, M> Codec<HashSet<T>> for ContainerMarshaler<T, M>
where
    T: Eq + std::hash::Hash,
    M: Codec<T>,
{
    fn marshal(value: &HashSet<T>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for item in value {
            M::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<HashSet<T>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = HashSet::with_capacity(len);
        for index in 0..len {
            value.insert(rb.indexed_span(index, |rb| M::unmarshal(rb))?);
        }
        Ok(value)
    }
}

impl<T, M> Codec<BTreeSet<T>> for ContainerMarshaler<T, M>
where
    T: Ord,
    M: Codec<T>,
{
    fn marshal(value: &BTreeSet<T>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for item in value {
            M::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<BTreeSet<T>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = BTreeSet::new();
        for index in 0..len {
            value.insert(rb.indexed_span(index, |rb| M::unmarshal(rb))?);
        }
        Ok(value)
    }
}

impl<T, M, const N: usize> Codec<[T; N]> for ContainerMarshaler<T, M>
where
    M: Codec<T>,
{
    fn marshal(value: &[T; N], wb: &mut WriteBuffer) {
        Self::marshal_len(wb, N);
        for item in value {
            M::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<[T; N], MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        if len != N {
            return Err(MarshalerError::ContainerOverflow { len, capacity: N });
        }
        let mut value = Vec::with_capacity(N);
        for index in 0..N {
            value.push(rb.indexed_span(index, |rb| M::unmarshal(rb))?);
        }
        value
            .try_into()
            .map_err(|value: Vec<T>| MarshalerError::ContainerOverflow {
                len: value.len(),
                capacity: N,
            })
    }
}

/// Source-shaped explicit map policy.
///
/// Mirrors `GridMate::MapContainerMarshaler`: raw `u16` entry count, then each
/// key/value pair through its configured marshaler.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapContainerMarshaler<K, V, KM = DefaultMarshaler<K>, VM = DefaultMarshaler<V>>(
    MapMarshalerMarker<K, V, KM, VM>,
);

/// Variance/ownership marker for [`MapContainerMarshaler`]'s four parameters.
///
/// `fn() -> (..)` keeps the policy covariant and `Send + Sync` regardless of
/// what the key, value, and codec types are.
type MapMarshalerMarker<K, V, KM, VM> = PhantomData<fn() -> (K, V, KM, VM)>;

impl<K, V, KM, VM> MapContainerMarshaler<K, V, KM, VM>
where
    KM: Codec<K>,
    VM: Codec<V>,
{
    fn marshal_len(wb: &mut WriteBuffer, len: usize) {
        marshal_container_len(wb, len);
    }

    fn unmarshal_len(rb: &mut ReadBuffer) -> Result<usize, MarshalerError> {
        unmarshal_container_len(rb)
    }
}

impl<K, V, KM, VM> Codec<HashMap<K, V>> for MapContainerMarshaler<K, V, KM, VM>
where
    K: Eq + std::hash::Hash,
    KM: Codec<K>,
    VM: Codec<V>,
{
    fn marshal(value: &HashMap<K, V>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for (key, item) in value {
            KM::marshal(key, wb);
            VM::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<HashMap<K, V>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = HashMap::with_capacity(len);
        for index in 0..len {
            let (key, item) =
                rb.indexed_span(index, |rb| Ok((KM::unmarshal(rb)?, VM::unmarshal(rb)?)))?;
            value.insert(key, item);
        }
        Ok(value)
    }
}

impl<K, V, KM, VM> Codec<BTreeMap<K, V>> for MapContainerMarshaler<K, V, KM, VM>
where
    K: Ord,
    KM: Codec<K>,
    VM: Codec<V>,
{
    fn marshal(value: &BTreeMap<K, V>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for (key, item) in value {
            KM::marshal(key, wb);
            VM::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<BTreeMap<K, V>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = BTreeMap::new();
        for index in 0..len {
            let (key, item) =
                rb.indexed_span(index, |rb| Ok((KM::unmarshal(rb)?, VM::unmarshal(rb)?)))?;
            value.insert(key, item);
        }
        Ok(value)
    }
}

impl<K, V, KM, VM> Codec<IndexMap<K, V>> for MapContainerMarshaler<K, V, KM, VM>
where
    K: Eq + std::hash::Hash,
    KM: Codec<K>,
    VM: Codec<V>,
{
    fn marshal(value: &IndexMap<K, V>, wb: &mut WriteBuffer) {
        Self::marshal_len(wb, value.len());
        for (key, item) in value {
            KM::marshal(key, wb);
            VM::marshal(item, wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<IndexMap<K, V>, MarshalerError> {
        let len = Self::unmarshal_len(rb)?;
        let mut value = IndexMap::with_capacity(len);
        for index in 0..len {
            let (key, item) =
                rb.indexed_span(index, |rb| Ok((KM::unmarshal(rb)?, VM::unmarshal(rb)?)))?;
            value.insert(key, item);
        }
        Ok(value)
    }
}

/// `Vec<T>` encoded as a VLQ32 length followed by `T` elements in order.
impl<T: Marshaler> Marshaler for Vec<T> {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for item in self {
            item.marshal(wb);
        }
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut v = Self::with_capacity(len);
        for index in 0..len {
            v.push(rb.indexed_span(index, |rb| T::unmarshal(rb))?);
        }
        Ok(v)
    }
}

/// `(A, B)` encoded as: `A` then `B`.
impl<A: Marshaler, B: Marshaler> Marshaler for (A, B) {
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.0.marshal(wb);
        self.1.marshal(wb);
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok((
            rb.span("0", |rb| A::unmarshal(rb))?,
            rb.span("1", |rb| B::unmarshal(rb))?,
        ))
    }
}

/// `(A, B, C)` encoded as: `A` then `B` then `C`.
impl<A: Marshaler, B: Marshaler, C: Marshaler> Marshaler for (A, B, C) {
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.0.marshal(wb);
        self.1.marshal(wb);
        self.2.marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok((
            rb.span("0", |rb| A::unmarshal(rb))?,
            rb.span("1", |rb| B::unmarshal(rb))?,
            rb.span("2", |rb| C::unmarshal(rb))?,
        ))
    }
}

/// `(A, B, C, D)` encoded as: `A`, `B`, `C`, then `D`.
impl<A: Marshaler, B: Marshaler, C: Marshaler, D: Marshaler> Marshaler for (A, B, C, D) {
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.0.marshal(wb);
        self.1.marshal(wb);
        self.2.marshal(wb);
        self.3.marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok((
            rb.span("0", |rb| A::unmarshal(rb))?,
            rb.span("1", |rb| B::unmarshal(rb))?,
            rb.span("2", |rb| C::unmarshal(rb))?,
            rb.span("3", |rb| D::unmarshal(rb))?,
        ))
    }
}

/// `ArrayVec<T, N>` encoded as a VLQ32 length followed by `T` elements.
impl<T: Marshaler + Debug, const N: usize> Marshaler for ArrayVec<T, N> {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for item in self {
            item.marshal(wb);
        }
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > N {
            return Err(MarshalerError::ContainerOverflow { len, capacity: N });
        }
        let mut v: Self = Self::new();
        for index in 0..len {
            v.push(rb.indexed_span(index, |rb| T::unmarshal(rb))?);
        }
        Ok(v)
    }
}

/// `SmallVec<A>` encoded as a VLQ32 length followed by its elements.
/// Inline capacity is a Rust storage optimization only.
impl<A> Marshaler for SmallVec<A>
where
    A: Array,
    A::Item: Marshaler,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for item in self {
            item.marshal(wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut value = Self::with_capacity(len);
        for index in 0..len {
            value.push(rb.indexed_span(index, |rb| A::Item::unmarshal(rb))?);
        }
        Ok(value)
    }
}

impl<const N: usize> Marshaler for ArrayString<N> {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        wb.write_bytes(self.as_bytes());
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > N {
            return Err(MarshalerError::ArrayStringOverflow { len, capacity: N });
        }
        let bytes = rb.read_bytes(len)?;
        let s = std::str::from_utf8(bytes)?;
        Self::from_str(s).map_err(|_| MarshalerError::ArrayStringOverflow { len, capacity: N })
    }
}

/// `[T; N]` encoded as exactly `N` consecutive elements without a prefix.
impl<T: Marshaler + Debug, const N: usize> Marshaler for [T; N] {
    fn marshal(&self, wb: &mut WriteBuffer) {
        for item in self {
            item.marshal(wb);
        }
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let mut tmp: ArrayVec<T, N> = ArrayVec::new();
        for index in 0..N {
            tmp.push(rb.indexed_span(index, |rb| T::unmarshal(rb))?);
        }
        tmp.into_inner()
            .map_err(|_| MarshalerError::ContainerOverflow {
                len: N + 1,
                capacity: N,
            })
    }
}

/// `IndexSet<T>` encoded as a VLQ32 length followed by its elements.
///
/// This keeps set semantics while preserving insertion order after unmarshal.
impl<T> Marshaler for IndexSet<T>
where
    T: Marshaler + Eq + std::hash::Hash,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for item in self {
            item.marshal(wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut set = Self::with_capacity(len);
        for index in 0..len {
            set.insert(rb.indexed_span(index, |rb| T::unmarshal(rb))?);
        }
        Ok(set)
    }
}

/// `IndexMap<K, V>` encoded as a VLQ32 length followed by key/value pairs.
///
/// This keeps map semantics while preserving insertion order after unmarshal.
impl<K, V> Marshaler for IndexMap<K, V>
where
    K: Marshaler + Eq + std::hash::Hash,
    V: Marshaler,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for (k, v) in self {
            k.marshal(wb);
            v.marshal(wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut map = Self::with_capacity(len);
        for index in 0..len {
            let (k, v) = rb.indexed_span(index, |rb| {
                Ok((
                    rb.span("key", |rb| K::unmarshal(rb))?,
                    rb.span("value", |rb| V::unmarshal(rb))?,
                ))
            })?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

/// `HashSet<T>` encoded as a VLQ32 length followed by its elements.
///
/// Rust's hash-table iteration order is not stable. Use [`IndexSet`] when
/// insertion order must survive a round trip.
impl<T, S> Marshaler for HashSet<T, S>
where
    T: Marshaler + Eq + std::hash::Hash,
    S: std::hash::BuildHasher + Default,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for item in self {
            item.marshal(wb);
        }
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut set = Self::with_capacity_and_hasher(len, S::default());
        for index in 0..len {
            set.insert(rb.indexed_span(index, |rb| T::unmarshal(rb))?);
        }
        Ok(set)
    }
}

/// `BTreeSet<T>` encoded as a VLQ32 length followed by sorted elements.
impl<T> Marshaler for BTreeSet<T>
where
    T: Marshaler + Ord,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for item in self {
            item.marshal(wb);
        }
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut set = Self::new();
        for index in 0..len {
            set.insert(rb.indexed_span(index, |rb| T::unmarshal(rb))?);
        }
        Ok(set)
    }
}

/// `HashMap<K, V>` encoded as a VLQ32 length followed by key/value pairs.
///
/// Rust's hash-table iteration order is not stable. Use [`IndexMap`] when
/// insertion order must survive a round trip.
impl<K, V, S> Marshaler for HashMap<K, V, S>
where
    K: Marshaler + Eq + std::hash::Hash,
    V: Marshaler,
    S: std::hash::BuildHasher + Default,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for (k, v) in self {
            k.marshal(wb);
            v.marshal(wb);
        }
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut map = Self::with_capacity_and_hasher(len, S::default());
        for index in 0..len {
            let (k, v) = rb.indexed_span(index, |rb| {
                Ok((
                    rb.span("key", |rb| K::unmarshal(rb))?,
                    rb.span("value", |rb| V::unmarshal(rb))?,
                ))
            })?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

/// `BTreeMap<K, V>` encoded as a VLQ32 length followed by key/value pairs.
/// Unlike `HashMap`, iteration order is deterministic (sorted by key).
impl<K, V> Marshaler for std::collections::BTreeMap<K, V>
where
    K: Marshaler + Ord,
    V: Marshaler,
{
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU32Marshaler.marshal(wb, wire_len(self.len()));
        for (k, v) in self {
            k.marshal(wb);
            v.marshal(wb);
        }
    }
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let len = VlqU32Marshaler.unmarshal(rb)? as usize;
        if len > WIRE_VEC_CAP {
            return Err(MarshalerError::ContainerOverflow {
                len,
                capacity: WIRE_VEC_CAP,
            });
        }
        let mut map = Self::new();
        for index in 0..len {
            let (k, v) = rb.indexed_span(index, |rb| {
                Ok((
                    rb.span("key", |rb| K::unmarshal(rb))?,
                    rb.span("value", |rb| V::unmarshal(rb))?,
                ))
            })?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    fn read_len_only<T: Marshaler>(len: usize) -> Result<T, MarshalerError> {
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        VlqU32Marshaler.marshal(&mut wb, wire_len(len));
        let bytes = wb.into_vec();
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        T::unmarshal(&mut rb)
    }

    #[test]
    fn default_vec_uses_vlq32_count() {
        let value = vec![0x11u8, 0x22, 0x33];
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        let bytes = wb.into_vec();

        assert_eq!(bytes, [3, 0x11, 0x22, 0x33]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        assert_eq!(Vec::<u8>::unmarshal(&mut rb).unwrap(), value);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn default_array_is_prefix_free() {
        let value = [0x11u8, 0x22];
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        let bytes = wb.into_vec();

        assert_eq!(bytes, [0x11, 0x22]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        assert_eq!(<[u8; 2]>::unmarshal(&mut rb).unwrap(), value);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn default_index_map_uses_vlq32_count_and_preserves_order() {
        let mut value = IndexMap::new();
        value.insert(2u8, 20u16);
        value.insert(1, 10);
        value.insert(3, 30);
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        let bytes = wb.into_vec();
        assert_eq!(bytes[0], 3);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = IndexMap::<u8, u16>::unmarshal(&mut rb).unwrap();

        assert_eq!(
            decoded.into_iter().collect::<Vec<_>>(),
            vec![(2, 20), (1, 10), (3, 30)]
        );
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn default_index_set_uses_vlq32_count_and_preserves_order() {
        let mut value = IndexSet::new();
        value.insert(3u8);
        value.insert(1);
        value.insert(2);
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);
        let bytes = wb.into_vec();
        assert_eq!(bytes[0], 3);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = IndexSet::<u8>::unmarshal(&mut rb).unwrap();

        assert_eq!(decoded.into_iter().collect::<Vec<_>>(), vec![3, 1, 2]);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn explicit_container_marshaler_uses_u16_count() {
        let value = vec![0x11u8, 0x22, 0x33];
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        ContainerMarshaler::<u8>::marshal(&value, &mut wb);
        let bytes = wb.into_vec();

        assert_eq!(&bytes[..2], &3u16.to_be_bytes());
        assert_eq!(&bytes[2..], &[0x11, 0x22, 0x33]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded: Vec<u8> = ContainerMarshaler::<u8>::unmarshal(&mut rb).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn explicit_container_marshaler_handles_string_as_u16_counted_bytes() {
        let value = String::from("mix");
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        ContainerMarshaler::<u8>::marshal(&value, &mut wb);
        let bytes = wb.into_vec();

        assert_eq!(&bytes[..2], &3u16.to_be_bytes());
        assert_eq!(&bytes[2..], b"mix");

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = <ContainerMarshaler<u8> as Codec<String>>::unmarshal(&mut rb).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn explicit_map_container_marshaler_uses_u16_count() {
        let mut value = IndexMap::new();
        value.insert(7u8, 70u16);
        value.insert(8, 80);

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        MapContainerMarshaler::<u8, u16>::marshal(&value, &mut wb);
        let bytes = wb.into_vec();

        assert_eq!(&bytes[..2], &2u16.to_be_bytes());

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded: IndexMap<u8, u16> =
            MapContainerMarshaler::<u8, u16>::unmarshal(&mut rb).unwrap();
        assert_eq!(
            decoded.into_iter().collect::<Vec<_>>(),
            vec![(7, 70), (8, 80)]
        );
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn default_maps_reject_oversized_wire_counts() {
        let len = WIRE_VEC_CAP + 1;
        for result in [
            read_len_only::<HashSet<u8>>(len).map(|_| ()),
            read_len_only::<HashMap<u8, u8>>(len).map(|_| ()),
            read_len_only::<IndexSet<u8>>(len).map(|_| ()),
            read_len_only::<IndexMap<u8, u8>>(len).map(|_| ()),
            read_len_only::<BTreeMap<u8, u8>>(len).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(MarshalerError::ContainerOverflow { len: got, capacity })
                    if got == len && capacity == WIRE_VEC_CAP
            ));
        }
    }
}
