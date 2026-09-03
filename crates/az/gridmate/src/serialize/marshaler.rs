// Common marshaler trait (correct spelling: Marshaler)

use super::{
    buffer::{ReadBuffer, WriteBuffer},
    error::MarshalerError,
};

/// One named span captured during an instrumented unmarshal.
///
/// `start..end` is the byte range inside the body that the span consumed,
/// suitable for driving a hex-view highlight. `value` is a `Debug`-formatted
/// rendering of the parsed value.
///
/// `name` is `Cow<'static, str>` so static literals (`"header"` from
/// `#[derive(Marshaler)]`) stay borrowed with zero allocation even when
/// recording, while dynamic names from container walkers flow through the
/// same API as owned strings. Both
/// shapes coexist behind one signature.
///
/// Only present under `cfg(debug_assertions)`.
#[cfg(debug_assertions)]
#[derive(Debug, Clone)]
pub struct DebugField {
    pub name: std::borrow::Cow<'static, str>,
    pub value: String,
    pub start: usize,
    pub end: usize,
}

/// "This value type can encode itself" — the common case.
///
/// Used by concrete `impl Marshaler for T` blocks and emitted by
/// `#[derive(Marshaler)]`. For external policies that encode another type with
/// a non-default wire shape, such as [`VlqU32Marshaler`](super::vlq::VlqU32Marshaler)
/// for `u32`, see [`Codec`].
///
/// Field-level introspection is **not** a separate method. Implementations
/// call [`ReadBuffer::field`] to delimit each named field. In production
/// builds those calls are zero-cost passthroughs; when a [`DebugRecorder`]
/// is attached to the buffer by debug capture tooling, the recorder captures
/// the byte range and `Debug`-formatted value of each top-level field. There's
/// no two-trait split, no duplicate codegen, and no risk of a type being
/// introspectable through one path but not the other.
///
/// [`DebugRecorder`]: super::buffer::DebugRecorder
pub trait Marshaler: Sized {
    /// Source equivalent of `GridMate::Marshaler<T>::MarshalSize`.
    ///
    /// `0` means dynamic or unspecified. That mirrors the C++ default and is
    /// what `IsFixedMarshaler` checks before treating a wire shape as fixed.
    const MARSHAL_SIZE: usize = 0;

    /// Serialize into the provided write buffer. `WriteBuffer` is
    /// infallible, so this stays `()`.
    fn marshal(&self, wb: &mut WriteBuffer);

    /// Deserialize from the provided read buffer.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::BufferUnderrun`] when the body ends before
    /// the type's fields are complete, and whichever decode error the
    /// implementation raises for a byte pattern it rejects —
    /// [`MarshalerError::InvalidDiscriminant`] for an out-of-set enum tag,
    /// [`MarshalerError::ContainerOverflow`] for a length prefix above the
    /// container's cap, [`MarshalerError::Utf8`] for a malformed string.
    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError>;
}

/// "This zero-sized type is a wire codec for some `T`".
///
/// `Codec<T>` separates "value type that encodes itself" (which stays on
/// [`Marshaler`]) from "external policy that overrides the wire shape for
/// some `T`." This is the split Lumberyard's C++ original kept ([`Marshaler<T>`]
/// for self codecs vs. `ContainerMarshaler<C, DataMarshaler>` for explicit
/// per-element policies); the prior Rust shape `Marshaler<T = Self>`
/// collapsed both into one trait with mutually-recursive defaults and
/// `where T: Marshaler<T>` bounds, which forced (a) a dead 3-tuple
/// `Marshaler` impl just to satisfy the bound, (b) method-resolution
/// ambiguity at any type that wanted to be both, (c) fully-qualified path
/// workarounds at call sites. Splitting them removes all three.
///
pub trait Codec<T> {
    /// Fixed wire size for this policy, or `0` for dynamic/unspecified.
    ///
    /// This is the policy-slot equivalent of `Marshaler<T>::MarshalSize`;
    /// field-local codecs such as `Vec3CompMarshaler` can expose the same
    /// metadata even though they are not value-level `Marshaler` impls.
    const MARSHAL_SIZE: usize = 0;

    /// Serialize `value` using this policy's wire shape.
    fn marshal(value: &T, wb: &mut WriteBuffer);

    /// Deserialize a `T` using this policy's wire shape.
    ///
    /// # Errors
    ///
    /// Returns whatever this policy's decode raises — at minimum
    /// [`MarshalerError::BufferUnderrun`] when the body is shorter than the
    /// shape requires. Policies that narrow the accepted values (compact enum
    /// codecs, bounded containers) also return
    /// [`MarshalerError::InvalidDiscriminant`] or
    /// [`MarshalerError::ContainerOverflow`].
    fn unmarshal(rb: &mut ReadBuffer) -> Result<T, MarshalerError>;
}

/// Default policy for a value that marshals itself.
///
/// This represents the public `GridMate` default policy
/// `GridMate::Marshaler<T>` used by buffers and container marshalers. Keeping
/// it as a named policy avoids making every `T: Marshaler` also expose a
/// second `unmarshal` associated function through [`Codec`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultMarshaler<T>(std::marker::PhantomData<fn() -> T>);

impl<T: Marshaler> Codec<T> for DefaultMarshaler<T> {
    const MARSHAL_SIZE: usize = T::MARSHAL_SIZE;

    #[inline]
    fn marshal(value: &T, wb: &mut WriteBuffer) {
        value.marshal(wb);
    }

    #[inline]
    fn unmarshal(rb: &mut ReadBuffer) -> Result<T, MarshalerError> {
        <T as Marshaler>::unmarshal(rb)
    }
}

/// Source-named fixed-size query for value marshalers.
///
/// Rust cannot reproduce the C++ SFINAE "false for any type" probe without
/// specialization, so this helper is intentionally a bound: it is available
/// when `M: Marshaler`, and `VALUE` follows `M::MARSHAL_SIZE != 0`.
pub struct IsFixedMarshaler<M>(std::marker::PhantomData<fn() -> M>);

impl<M: Marshaler> IsFixedMarshaler<M> {
    pub const MARSHAL_SIZE: usize = M::MARSHAL_SIZE;
    pub const VALUE: bool = M::MARSHAL_SIZE != 0;
}

/// Fixed-size query for policy codecs.
pub struct IsFixedCodec<T, M>(std::marker::PhantomData<fn() -> (T, M)>);

impl<T, M: Codec<T>> IsFixedCodec<T, M> {
    pub const MARSHAL_SIZE: usize = M::MARSHAL_SIZE;
    pub const VALUE: bool = M::MARSHAL_SIZE != 0;
}

/// Source-named marshaler/type compatibility helper.
///
/// The C++ helper returns a bool after probing method signatures. Rust encodes
/// the same fact as a trait bound: this associated `VALUE` exists exactly when
/// `M` can encode `T` through [`Codec<T>`].
pub struct IsMarshalerForType<T, M>(std::marker::PhantomData<fn() -> (T, M)>);

impl<T, M: Codec<T>> IsMarshalerForType<T, M> {
    pub const VALUE: bool = true;
}

#[inline]
#[must_use]
pub const fn is_fixed_marshaler<M: Marshaler>() -> bool {
    M::MARSHAL_SIZE != 0
}

#[inline]
#[must_use]
pub const fn fixed_marshal_size<M: Marshaler>() -> usize {
    M::MARSHAL_SIZE
}

#[inline]
#[must_use]
pub const fn is_fixed_codec<T, M: Codec<T>>() -> bool {
    M::MARSHAL_SIZE != 0
}
