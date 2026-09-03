//! AZ networking reflection layer.
//!
//! Native-compatible type registry for Hub protocol values.
//! Rust types register their own descriptors, and debug builds can validate
//! those registrations against the parsed native `resources/typeregistry.json`
//! table.
//!
//! - **Native registry index.** The runtime assigns every reflected class a dense
//!   `localIndex` for registry storage; it is not a protocol identity.
//! - **Compact type index.** The sparse `typeIndex` is the protocol-facing
//!   identity used by `IMessage` envelopes, state-fragment type info, and other
//!   explicitly type-indexed values.
//!
//! These indices are derived from the same AZ UUID space. Runtime lookup is
//! owned by [`TypeRegistry`]; the native JSON table is a debug validation
//! input, not a release dependency.
//!
//! # Layering
//!
//! - [`Class`] — per-type credentials (UUID + name + native `typeIndex` when
//!   the type has compact wire identity).
//! - [`TypeRegistry`] — the central registry. Holds Rust descriptor
//!   registrations keyed by UUID and native `typeIndex`.
//! - [`NetworkBase`] / [`NetworkValue`] — the polymorphic-value field codec.
//!   Independent of registration for known variants; consults
//!   the read buffer's bound [`TypeRegistry`] only to resolve an unknown compact id on the
//!   read path.

pub mod class;
pub mod network_value;
pub mod reflected_enum;
pub mod type_registry;

pub use class::Class;
pub use network_value::{ClassValue, NetworkBase, NetworkValue, VariantEntry};
pub use type_registry::{
    ClassDesc, ClassRegistration, CollectDiagnostics, MessageInfo, NameMismatch, TypeIndexClash,
    TypeRegistry,
};
#[cfg(debug_assertions)]
pub use type_registry::{DebugIntrospect, introspect};
