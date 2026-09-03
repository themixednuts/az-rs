//! Per-type AZ class identity used by the native `TypeRegistry`.
//!
//! In Lumberyard, every type that participates in network replication declares
//! itself to a network class registry. The builder records the type's UUID
//! (from `AZ_TYPE_INFO`) and a `ClassDesc` carrying factory functors and
//! marshal handlers.
//!
//! In Rust, AZ RTTI identity is supplied by [`AzTypeInfo`]. This trait adds
//! only the compact native `typeIndex` when that type has one.

use crate::serialize::Marshaler;
use az_core::type_info::AzTypeInfo;

/// A type that can produce a native-style [`ClassDesc`](super::ClassDesc).
///
/// In C++, `AZ_TYPE_INFO(T, "{...}")` provides the UUID and name; the network
/// reflection step ties them to a descriptor. In Rust, [`AzTypeInfo`] carries
/// that identity, the existing [`Marshaler`] impl carries the (un)marshal
/// handlers, and `ClassRegistration::of::<T>()` — handed to a contribution's
/// registrar by the crate that declares `T` — carries the `GridMate`
/// registration itself. A type that is also a
/// [`Message`](crate::message::Message) uses `of_message::<T>()` instead, and
/// registers once, not twice.
///
/// # Implementing
///
/// Provided by `#[derive(ClassDesc)]` (`gridmate-derive`). For top-level
/// wire messages, additionally derive [`Message`](crate::message::Message)
/// — the derives compose as
/// `#[derive(AzTypeInfo, Marshaler, ClassDesc, Message)]`.
pub trait Class: Marshaler + AzTypeInfo + 'static {
    /// Native compact `typeIndex` for this UUID, or `0` when this type is
    /// not present in the selected native registry.
    ///
    /// `FragmentKey` and `StateFragmentTypeId` values in state bundles are not
    /// this value. Native state-fragment type info uses the dense registry
    /// index, while `IMessage` envelopes use this sparse type index.
    const TYPE_INDEX: u32 = 0;
}
