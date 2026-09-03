//! Generic AZ/Lumberyard derive macros.
//!
//! Macro layers mirror native `AzCore`:
//! - [`AzTypeInfo`] ↔ `AZ_TYPE_INFO` (static UUID + name)
//! - [`AzRtti`] ↔ `AZ_RTTI` (shared type info + base `TypeIds`)
//! - [`AzComponent`] ↔ `AZ_COMPONENT` (`AZ_RTTI` + descriptor registration)
//!
//! Attribute syntax follows the native macro shape:
//!
//! ```ignore
//! #[derive(AzTypeInfo)]
//! #[az_type_info("6383F1D3-BB27-4E6B-A49A-6409B2059EAA")]
//! struct EntityId;
//!
//! #[derive(AzRtti)]
//! #[az_rtti("{27F37921-4B40-4BE6-B47B-7D3AB8682D58}", EntityId)]
//! struct NamedEntityId;
//! ```
//!
//! A UUID may be a string literal, a braced source-copied string literal, or a
//! const expression returning [`uuid::Uuid`]. When a native display-name
//! override is needed, write it before the positional UUID:
//! `#[az_rtti(name = "Native::Type", "uuid", Base)]`.
//!
//! `AzRtti` only implements native identity by default. Add `register` when a
//! concrete type must reach a host's composed `AzTypeRegistration` registry:
//! `#[az_rtti("uuid", Base, register)]`. That emits a crate-private
//! `Self::REGISTRATION` const holding the entry, which the owning crate lists
//! in its `types()` enumeration and hands to a host through `register(ctx)`.
//! Leaving a registered type out of that list does not compile — the const is
//! `deny(dead_code)`, so the miss is named at the type instead of becoming an
//! entry no host ever receives.

mod attrs;
mod expand;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

use crate::expand::{expand_az_component, expand_az_rtti, expand_az_type_info};

/// Implements [`az_core::type_info::AzTypeInfo`].
#[proc_macro_derive(AzTypeInfo, attributes(az_type_info, type_info))]
pub fn derive_az_type_info(input: TokenStream) -> TokenStream {
    derive(input, expand_az_type_info)
}

/// Implements [`az_core::type_info::AzTypeInfo`] and
/// [`az_core::rtti::AzRtti`].
#[proc_macro_derive(AzRtti, attributes(az_rtti, rtti))]
pub fn derive_az_rtti(input: TokenStream) -> TokenStream {
    derive(input, expand_az_rtti)
}

/// Implements [`az_core::component::AzComponent`] and registers the native
/// component descriptor selected by `az-core`'s feature set.
#[proc_macro_derive(AzComponent, attributes(az_component, component))]
pub fn derive_az_component(input: TokenStream) -> TokenStream {
    derive(input, expand_az_component)
}

fn derive(
    input: TokenStream,
    expand: fn(&DeriveInput) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
