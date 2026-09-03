//! `#[derive(ClassDesc)]` — emits descriptor registration.
//!
//! Companion to `#[derive(Message)]`. Use it on every type reflected into
//! the native `TypeRegistry`. Pair it with `#[derive(AzTypeInfo)]`;
//! AZ RTTI owns UUID/name, while `ClassDesc` owns only `GridMate`'s compact
//! `typeIndex` registration data.
//!
//! Emits `impl Class` — the native `TYPE_INDEX` constant — and nothing else.
//!
//! It does **not** register the type. Deriving a trait says what a type *is*;
//! it cannot say which host wants it, and a link-time submission answered that
//! question by accident — whichever crates the linker happened to keep. The
//! crate that declares the type publishes it instead, by handing
//! `ClassRegistration::of::<T>()` to a contribution's registrar. `ClassDesc` is
//! what makes that call type-check.
//!
//! **Not** emitted: `Marshaler` and `Message`. `Class` has `Marshaler` as a
//! supertrait but the impl is the user's responsibility
//! (`#[derive(Marshaler)]` for plain field-by-field, or hand-rolled when
//! the wire layout has internal discriminants etc.). For top-level wire
//! messages, also derive [`Message`](crate::derive_message).
//!
//! # Example
//!
//! ```ignore
//! use az_derive::AzTypeInfo;
//! use gridmate::{ClassDesc, Marshaler};
//!
//! #[derive(AzTypeInfo, Debug, Clone, Marshaler, ClassDesc)]
//! #[az_type_info("A1B2C3D4-E5F6-7890-ABCD-EF1234567890")]
//! #[class_desc(type_index = 1234)]
//! pub struct StorageItemSimple {
//!     pub field_08: u32,
//!     pub field_0c: u32,
//! }
//! ```

// darling's `FromDeriveInput` expansion lands here as a sibling impl carrying
// this file's spans, so its generated parse loop and `#[darling(default)]`
// fallbacks can only be exempted at module scope.
#![allow(clippy::needless_continue, clippy::option_if_let_else)]

use darling::FromDeriveInput;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Ident};

use crate::error::darling_to_syn;

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(class_desc), supports(struct_any, enum_any))]
struct ClassDescOpts {
    ident: Ident,
    generics: syn::Generics,

    /// Native `typeIndex` for this class. Message and fragment types should
    /// set this explicitly in Rust; `0`/omitted is only for helper types that
    /// never use compact wire identity.
    #[darling(default)]
    type_index: Option<u32>,
}

pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    // Reject unions explicitly — darling's `supports` already covers it,
    // but doing it here keeps the error message specific.
    if let Data::Union(u) = &input.data {
        return Err(syn::Error::new_spanned(
            u.union_token,
            "#[derive(ClassDesc)] does not support unions",
        ));
    }

    let opts = ClassDescOpts::from_derive_input(input).map_err(darling_to_syn)?;

    let ident = &opts.ident;
    let (impl_generics, ty_generics, where_clause) = opts.generics.split_for_impl();
    let type_index = opts.type_index.unwrap_or(0);

    Ok(quote! {
        impl #impl_generics ::gridmate::az::Class
        for #ident #ty_generics #where_clause
        {
            const TYPE_INDEX: u32 = #type_index;
        }
    })
}
