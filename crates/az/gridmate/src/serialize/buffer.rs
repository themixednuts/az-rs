// GridMate Serialize Buffer (GridMate/Serialize/Buffer.h)
// Read/Write buffers for serialization

use super::error::MarshalerError;
#[cfg(debug_assertions)]
use super::marshaler::DebugField;
use crate::az::TypeRegistry;
use std::io::{Read, Result as IoResult};

/// Endian type for serialization (`GridMate`: `EndianType`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Endian {
    #[default]
    BigEndian, // Network byte order
    LittleEndian, // Host byte order (x86)
}

/// `GridMate` Carrier uses `BigEndian` (network byte order)
pub const CARRIER_ENDIAN: Endian = Endian::BigEndian;

/// Field-tracking sidecar for a [`ReadBuffer`].
///
/// When attached, [`ReadBuffer::field`] records each named field's byte range
/// and `Debug`-formatted value. Nested `field()` calls are preserved as dotted
/// paths (`pools[0].fields[1].name`) so capture tooling can select collection
/// elements and their fields without a second parser.
///
#[cfg(debug_assertions)]
#[derive(Debug, Default)]
pub struct DebugRecorder {
    fields: Vec<DebugField>,
    path: Vec<std::borrow::Cow<'static, str>>,
}

#[cfg(debug_assertions)]
impl DebugRecorder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_fields(self) -> Vec<DebugField> {
        self.fields
    }

    fn push_path(&mut self, name: std::borrow::Cow<'static, str>) {
        self.path.push(name);
    }

    fn pop_path(&mut self) {
        self.path.pop();
    }

    fn path_name(&self) -> std::borrow::Cow<'static, str> {
        if self.path.len() == 1
            && let Some(std::borrow::Cow::Borrowed(name)) = self.path.first()
        {
            return std::borrow::Cow::Borrowed(*name);
        }

        let mut out = String::new();
        for part in &self.path {
            // Index segments (`[3]`) abut the name they index; every other
            // segment is dot-separated from what precedes it.
            if !part.starts_with('[') && !out.is_empty() {
                out.push('.');
            }
            out.push_str(part);
        }
        std::borrow::Cow::Owned(out)
    }
}

/// Read buffer for deserializing data (`GridMate`: `ReadBuffer`)
///
/// # Reflected fields need a bound registry
///
/// Concrete fields decode from bytes and endianness alone. Polymorphic ones —
/// [`NetworkValue`](crate::az::NetworkValue), [`ClassValue`](crate::az::ClassValue)
/// — carry a compact `type_index` that only a host's composed
/// [`TypeRegistry`] can resolve, and `Marshaler::unmarshal` has nothing but the
/// buffer in scope. So the wire seam that builds the buffer binds the registry
/// with [`Self::with_types`], and a buffer that was never bound fails those
/// reads with [`MarshalerError::UnboundTypes`] rather than reporting every
/// reflected type as unknown.
pub struct ReadBuffer<'a> {
    data: &'a [u8],
    position: usize,
    endian: Endian,
    types: Option<TypeRegistry<'a>>,
    #[cfg(debug_assertions)]
    recorder: Option<DebugRecorder>,
}

impl<'a> ReadBuffer<'a> {
    /// Create new `ReadBuffer` with specified endianness (`GridMate`: ReadBuffer(endian, data, size))
    #[must_use]
    pub const fn new(endian: Endian, data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            endian,
            types: None,
            #[cfg(debug_assertions)]
            recorder: None,
        }
    }

    /// Bind the composed class registry this read should resolve reflected
    /// fields against.
    #[must_use]
    pub const fn with_types(mut self, types: TypeRegistry<'a>) -> Self {
        self.types = Some(types);
        self
    }

    /// The bound class registry, or `None` when this buffer decodes only
    /// concrete types.
    #[must_use]
    pub const fn types(&self) -> Option<TypeRegistry<'a>> {
        self.types
    }

    /// The bound class registry, or the wiring error naming the omission.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::UnboundTypes`] when the buffer was built
    /// without [`Self::with_types`], so a reflected field has no registry to
    /// resolve its `type_index` against.
    pub fn require_types(&self) -> Result<TypeRegistry<'a>, MarshalerError> {
        self.types.ok_or(MarshalerError::UnboundTypes)
    }

    /// Attach a fresh [`DebugRecorder`] for the duration of an unmarshal.
    /// Used by capture/UI introspection paths that need byte-accurate spans.
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn with_recorder(endian: Endian, data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            endian,
            types: None,
            recorder: Some(DebugRecorder::new()),
        }
    }

    /// Detach and return the recorder (if any). Call after `unmarshal`
    /// completes to harvest the field list.
    #[cfg(debug_assertions)]
    pub const fn take_recorder(&mut self) -> Option<DebugRecorder> {
        self.recorder.take()
    }

    /// Run `f` and, if a recorder is attached, record the resulting byte
    /// range and `Debug`-formatted value as a [`DebugField`] named `name`.
    /// Nested calls are recorded with a full path.
    ///
    /// **Performance with no recorder.** The
    /// `is_none` early-return skips the `into()` call entirely.
    ///
    /// **Performance with recorder.** The `Cow` is built
    /// (zero alloc for `&'static str`, one move for `String`). Nested names
    /// allocate one path string per recorded field, which is debug-tooling
    /// only.
    ///
    /// **Ergonomics.** Both static and dynamic names work without ceremony:
    ///
    /// ```ignore
    /// rb.field("header", |rb| ActorRequestId::unmarshal(rb))?;
    /// rb.field(format!("fragment[{i}]"), |rb| Fragment::unmarshal(rb))?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns whatever error `f` returns; this wrapper adds no failure of its
    /// own, and a failed `f` is recorded as no field rather than a partial one.
    #[cfg(debug_assertions)]
    pub fn field<T, F, N>(&mut self, name: N, f: F) -> Result<T, MarshalerError>
    where
        F: FnOnce(&mut Self) -> Result<T, MarshalerError>,
        T: core::fmt::Debug,
        N: Into<std::borrow::Cow<'static, str>>,
    {
        if self.recorder.is_none() {
            return f(self);
        }
        let name = name.into();
        let start = self.position;
        if let Some(rec) = self.recorder.as_mut() {
            rec.push_path(name);
        }
        let result = f(self);
        let end = self.position;
        if let Some(rec) = self.recorder.as_mut() {
            if let Ok(value) = &result {
                let name = rec.path_name();
                rec.fields.push(DebugField {
                    name,
                    value: format!("{value:?}"),
                    start,
                    end,
                });
            }
            rec.pop_path();
        }
        result
    }

    /// Record a named byte span without requiring the parsed value to
    /// implement `Debug`. Used by generic containers to expose element ranges
    /// while the element's own `Marshaler` records field values below it.
    ///
    /// # Errors
    ///
    /// Returns whatever error `f` returns; this wrapper adds no failure of its
    /// own, and a failed `f` records no span.
    #[cfg(debug_assertions)]
    pub fn span<T, F, N>(&mut self, name: N, f: F) -> Result<T, MarshalerError>
    where
        F: FnOnce(&mut Self) -> Result<T, MarshalerError>,
        N: Into<std::borrow::Cow<'static, str>>,
    {
        if self.recorder.is_none() {
            return f(self);
        }
        let name = name.into();
        let start = self.position;
        if let Some(rec) = self.recorder.as_mut() {
            rec.push_path(name);
        }
        let result = f(self);
        let end = self.position;
        if let Some(rec) = self.recorder.as_mut() {
            if result.is_ok() {
                rec.fields.push(DebugField {
                    name: rec.path_name(),
                    value: String::new(),
                    start,
                    end,
                });
            }
            rec.pop_path();
        }
        result
    }

    /// Debug-only indexed element span. Release builds compile this down to
    /// the underlying read without formatting an index label.
    ///
    /// # Errors
    ///
    /// Returns whatever error `f` returns; see [`Self::span`].
    #[cfg(debug_assertions)]
    pub fn indexed_span<T, F>(&mut self, index: usize, f: F) -> Result<T, MarshalerError>
    where
        F: FnOnce(&mut Self) -> Result<T, MarshalerError>,
    {
        self.span(format!("[{index}]"), f)
    }

    /// Release-build field wrapper. The name and `Debug` formatting path are
    /// compiled out; this is just the production unmarshal call.
    ///
    /// # Errors
    ///
    /// Returns whatever error `f` returns.
    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn field<T, F, N>(&mut self, _name: N, f: F) -> Result<T, MarshalerError>
    where
        F: FnOnce(&mut ReadBuffer<'a>) -> Result<T, MarshalerError>,
    {
        f(self)
    }

    /// Release-build span wrapper; the name is compiled out.
    ///
    /// # Errors
    ///
    /// Returns whatever error `f` returns.
    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn span<T, F, N>(&mut self, _name: N, f: F) -> Result<T, MarshalerError>
    where
        F: FnOnce(&mut ReadBuffer<'a>) -> Result<T, MarshalerError>,
    {
        f(self)
    }

    /// Release-build indexed-span wrapper; the index label is compiled out.
    ///
    /// # Errors
    ///
    /// Returns whatever error `f` returns.
    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn indexed_span<T, F>(&mut self, _index: usize, f: F) -> Result<T, MarshalerError>
    where
        F: FnOnce(&mut ReadBuffer<'a>) -> Result<T, MarshalerError>,
    {
        f(self)
    }

    /// Read one byte and advance the cursor.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if the cursor is already at
    /// the end of the body.
    pub fn read_u8(&mut self) -> Result<u8, MarshalerError> {
        if self.position < self.data.len() {
            let value = self.data[self.position];
            self.position += 1;
            Ok(value)
        } else {
            Err(MarshalerError::buffer_underrun(self.left(), 1))
        }
    }

    /// Engine-compatible `ReadRawBit`.
    ///
    /// Current protocol helpers consume strict one-byte bools at the call
    /// sites this crate models. Keep the cursor byte-addressed while preserving
    /// the engine method name at the buffer layer.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if no byte remains, or
    /// [`MarshalerError::InvalidDiscriminant`] if the byte is neither `0`
    /// nor `1`.
    pub fn read_raw_bit(&mut self) -> Result<bool, MarshalerError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(MarshalerError::InvalidDiscriminant { value }),
        }
    }

    /// Read two bytes in the buffer's endianness and advance the cursor.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if fewer than 2 bytes remain.
    pub fn read_u16(&mut self) -> Result<u16, MarshalerError> {
        if self.position + 2 <= self.data.len() {
            let bytes = [self.data[self.position], self.data[self.position + 1]];
            self.position += 2;
            Ok(match self.endian {
                Endian::BigEndian => u16::from_be_bytes(bytes),
                Endian::LittleEndian => u16::from_le_bytes(bytes),
            })
        } else {
            Err(MarshalerError::buffer_underrun(self.left(), 2))
        }
    }

    /// Read four bytes in the buffer's endianness and advance the cursor.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if fewer than 4 bytes remain.
    pub fn read_u32(&mut self) -> Result<u32, MarshalerError> {
        if self.position + 4 <= self.data.len() {
            let bytes = [
                self.data[self.position],
                self.data[self.position + 1],
                self.data[self.position + 2],
                self.data[self.position + 3],
            ];
            self.position += 4;
            Ok(match self.endian {
                Endian::BigEndian => u32::from_be_bytes(bytes),
                Endian::LittleEndian => u32::from_le_bytes(bytes),
            })
        } else {
            Err(MarshalerError::buffer_underrun(self.left(), 4))
        }
    }

    /// Borrow the next `len` bytes and advance the cursor past them.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if fewer than `len` bytes
    /// remain — the cursor is left untouched, so the caller may report on the
    /// same position.
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], MarshalerError> {
        if self.position + len <= self.data.len() {
            let slice = &self.data[self.position..self.position + len];
            self.position += len;
            Ok(slice)
        } else {
            Err(MarshalerError::buffer_underrun(self.left(), len))
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.position >= self.data.len()
    }

    #[must_use]
    pub const fn left(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }

    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    pub const fn endian(&self) -> Endian {
        self.endian
    }

    /// Bytes that have not yet been consumed.
    #[must_use]
    pub fn remaining(&self) -> &'a [u8] {
        &self.data[self.position..]
    }

    /// Bytes in an already-observed absolute range.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] if the range is inverted
    /// (`start > end`) or its end lies past the body.
    pub fn range(&self, range: std::ops::Range<usize>) -> Result<&'a [u8], MarshalerError> {
        if range.start <= range.end && range.end <= self.data.len() {
            Ok(&self.data[range])
        } else {
            Err(MarshalerError::buffer_underrun(
                self.left(),
                range.end.saturating_sub(range.start),
            ))
        }
    }

    pub fn skip(&mut self, bytes: usize) {
        self.position = self.position.saturating_add(bytes).min(self.data.len());
    }
}

#[cfg(all(test, debug_assertions))]
mod recorder_tests {
    use super::*;
    use crate::serialize::Marshaler;

    /// Confirms that a `ReadBuffer::with_recorder` captures one
    /// `DebugField` per `rb.field(...)` call inside an unmarshal, with byte
    /// ranges that match the consumed positions.
    #[test]
    fn recorder_captures_top_level_fields_with_byte_ranges() {
        // Define a small test type by hand using the public `rb.field` API.
        // We don't need the derive here — the test is about the recorder.
        struct Outer {
            a: u32,
            b: u16,
        }
        impl Marshaler for Outer {
            fn marshal(&self, _wb: &mut WriteBuffer) {}
            fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
                let a = rb.field("a", |rb| u32::unmarshal(rb))?;
                let b = rb.field("b", |rb| u16::unmarshal(rb))?;
                Ok(Self { a, b })
            }
        }

        let bytes = [0x00, 0x00, 0x00, 0x2a, 0x01, 0x02];
        let mut rb = ReadBuffer::with_recorder(Endian::BigEndian, &bytes);
        let v = Outer::unmarshal(&mut rb).unwrap();
        assert_eq!(v.a, 42);
        assert_eq!(v.b, 0x0102);

        let fields = rb.take_recorder().unwrap().into_fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "a");
        assert_eq!(fields[0].start, 0);
        assert_eq!(fields[0].end, 4);
        assert_eq!(fields[1].name, "b");
        assert_eq!(fields[1].start, 4);
        assert_eq!(fields[1].end, 6);
    }

    /// Confirms that dynamic span names (e.g. `format!("fragment[{i}]")`)
    /// flow through the same API. This is the key affordance that lets the
    /// `StateBundle` walker — which produces dynamically-named fragments — use
    /// the universal recorder without a special-case path.
    ///
    /// Static literals stay borrowed (zero-alloc `Cow::Borrowed`); dynamic
    /// names are passed as owned `String` and become `Cow::Owned`.
    #[test]
    fn recorder_accepts_static_and_dynamic_names() {
        struct Walker;
        impl Marshaler for Walker {
            fn marshal(&self, _wb: &mut WriteBuffer) {}
            fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
                rb.field("static_name", |rb| u8::unmarshal(rb))?;
                for i in 0..2u32 {
                    rb.field(format!("fragment[{i}]"), |rb| u8::unmarshal(rb))?;
                }
                Ok(Self)
            }
        }

        let bytes = [0xa, 0xb, 0xc];
        let mut rb = ReadBuffer::with_recorder(Endian::BigEndian, &bytes);
        let _ = Walker::unmarshal(&mut rb).unwrap();
        let fields = rb.take_recorder().unwrap().into_fields();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "static_name");
        assert_eq!(fields[1].name, "fragment[0]");
        assert_eq!(fields[2].name, "fragment[1]");
        // The static name is borrowed — no allocation cost when recording.
        assert!(matches!(&fields[0].name, std::borrow::Cow::Borrowed(_)));
        // The dynamic names are owned.
        assert!(matches!(&fields[1].name, std::borrow::Cow::Owned(_)));
    }

    #[test]
    fn recorder_captures_nested_field_paths() {
        #[derive(Debug)]
        struct Inner;
        impl Marshaler for Inner {
            fn marshal(&self, _wb: &mut WriteBuffer) {}
            fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
                let _ = rb.field("inner_field", |rb| u8::unmarshal(rb))?;
                Ok(Self)
            }
        }
        #[derive(Debug)]
        struct Outer;
        impl Marshaler for Outer {
            fn marshal(&self, _wb: &mut WriteBuffer) {}
            fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
                let _ = rb.field("outer_field", |rb| Inner::unmarshal(rb))?;
                Ok(Self)
            }
        }

        let bytes = [0x42];
        let mut rb = ReadBuffer::with_recorder(Endian::BigEndian, &bytes);
        let _ = Outer::unmarshal(&mut rb).unwrap();
        let fields = rb.take_recorder().unwrap().into_fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "outer_field.inner_field");
        assert_eq!(fields[0].start, 0);
        assert_eq!(fields[0].end, 1);
        assert_eq!(fields[1].name, "outer_field");
        assert_eq!(fields[1].start, 0);
        assert_eq!(fields[1].end, 1);
    }

    #[test]
    fn recorder_captures_indexed_spans_without_debug_values() {
        let bytes = [0x11, 0x22];
        let mut rb = ReadBuffer::with_recorder(Endian::BigEndian, &bytes);
        let first = rb.indexed_span(0, |rb| u8::unmarshal(rb)).unwrap();
        let second = rb.indexed_span(1, |rb| u8::unmarshal(rb)).unwrap();

        assert_eq!((first, second), (0x11, 0x22));
        let fields = rb.take_recorder().unwrap().into_fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "[0]");
        assert_eq!(fields[0].value, "");
        assert_eq!(fields[0].start, 0);
        assert_eq!(fields[0].end, 1);
        assert_eq!(fields[1].name, "[1]");
        assert_eq!(fields[1].start, 1);
        assert_eq!(fields[1].end, 2);
    }

    #[test]
    fn no_recorder_means_zero_overhead_passthrough() {
        struct Tracker(u32);
        impl Marshaler for Tracker {
            fn marshal(&self, _wb: &mut WriteBuffer) {}
            fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
                let v = rb.field("v", |rb| u32::unmarshal(rb))?;
                Ok(Self(v))
            }
        }
        let bytes = [0x00, 0x00, 0x00, 0x07];
        let mut rb = ReadBuffer::new(Endian::BigEndian, &bytes);
        let t = Tracker::unmarshal(&mut rb).unwrap();
        assert_eq!(t.0, 7);
        assert!(rb.take_recorder().is_none());
    }
}

impl Read for ReadBuffer<'_> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let available = self.left();
        if available == 0 || buf.is_empty() {
            return Ok(0);
        }
        let to_copy = available.min(buf.len());
        buf[..to_copy].copy_from_slice(&self.data[self.position..self.position + to_copy]);
        self.position += to_copy;
        Ok(to_copy)
    }
}

/// Write buffer for serializing data (`GridMate`: `WriteBuffer`)
pub struct WriteBuffer {
    data: Vec<u8>,
    endian: Endian,
}

impl WriteBuffer {
    /// Create new `WriteBuffer` with specified endianness (`GridMate`: WriteBuffer(endian))
    #[must_use]
    pub const fn new(endian: Endian) -> Self {
        Self {
            data: Vec::new(),
            endian,
        }
    }

    #[must_use]
    pub fn with_capacity(endian: Endian, capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            endian,
        }
    }

    #[must_use]
    pub const fn from_vec(endian: Endian, data: Vec<u8>) -> Self {
        Self { data, endian }
    }

    /// Endianness this buffer encodes integers with. Useful for
    /// downstream helpers that need to build a temporary
    /// [`WriteBuffer`] in the same byte order (e.g.
    /// `replica::cmd_marshal`'s `PackedSize` / VLQ side buffers).
    #[must_use]
    pub const fn endian(&self) -> Endian {
        self.endian
    }

    pub fn write_u8(&mut self, value: u8) {
        self.data.push(value);
    }

    /// Engine-compatible `WriteRawBit`.
    ///
    /// The active protocol paths are byte-addressed; this writes a strict
    /// `0`/`1` byte while keeping bool marshaling source-followable.
    pub fn write_raw_bit(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    pub fn write_u16(&mut self, value: u16) {
        let bytes = match self.endian {
            Endian::BigEndian => value.to_be_bytes(),
            Endian::LittleEndian => value.to_le_bytes(),
        };
        self.data.extend_from_slice(&bytes);
    }

    pub fn write_u32(&mut self, value: u32) {
        let bytes = match self.endian {
            Endian::BigEndian => value.to_be_bytes(),
            Endian::LittleEndian => value.to_le_bytes(),
        };
        self.data.extend_from_slice(&bytes);
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Reset length to zero, keeping the allocated capacity. Use to
    /// reuse the same buffer across many encodes (per-system `Local<>`
    /// in Bevy, thread-local outside Bevy).
    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn truncate(&mut self, len: usize) {
        self.data.truncate(len);
    }

    /// Reserve a fixed-length prefix region, write the body, then patch
    /// the prefix in place once the body is known.
    ///
    /// This is the back-patch dance every length-prefixed framing needs:
    /// 1. Reserve `prefix_len` zero bytes upfront so the body lands at a
    ///    known offset.
    /// 2. Run `write_body` to append the body.
    /// 3. Invoke `patch_prefix(prefix_slice, body_slice)` so the caller
    ///    can compute checksums / sizes / headers over the body and write
    ///    them into the reserved region.
    ///
    /// Owning the dance here keeps the magic-offset arithmetic (and the
    /// need for `pub` mutable buffer access) inside `WriteBuffer`.
    pub fn with_fixed_prefix<R>(
        &mut self,
        prefix_len: usize,
        write_body: impl FnOnce(&mut Self) -> R,
        patch_prefix: impl FnOnce(&mut [u8], &[u8]),
    ) -> R {
        let prefix_pos = self.data.len();
        self.data.resize(prefix_pos + prefix_len, 0);
        let body_pos = self.data.len();
        let result = write_body(self);
        let body_end = self.data.len();
        let body_len = body_end - body_pos;
        let region = &mut self.data[prefix_pos..body_end];
        let (prefix_slice, body_tail) = region.split_at_mut(prefix_len);
        let body_slice: &[u8] = &body_tail[..body_len];
        patch_prefix(prefix_slice, body_slice);
        result
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.data.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Default for WriteBuffer {
    fn default() -> Self {
        Self::new(CARRIER_ENDIAN)
    }
}
