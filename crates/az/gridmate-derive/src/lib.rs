//! Derive macros for the `GridMate` protocol layer.
//!
//! Every derive in this crate maps **one derive name → one trait impl**,
//! mirroring serde's `Serialize` / `Deserialize` split. This keeps each
//! grant visible at the call site:
//!
//! ```ignore
//! #[derive(AzTypeInfo, Marshaler, ClassDesc, Message)]
//! #[az_type_info("6A379FB8-0BDD-43A1-AB3E-9843D7BE8CD3")]
//! #[class_desc(type_index = 349)]
//! pub struct PingMsg { pub timestamp: u64 }
//! ```
//!
//! …says "this type can be wire-encoded, is in the network registry, and
//! has an envelope index" — three statements visible in three derive
//! names. Compare to a kit derive that hides those grants behind a single
//! word.
//!
//! # Error handling
//!
//! Each derive's helper code returns [`syn::Result<TokenStream>`]. The
//! proc-macro entry points below convert errors at the boundary via
//! `.unwrap_or_else(syn::Error::into_compile_error)` so user-facing
//! diagnostics are always `compile_error!`-shaped with proper spans.
//! Helpers never need to construct `compile_error!` by hand.
//!
//! Darling's own attribute-parsing errors are bridged into [`syn::Error`]
//! by [`error::darling_to_syn`] for uniformity.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod attrs;
mod chunk_marshaler;
mod class_desc;
mod error;
mod fixed_replicated_state_fields;
mod generics;
mod marshal;
mod marshaler;
mod message;
mod network_base;
mod replicated_state;
mod unmarshal;

/// Run a derive's `derive(input) -> syn::Result<TokenStream>` helper and
/// convert any error into a `compile_error!`-shaped `TokenStream` at the
/// boundary. This is the single integration point between the proc-macro
/// entry contract and the rest of the crate's `syn::Result`-based code.
fn run<F>(input: TokenStream, f: F) -> TokenStream
where
    F: FnOnce(&DeriveInput) -> syn::Result<proc_macro2::TokenStream>,
{
    let input = parse_macro_input!(input as DeriveInput);
    f(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the [`Marshaler`](::gridmate::serialize::Marshaler) wire codec
/// for a struct or enum.
///
/// # Field attributes (`#[marshal(...)]`)
///
/// - `#[marshal(as = "Type")]` — marshal the field as a different type;
///   requires `From<T>` + `Into<T>`.
/// - `#[marshal(with = "path::to::write_fn")]` — custom marshal function.
/// - `#[marshal(unmarshal_with = "path::to::read_fn")]` — custom unmarshal.
/// - `#[marshal(skip)]` — skip the field; must be `Default` for
///   unmarshaling.
///
/// # Variant attributes (enums)
///
/// - Native Rust discriminants, such as `First = 1`, are honored and later
///   variants follow Rust's normal auto-increment rule.
/// - `#[marshal(discriminant = N)]` — legacy explicit discriminant override.
///   Prefer native Rust discriminants for new code.
/// - `#[marshal(unknown)]` — for primitive `#[repr(...)]` enums, preserve
///   unrecognized values in a single-field catch-all variant. A variant named
///   `Unknown` is treated this way automatically.
///
/// # Generated code
///
/// One `impl Marshaler` block. Tagged union enums write a `u8` variant tag
/// before the variant body. Primitive `#[repr(u32)]`-style enums marshal as
/// that primitive value directly.
#[proc_macro_derive(Marshaler, attributes(marshal))]
pub fn derive_marshaler(input: TokenStream) -> TokenStream {
    run(input, marshaler::derive)
}

/// Derive [`Marshaler`](::gridmate::serialize::Marshaler) as the standard
/// [`IFragment`](::gridmate::hub::Fragment) contents delegate.
///
/// Pair with a hand-written [`Fragment`](::gridmate::hub::Fragment) impl
/// whenever the bitmask layout doesn't match what `#[derive(ReplicatedState)]` emits. The
/// derive turns the previously hand-copied ten-line `impl Marshaler` stub
/// into a single word at the type's derive site:
///
/// ```ignore
/// #[derive(Default, ChunkMarshaler, ClassDesc, Message)]
/// #[class_desc(uuid = "…")]
/// pub struct FooReplicatedState { /* … */ }
///
/// impl gridmate::hub::Fragment for FooReplicatedState { /* hand-rolled bitmask */ }
/// ```
///
/// A blanket `impl<T: Fragment> Marshaler for T` is rejected by Rust
/// coherence (the existing `impl<T: Marshaler> Marshaler for Box<T>` in
/// gridmate would overlap if a downstream crate ever implemented
/// `Fragment for Box<X>`); one concrete impl per type is fine, so the
/// derive emits exactly that.
#[proc_macro_derive(ChunkMarshaler)]
pub fn derive_chunk_marshaler(input: TokenStream) -> TokenStream {
    run(input, chunk_marshaler::derive)
}

/// Derive a native-style [`ClassDesc`](::gridmate::az::ClassDesc)
/// registration.
///
/// Use on every reflected wire type. Pair with [`derive_message`] when
/// the type is also a top-level wire envelope.
///
/// # Attribute (`#[class_desc(...)]`)
///
/// - `type_index = N` — native compact type index. Required for
///   top-level messages and fragments that use compact wire identity.
///
/// # Example
///
/// ```ignore
/// use az_derive::AzTypeInfo;
/// use gridmate::{ClassDesc, Marshaler};
///
/// #[derive(AzTypeInfo, Debug, Clone, Marshaler, ClassDesc)]
/// #[az_type_info("A1B2C3D4-E5F6-7890-ABCD-EF1234567890")]
/// #[class_desc(type_index = 1234)]
/// pub struct StorageItemSimple { /* … */ }
/// ```
#[proc_macro_derive(ClassDesc, attributes(class_desc))]
pub fn derive_class_desc(input: TokenStream) -> TokenStream {
    run(input, class_desc::derive)
}

/// Derive [`Message`](::gridmate::message::Message) — the top-level
/// wire-envelope marker.
///
/// Requires the type to also be `Class` (the `Message` trait has
/// `Class` as a supertrait). Compose with `#[derive(ClassDesc)]`
/// at the call site:
///
/// ```ignore
/// #[derive(AzTypeInfo, Marshaler, ClassDesc, Message)]
/// #[az_type_info("6A379FB8-0BDD-43A1-AB3E-9843D7BE8CD3")]
/// #[class_desc(type_index = 349)]
/// pub struct PingMsg { pub timestamp: u64 }
/// ```
///
/// # Attribute (`#[message(...)]`)
///
/// - `type_index = N` — optional override for the envelope `TYPE_INDEX`.
///   Prefer putting the native `typeIndex` on `#[class_desc(...)]`, keeping
///   the class descriptor self-contained in Rust.
/// - `client_to_server` — source `MF_TraitClientMsg`; emits
///   `Sendable<ClientToServer>` and `Receivable<ClientToServer>`.
/// - `server_to_client` — source `MF_TraitMsg`; emits
///   `Sendable<ServerToClient>` and `Receivable<ServerToClient>`.
#[proc_macro_derive(Message, attributes(message))]
pub fn derive_message(input: TokenStream) -> TokenStream {
    run(input, message::derive)
}

/// Derive [`NetworkBase`](::gridmate::az::NetworkBase) for a closed
/// polymorphic-value family enum.
///
/// # Variant shapes
///
/// - **Known variant**: `Variant(T)` where `T: Class`. The
///   variant's UUID comes from `AzTypeInfo::TYPE_ID`; its body codec from
///   `T`'s `Marshaler` impl.
/// - **Unknown variant**: `#[network_base(unknown)] Variant { uuid: Uuid, body: Vec<u8> }`.
///   Captures wire UUIDs not in the variant table — useful while the
///   codebase is still being mapped to Rust types. At most one allowed.
///
/// # Example
///
/// ```ignore
/// use gridmate::{ClassDesc, NetworkBase};
///
/// #[derive(Debug, Clone, PartialEq, Eq, NetworkBase)]
/// pub enum StorageItem {
///     Simple(StorageItemSimple),
///     Detailed(StorageItemDetailed),
///     #[network_base(unknown)]
///     Unknown { uuid: ::uuid::Uuid, body: Vec<u8> },
/// }
/// ```
#[proc_macro_derive(NetworkBase, attributes(network_base))]
pub fn derive_network_base(input: TokenStream) -> TokenStream {
    run(input, network_base::derive)
}

/// Derive source `MB::ReplicatedState` `IFragment` and `Marshaler` behavior.
///
/// Struct attributes:
/// - `#[replicated_state(category = "player_character")]` sets a fixed
///   fragment category.
/// - `#[replicated_state(metadata, category_field = "field")]` marks a
///   metadata fragment and reads its category from a registered attribute.
/// - `#[replicated_state(world_position = "method()")]` forwards the source
///   world-position capability.
/// - `#[replicated_state(skip_group = N)]` consumes an unmodeled native
///   descriptor group.
///
/// Field attributes:
/// - `#[replicated_state(group = N)]` selects the content descriptor group.
/// - `#[replicated_state(name = "m_sourceName")]` records the native name.
/// - `#[replicated_state(attribute)]` registers a hub-only attribute.
/// - `#[replicated_state(skip)]` excludes a non-wire field.
///
/// The embedded base is inferred from a field typed as
/// `gridmate::hub::ReplicatedState`.
#[proc_macro_derive(ReplicatedState, attributes(replicated_state))]
pub fn derive_replicated_state(input: TokenStream) -> TokenStream {
    run(input, replicated_state::derive)
}

/// Derive the Rust adapter for a source-shaped
/// `gridmate::hub::FixedReplicatedState<...>` fragment.
///
/// `#[fixed_state(group = N)]` assigns a typed field to a native group and
/// `#[fixed_state(skip)]` excludes non-wire state. The embedded base is
/// inferred from the `gridmate::hub::FixedReplicatedState<...>` field.
#[proc_macro_derive(FixedReplicatedStateFields, attributes(fixed_state))]
pub fn derive_fixed_replicated_state_fields(input: TokenStream) -> TokenStream {
    run(input, fixed_replicated_state_fields::derive)
}
